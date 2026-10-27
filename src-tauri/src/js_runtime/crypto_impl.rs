//! JS 脚本运行时（V8）——同步高性能 crypto 宿主桥。
//!
//! 算法支持：
//! - 哈希：md5, sha1, sha256, sha384, sha512, sha3 (SHA3-256)
//! - CRC32：硬件加速循环冗余校验
//! - HMAC：hmac-md5, hmac-sha1, hmac-sha256, hmac-sha384, hmac-sha512, hmac-sha3
//! - 对称加密：aes-128-cbc（PKCS7 padding）、aes-256-gcm（AEAD 硬件加速）
//! - KDF 密钥派生：pbkdf2, hkdf
//! - 现代椭圆曲线：ed25519（签名/验签/密钥对）、x25519（ECDH 共享密钥交换）
//! - NIST 椭圆曲线：p256（ECDSA 签名/验签、ECDH 协商）
//! - RSA 系列：RSA-PKCS1-v1_5 / RSA-PSS 签名验签、RSA-OAEP 公钥加密与私钥解密、密钥对生成
//! - 随机辅助：randomUUID, randomBytes
//!
//! 特性：
//! - 纯同步阻塞无异步（无 Promise、无事件循环）。
//! - 硬件级指令集加速（x86 AES-NI/SHA-NI/CLMUL/SSE4.2，ARMv8 Crypto/CRC32 Extensions），零外部动态链接库依赖。

use std::fmt::Write as _;
use aes::Aes128;
use aes::cipher::block_padding::Pkcs7;
use aes::cipher::{BlockModeDecrypt, BlockModeEncrypt, KeyIvInit};
use aes_gcm::{
    aead::{Aead, Payload},
    Aes256Gcm, Nonce,
};
use base64::prelude::*;
use ed25519_dalek::ed25519::SignatureEncoding as _;
use ed25519_dalek::Signer as _;
use hmac::{Hmac, Mac};
use md5::Md5;
use p256::pkcs8::{EncodePrivateKey as _, EncodePublicKey as _};
use rand::RngExt as _;
use rsa::signature::{RandomizedSigner as _, Verifier as _};
use sha1::Sha1;
use sha2::{Digest, Sha256, Sha384, Sha512};
use sha3::Sha3_256;

type Aes128CbcEnc = cbc::Encryptor<Aes128>;
type Aes128CbcDec = cbc::Decryptor<Aes128>;

// ── 编解码辅助 ─────────────────────────────────────────────────────────

fn bytes_to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        let _ = write!(s, "{:02x}", b);
    }
    s
}

fn hex_to_bytes(hex: &str) -> Result<Vec<u8>, String> {
    let hex = hex.trim();
    if !hex.len().is_multiple_of(2) {
        return Err("Hex 长度必须为偶数".to_string());
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| {
            let chunk = hex.get(i..i + 2).unwrap_or("");
            u8::from_str_radix(chunk, 16)
                .map_err(|e| format!("无效 Hex 字符: {e}"))
        })
        .collect()
}

fn set_formatted_output(
    scope: &mut v8::PinScope,
    rv: &mut v8::ReturnValue<v8::Value>,
    bytes: &[u8],
    encoding: &str,
) {
    match encoding.to_ascii_lowercase().as_str() {
        "base64" => {
            let b64 = BASE64_STANDARD.encode(bytes);
            if let Some(s) = v8::String::new(scope, &b64) {
                rv.set(s.into());
            }
        }
        "bytes" | "buffer" | "uint8array" => {
            let len = bytes.len();
            let ab = v8::ArrayBuffer::new(scope, len);
            if len > 0 {
                let store = ab.get_backing_store();
                if let Some(ptr) = store.data() {
                    unsafe {
                        std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr.as_ptr() as *mut u8, len);
                    }
                }
            }
            if let Some(arr) = v8::Uint8Array::new(scope, ab, 0, len) {
                rv.set(arr.into());
            }
        }
        _ => {
            let hex = bytes_to_hex(bytes);
            if let Some(s) = v8::String::new(scope, &hex) {
                rv.set(s.into());
            }
        }
    }
}

fn extract_input_bytes(
    scope: &mut v8::PinScope,
    val: v8::Local<v8::Value>,
) -> Vec<u8> {
    if val.is_string() {
        val.to_rust_string_lossy(scope).into_bytes()
    } else if val.is_array_buffer() {
        if let Ok(ab) = v8::Local::<v8::ArrayBuffer>::try_from(val)
            && let Some(ptr) = ab.get_backing_store().data()
        {
            let len = ab.byte_length();
            let slice = unsafe { std::slice::from_raw_parts(ptr.as_ptr() as *const u8, len) };
            slice.to_vec()
        } else {
            Vec::new()
        }
    } else if val.is_uint8_array() {
        if let Ok(u8arr) = v8::Local::<v8::Uint8Array>::try_from(val) {
            let len = u8arr.byte_length();
            let mut buf = vec![0u8; len];
            u8arr.copy_contents(&mut buf);
            buf
        } else {
            Vec::new()
        }
    } else {
        val.to_rust_string_lossy(scope).into_bytes()
    }
}

/// 自适应提取二进制数据（支持 Uint8Array / ArrayBuffer / Hex 串 / Base64 串 / 原生文本）
fn extract_binary_flexible(
    scope: &mut v8::PinScope,
    val: v8::Local<v8::Value>,
) -> Vec<u8> {
    if val.is_array_buffer() || val.is_uint8_array() {
        extract_input_bytes(scope, val)
    } else if val.is_string() {
        let s = val.to_rust_string_lossy(scope);
        let trimmed = s.trim();
        if trimmed.starts_with("-----BEGIN") {
            return s.into_bytes();
        }
        if trimmed.len() >= 2 && trimmed.len().is_multiple_of(2) && trimmed.chars().all(|c| c.is_ascii_hexdigit())
            && let Ok(bytes) = hex_to_bytes(trimmed) {
            return bytes;
        }
        if let Ok(bytes) = BASE64_STANDARD.decode(trimmed) {
            return bytes;
        }
        s.into_bytes()
    } else {
        extract_input_bytes(scope, val)
    }
}

fn throw_js_error(scope: &mut v8::PinScope, msg: &str) {
    let s = v8::String::new(scope, msg).unwrap_or_else(|| v8::String::empty(scope));
    let err = v8::Exception::error(scope, s);
    scope.throw_exception(err);
}

// ── 哈希与校验计算回调 ─────────────────────────────────────────────────

/// crypto.md5(data, encoding?: 'hex' | 'base64' | 'bytes')
pub fn crypto_md5_cb(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue<v8::Value>,
) {
    if args.length() == 0 {
        throw_js_error(scope, "md5 缺少输入数据参数");
        return;
    }
    let data = extract_input_bytes(scope, args.get(0));
    let encoding = if args.length() > 1 {
        args.get(1).to_rust_string_lossy(scope)
    } else {
        "hex".to_string()
    };
    let hash = Md5::digest(&data);
    set_formatted_output(scope, &mut rv, &hash, &encoding);
}

/// crypto.sha1(data, encoding?: 'hex' | 'base64' | 'bytes')
pub fn crypto_sha1_cb(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue<v8::Value>,
) {
    if args.length() == 0 {
        throw_js_error(scope, "sha1 缺少输入数据参数");
        return;
    }
    let data = extract_input_bytes(scope, args.get(0));
    let encoding = if args.length() > 1 {
        args.get(1).to_rust_string_lossy(scope)
    } else {
        "hex".to_string()
    };
    let hash = Sha1::digest(&data);
    set_formatted_output(scope, &mut rv, &hash, &encoding);
}

/// crypto.sha256(data, encoding?: 'hex' | 'base64' | 'bytes')
pub fn crypto_sha256_cb(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue<v8::Value>,
) {
    if args.length() == 0 {
        throw_js_error(scope, "sha256 缺少输入数据参数");
        return;
    }
    let data = extract_input_bytes(scope, args.get(0));
    let encoding = if args.length() > 1 {
        args.get(1).to_rust_string_lossy(scope)
    } else {
        "hex".to_string()
    };
    let hash = Sha256::digest(&data);
    set_formatted_output(scope, &mut rv, &hash, &encoding);
}

/// crypto.sha384(data, encoding?: 'hex' | 'base64' | 'bytes')
pub fn crypto_sha384_cb(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue<v8::Value>,
) {
    if args.length() == 0 {
        throw_js_error(scope, "sha384 缺少输入数据参数");
        return;
    }
    let data = extract_input_bytes(scope, args.get(0));
    let encoding = if args.length() > 1 {
        args.get(1).to_rust_string_lossy(scope)
    } else {
        "hex".to_string()
    };
    let hash = Sha384::digest(&data);
    set_formatted_output(scope, &mut rv, &hash, &encoding);
}

/// crypto.sha512(data, encoding?: 'hex' | 'base64' | 'bytes')
pub fn crypto_sha512_cb(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue<v8::Value>,
) {
    if args.length() == 0 {
        throw_js_error(scope, "sha512 缺少输入数据参数");
        return;
    }
    let data = extract_input_bytes(scope, args.get(0));
    let encoding = if args.length() > 1 {
        args.get(1).to_rust_string_lossy(scope)
    } else {
        "hex".to_string()
    };
    let hash = Sha512::digest(&data);
    set_formatted_output(scope, &mut rv, &hash, &encoding);
}

/// crypto.sha3(data, encoding?: 'hex' | 'base64' | 'bytes') —— SHA3-256
pub fn crypto_sha3_cb(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue<v8::Value>,
) {
    if args.length() == 0 {
        throw_js_error(scope, "sha3 缺少输入数据参数");
        return;
    }
    let data = extract_input_bytes(scope, args.get(0));
    let encoding = if args.length() > 1 {
        args.get(1).to_rust_string_lossy(scope)
    } else {
        "hex".to_string()
    };
    let hash = Sha3_256::digest(&data);
    set_formatted_output(scope, &mut rv, &hash, &encoding);
}

/// crypto.crc32(data, encoding?: 'number' | 'hex') -> number | string
pub fn crypto_crc32_cb(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue<v8::Value>,
) {
    if args.length() == 0 {
        throw_js_error(scope, "crc32 缺少输入数据参数");
        return;
    }
    let data = extract_input_bytes(scope, args.get(0));
    let encoding = if args.length() > 1 {
        args.get(1).to_rust_string_lossy(scope)
    } else {
        "number".to_string()
    };
    let mut hasher = crc32fast::Hasher::new();
    hasher.update(&data);
    let checksum = hasher.finalize();

    if encoding.eq_ignore_ascii_case("hex") {
        let hex = format!("{:08x}", checksum);
        if let Some(s) = v8::String::new(scope, &hex) {
            rv.set(s.into());
        }
    } else {
        rv.set(v8::Integer::new_from_unsigned(scope, checksum).into());
    }
}

/// crypto.hmac(algorithm, key, data, encoding?: 'hex' | 'base64' | 'bytes')
pub fn crypto_hmac_cb(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue<v8::Value>,
) {
    if args.length() < 3 {
        throw_js_error(scope, "hmac 需要 algorithm、key、data 至少 3 个参数");
        return;
    }
    let algo = args.get(0).to_rust_string_lossy(scope);
    let key = extract_input_bytes(scope, args.get(1));
    let data = extract_input_bytes(scope, args.get(2));
    let encoding = if args.length() > 3 {
        args.get(3).to_rust_string_lossy(scope)
    } else {
        "hex".to_string()
    };

    let normalized_algo = algo.to_ascii_lowercase().replace(['-', '_'], "");
    let mac_res: Result<Vec<u8>, String> = match normalized_algo.as_str() {
        "md5" => match <Hmac<Md5> as Mac>::new_from_slice(&key) {
            Ok(mut mac) => {
                mac.update(&data);
                Ok(mac.finalize().into_bytes().to_vec())
            }
            Err(e) => Err(format!("HMAC-MD5 密钥初始化失败: {e}")),
        },
        "sha1" => match <Hmac<Sha1> as Mac>::new_from_slice(&key) {
            Ok(mut mac) => {
                mac.update(&data);
                Ok(mac.finalize().into_bytes().to_vec())
            }
            Err(e) => Err(format!("HMAC-SHA1 密钥初始化失败: {e}")),
        },
        "sha256" => match <Hmac<Sha256> as Mac>::new_from_slice(&key) {
            Ok(mut mac) => {
                mac.update(&data);
                Ok(mac.finalize().into_bytes().to_vec())
            }
            Err(e) => Err(format!("HMAC-SHA256 密钥初始化失败: {e}")),
        },
        "sha384" => match <Hmac<Sha384> as Mac>::new_from_slice(&key) {
            Ok(mut mac) => {
                mac.update(&data);
                Ok(mac.finalize().into_bytes().to_vec())
            }
            Err(e) => Err(format!("HMAC-SHA384 密钥初始化失败: {e}")),
        },
        "sha512" => match <Hmac<Sha512> as Mac>::new_from_slice(&key) {
            Ok(mut mac) => {
                mac.update(&data);
                Ok(mac.finalize().into_bytes().to_vec())
            }
            Err(e) => Err(format!("HMAC-SHA512 密钥初始化失败: {e}")),
        },
        "sha3" | "sha3256" => match <Hmac<Sha3_256> as Mac>::new_from_slice(&key) {
            Ok(mut mac) => {
                mac.update(&data);
                Ok(mac.finalize().into_bytes().to_vec())
            }
            Err(e) => Err(format!("HMAC-SHA3-256 密钥初始化失败: {e}")),
        },
        _ => Err(format!("不支持的 HMAC 算法「{}」，支持 md5 / sha1 / sha256 / sha384 / sha512 / sha3", algo)),
    };

    match mac_res {
        Ok(bytes) => {
            set_formatted_output(scope, &mut rv, &bytes, &encoding);
        }
        Err(err_msg) => throw_js_error(scope, &err_msg),
    }
}

// ── 对称加密：aes-128-cbc ─────────────────────────────────────────────

/// crypto.aes128CbcEncrypt(key, iv, data, encoding?: 'hex' | 'base64' | 'bytes')
pub fn crypto_aes128_cbc_encrypt_cb(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue<v8::Value>,
) {
    if args.length() < 3 {
        throw_js_error(scope, "aes128CbcEncrypt 需要 key、iv、data 3 个参数");
        return;
    }
    let key = extract_input_bytes(scope, args.get(0));
    let iv = extract_input_bytes(scope, args.get(1));
    let data = extract_input_bytes(scope, args.get(2));
    let encoding = if args.length() > 3 {
        args.get(3).to_rust_string_lossy(scope)
    } else {
        "hex".to_string()
    };

    if key.len() != 16 {
        throw_js_error(scope, &format!("aes-128-cbc Key 长度必须恰为 16 字节（当前 {} 字节）", key.len()));
        return;
    }
    if iv.len() != 16 {
        throw_js_error(scope, &format!("aes-128-cbc IV 长度必须恰为 16 字节（当前 {} 字节）", iv.len()));
        return;
    }

    let k: &[u8; 16] = key.as_slice().try_into().unwrap();
    let iv_arr: &[u8; 16] = iv.as_slice().try_into().unwrap();

    let encryptor = Aes128CbcEnc::new(k.into(), iv_arr.into());
    let ciphertext = encryptor.encrypt_padded_vec::<Pkcs7>(&data);

    set_formatted_output(scope, &mut rv, &ciphertext, &encoding);
}

/// crypto.aes128CbcDecrypt(key, iv, ciphertext, outputEncoding?: 'utf8' | 'bytes')
pub fn crypto_aes128_cbc_decrypt_cb(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue<v8::Value>,
) {
    if args.length() < 3 {
        throw_js_error(scope, "aes128CbcDecrypt 需要 key、iv、ciphertext 3 个参数");
        return;
    }
    let key = extract_input_bytes(scope, args.get(0));
    let iv = extract_input_bytes(scope, args.get(1));
    let ct_val = args.get(2);
    let ciphertext = if ct_val.is_string() {
        let s = ct_val.to_rust_string_lossy(scope);
        if let Ok(bytes) = hex_to_bytes(&s) {
            bytes
        } else if let Ok(bytes) = BASE64_STANDARD.decode(&s) {
            bytes
        } else {
            s.into_bytes()
        }
    } else {
        extract_input_bytes(scope, ct_val)
    };
    let output_encoding = if args.length() > 3 {
        args.get(3).to_rust_string_lossy(scope)
    } else {
        "utf8".to_string()
    };

    if key.len() != 16 {
        throw_js_error(scope, &format!("aes-128-cbc Key 长度必须恰为 16 字节（当前 {} 字节）", key.len()));
        return;
    }
    if iv.len() != 16 {
        throw_js_error(scope, &format!("aes-128-cbc IV 长度必须恰为 16 字节（当前 {} 字节）", iv.len()));
        return;
    }

    let k: &[u8; 16] = key.as_slice().try_into().unwrap();
    let iv_arr: &[u8; 16] = iv.as_slice().try_into().unwrap();

    let decryptor = Aes128CbcDec::new(k.into(), iv_arr.into());
    match decryptor.decrypt_padded_vec::<Pkcs7>(&ciphertext) {
        Ok(plaintext) => {
            if output_encoding.eq_ignore_ascii_case("bytes") {
                let len = plaintext.len();
                let ab = v8::ArrayBuffer::new(scope, len);
                if len > 0 {
                    let store = ab.get_backing_store();
                    if let Some(ptr) = store.data() {
                        unsafe {
                            std::ptr::copy_nonoverlapping(plaintext.as_ptr(), ptr.as_ptr() as *mut u8, len);
                        }
                    }
                }
                if let Some(arr) = v8::Uint8Array::new(scope, ab, 0, len) {
                    rv.set(arr.into());
                }
            } else {
                let text = String::from_utf8_lossy(&plaintext);
                if let Some(s) = v8::String::new(scope, &text) {
                    rv.set(s.into());
                }
            }
        }
        Err(e) => {
            throw_js_error(scope, &format!("aes-128-cbc 解密失败（Padding 损坏或密钥错误）: {e:?}"));
        }
    }
}

// ── 对称加密：aes-256-gcm ─────────────────────────────────────────────

/// crypto.aes256GcmEncrypt(key, nonce, plaintext, aad?: string | Uint8Array, encoding?: 'hex' | 'base64')
pub fn crypto_aes256_gcm_encrypt_cb(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue<v8::Value>,
) {
    if args.length() < 3 {
        throw_js_error(scope, "aes256GcmEncrypt 需要 key、nonce、plaintext 3 个参数");
        return;
    }
    let key = extract_input_bytes(scope, args.get(0));
    let nonce = extract_input_bytes(scope, args.get(1));
    let plaintext = extract_input_bytes(scope, args.get(2));
    let aad = if args.length() > 3 && !args.get(3).is_undefined() && !args.get(3).is_null() {
        Some(extract_input_bytes(scope, args.get(3)))
    } else {
        None
    };
    let encoding = if args.length() > 4 {
        args.get(4).to_rust_string_lossy(scope)
    } else {
        "hex".to_string()
    };

    if key.len() != 32 {
        throw_js_error(scope, &format!("aes-256-gcm Key 长度必须恰为 32 字节（当前 {} 字节）", key.len()));
        return;
    }
    if nonce.len() != 12 {
        throw_js_error(scope, &format!("aes-256-gcm Nonce 长度必须恰为 12 字节（当前 {} 字节）", nonce.len()));
        return;
    }

    use aes_gcm::aead::KeyInit;
    let cipher = match Aes256Gcm::new_from_slice(&key) {
        Ok(c) => c,
        Err(e) => {
            throw_js_error(scope, &format!("aes-256-gcm 构造 Cipher 失败: {e}"));
            return;
        }
    };

    let nonce_obj = Nonce::from_slice(&nonce);
    let payload = Payload {
        msg: &plaintext,
        aad: aad.as_deref().unwrap_or(&[]),
    };

    match cipher.encrypt(nonce_obj, payload) {
        Ok(mut ct_with_tag) => {
            if ct_with_tag.len() < 16 {
                throw_js_error(scope, "aes-256-gcm 加密输出异常");
                return;
            }
            let tag = ct_with_tag.split_off(ct_with_tag.len() - 16);
            let ct = ct_with_tag;

            let out_obj = v8::Object::new(scope);

            let (ct_str, tag_str) = if encoding.eq_ignore_ascii_case("base64") {
                (BASE64_STANDARD.encode(&ct), BASE64_STANDARD.encode(&tag))
            } else {
                (bytes_to_hex(&ct), bytes_to_hex(&tag))
            };

            if let Some(k) = v8::String::new(scope, "ciphertext")
                && let Some(v) = v8::String::new(scope, &ct_str)
            {
                out_obj.set(scope, k.into(), v.into());
            }
            if let Some(k) = v8::String::new(scope, "tag")
                && let Some(v) = v8::String::new(scope, &tag_str)
            {
                out_obj.set(scope, k.into(), v.into());
            }

            rv.set(out_obj.into());
        }
        Err(e) => {
            throw_js_error(scope, &format!("aes-256-gcm 加密失败: {e}"));
        }
    }
}

/// crypto.aes256GcmDecrypt(key, nonce, ciphertext, tag, aad?: string | Uint8Array, outputEncoding?: 'utf8' | 'bytes')
pub fn crypto_aes256_gcm_decrypt_cb(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue<v8::Value>,
) {
    if args.length() < 4 {
        throw_js_error(scope, "aes256GcmDecrypt 需要 key、nonce、ciphertext、tag 4 个参数");
        return;
    }
    let key = extract_input_bytes(scope, args.get(0));
    let nonce = extract_input_bytes(scope, args.get(1));
    let ct_val = args.get(2);
    let mut ciphertext = if ct_val.is_string() {
        let s = ct_val.to_rust_string_lossy(scope);
        if let Ok(bytes) = hex_to_bytes(&s) {
            bytes
        } else if let Ok(bytes) = BASE64_STANDARD.decode(&s) {
            bytes
        } else {
            s.into_bytes()
        }
    } else {
        extract_input_bytes(scope, ct_val)
    };

    let tag_val = args.get(3);
    let tag = if tag_val.is_string() {
        let s = tag_val.to_rust_string_lossy(scope);
        if let Ok(bytes) = hex_to_bytes(&s) {
            bytes
        } else if let Ok(bytes) = BASE64_STANDARD.decode(&s) {
            bytes
        } else {
            s.into_bytes()
        }
    } else {
        extract_input_bytes(scope, tag_val)
    };

    let aad = if args.length() > 4 && !args.get(4).is_undefined() && !args.get(4).is_null() {
        Some(extract_input_bytes(scope, args.get(4)))
    } else {
        None
    };
    let output_encoding = if args.length() > 5 {
        args.get(5).to_rust_string_lossy(scope)
    } else {
        "utf8".to_string()
    };

    if key.len() != 32 {
        throw_js_error(scope, &format!("aes-256-gcm Key 长度必须恰为 32 字节（当前 {} 字节）", key.len()));
        return;
    }
    if nonce.len() != 12 {
        throw_js_error(scope, &format!("aes-256-gcm Nonce 长度必须恰为 12 字节（当前 {} 字节）", nonce.len()));
        return;
    }
    if tag.len() != 16 {
        throw_js_error(scope, &format!("aes-256-gcm Tag 长度必须恰为 16 字节（当前 {} 字节）", tag.len()));
        return;
    }

    use aes_gcm::aead::KeyInit;
    let cipher = match Aes256Gcm::new_from_slice(&key) {
        Ok(c) => c,
        Err(e) => {
            throw_js_error(scope, &format!("aes-256-gcm 构造 Cipher 失败: {e}"));
            return;
        }
    };

    let nonce_obj = Nonce::from_slice(&nonce);
    ciphertext.extend_from_slice(&tag);
    let payload = Payload {
        msg: &ciphertext,
        aad: aad.as_deref().unwrap_or(&[]),
    };

    match cipher.decrypt(nonce_obj, payload) {
        Ok(plaintext) => {
            if output_encoding.eq_ignore_ascii_case("bytes") {
                let len = plaintext.len();
                let ab = v8::ArrayBuffer::new(scope, len);
                if len > 0 {
                    let store = ab.get_backing_store();
                    if let Some(ptr) = store.data() {
                        unsafe {
                            std::ptr::copy_nonoverlapping(plaintext.as_ptr(), ptr.as_ptr() as *mut u8, len);
                        }
                    }
                }
                if let Some(arr) = v8::Uint8Array::new(scope, ab, 0, len) {
                    rv.set(arr.into());
                }
            } else {
                let text = String::from_utf8_lossy(&plaintext);
                if let Some(s) = v8::String::new(scope, &text) {
                    rv.set(s.into());
                }
            }
        }
        Err(e) => {
            throw_js_error(scope, &format!("aes-256-gcm 认证解密失败（Tag 校验不匹配或密文被篡改）: {e}"));
        }
    }
}

// ── KDF 密钥派生 ───────────────────────────────────────────────────────

/// crypto.pbkdf2(password, salt, iterations, keylen, digest = 'sha256', encoding = 'hex')
pub fn crypto_pbkdf2_cb(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue<v8::Value>,
) {
    if args.length() < 4 {
        throw_js_error(scope, "pbkdf2 需要 password, salt, iterations, keylen 至少 4 个参数");
        return;
    }
    let password = extract_input_bytes(scope, args.get(0));
    let salt = extract_input_bytes(scope, args.get(1));
    let iterations = args.get(2).to_integer(scope).map(|i| i.value().max(1) as u32).unwrap_or(10000);
    let keylen = args.get(3).to_integer(scope).map(|i| i.value().clamp(1, 1024 * 64) as usize).unwrap_or(32);
    let digest = if args.length() > 4 && !args.get(4).is_undefined() {
        args.get(4).to_rust_string_lossy(scope)
    } else {
        "sha256".to_string()
    };
    let encoding = if args.length() > 5 {
        args.get(5).to_rust_string_lossy(scope)
    } else {
        "hex".to_string()
    };

    let mut out = vec![0u8; keylen];
    let norm = digest.to_ascii_lowercase().replace(['-', '_'], "");
    let res = match norm.as_str() {
        "sha1" => {
            let _ = pbkdf2::pbkdf2::<Hmac<Sha1>>(&password, &salt, iterations, &mut out);
            Ok(())
        }
        "sha256" => {
            let _ = pbkdf2::pbkdf2::<Hmac<Sha256>>(&password, &salt, iterations, &mut out);
            Ok(())
        }
        "sha384" => {
            let _ = pbkdf2::pbkdf2::<Hmac<Sha384>>(&password, &salt, iterations, &mut out);
            Ok(())
        }
        "sha512" => {
            let _ = pbkdf2::pbkdf2::<Hmac<Sha512>>(&password, &salt, iterations, &mut out);
            Ok(())
        }
        _ => Err(format!("不支持的 PBKDF2 哈希摘要「{}」，支持 sha1 / sha256 / sha384 / sha512", digest)),
    };

    match res {
        Ok(()) => set_formatted_output(scope, &mut rv, &out, &encoding),
        Err(e) => throw_js_error(scope, &e),
    }
}

/// crypto.hkdf(digest = 'sha256', ikm, salt, info, length, encoding = 'hex')
pub fn crypto_hkdf_cb(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue<v8::Value>,
) {
    if args.length() < 5 {
        throw_js_error(scope, "hkdf 需要 digest, ikm, salt, info, length 5 个参数");
        return;
    }
    let digest = args.get(0).to_rust_string_lossy(scope);
    let ikm = extract_input_bytes(scope, args.get(1));
    let salt = if args.get(2).is_null() || args.get(2).is_undefined() {
        None
    } else {
        Some(extract_input_bytes(scope, args.get(2)))
    };
    let info = extract_input_bytes(scope, args.get(3));
    let length = args.get(4).to_integer(scope).map(|i| i.value().clamp(1, 1024 * 64) as usize).unwrap_or(32);
    let encoding = if args.length() > 5 {
        args.get(5).to_rust_string_lossy(scope)
    } else {
        "hex".to_string()
    };

    let mut out = vec![0u8; length];
    let norm = digest.to_ascii_lowercase().replace(['-', '_'], "");
    let res = match norm.as_str() {
        "sha1" => {
            let hk = hkdf::Hkdf::<Sha1>::new(salt.as_deref(), &ikm);
            hk.expand(&info, &mut out).map_err(|e| format!("HKDF expand 失败: {e:?}"))
        }
        "sha256" => {
            let hk = hkdf::Hkdf::<Sha256>::new(salt.as_deref(), &ikm);
            hk.expand(&info, &mut out).map_err(|e| format!("HKDF expand 失败: {e:?}"))
        }
        "sha384" => {
            let hk = hkdf::Hkdf::<Sha384>::new(salt.as_deref(), &ikm);
            hk.expand(&info, &mut out).map_err(|e| format!("HKDF expand 失败: {e:?}"))
        }
        "sha512" => {
            let hk = hkdf::Hkdf::<Sha512>::new(salt.as_deref(), &ikm);
            hk.expand(&info, &mut out).map_err(|e| format!("HKDF expand 失败: {e:?}"))
        }
        _ => Err(format!("不支持的 HKDF 摘要「{}」，支持 sha1 / sha256 / sha384 / sha512", digest)),
    };

    match res {
        Ok(()) => set_formatted_output(scope, &mut rv, &out, &encoding),
        Err(e) => throw_js_error(scope, &e),
    }
}

// ── 现代椭圆曲线：Ed25519 签名与验签 ────────────────────────────────────

/// crypto.generateEd25519KeyPair(): { publicKey: string, privateKey: string } (hex)
pub fn crypto_generate_ed25519_keypair_cb(
    scope: &mut v8::PinScope,
    _args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue<v8::Value>,
) {
    let mut rng = rand::rng();
    let signing_key = ed25519_dalek::SigningKey::generate(&mut rng);
    let verifying_key = signing_key.verifying_key();

    let priv_hex = bytes_to_hex(&signing_key.to_bytes());
    let pub_hex = bytes_to_hex(verifying_key.as_bytes());

    let out_obj = v8::Object::new(scope);
    if let Some(k) = v8::String::new(scope, "privateKey")
        && let Some(v) = v8::String::new(scope, &priv_hex)
    {
        out_obj.set(scope, k.into(), v.into());
    }
    if let Some(k) = v8::String::new(scope, "publicKey")
        && let Some(v) = v8::String::new(scope, &pub_hex)
    {
        out_obj.set(scope, k.into(), v.into());
    }
    rv.set(out_obj.into());
}

/// crypto.ed25519Sign(privateKey, message, encoding = 'hex')
pub fn crypto_ed25519_sign_cb(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue<v8::Value>,
) {
    if args.length() < 2 {
        throw_js_error(scope, "ed25519Sign 需要 privateKey, message 两个参数");
        return;
    }
    let priv_bytes = extract_binary_flexible(scope, args.get(0));
    let message = extract_input_bytes(scope, args.get(1));
    let encoding = if args.length() > 2 {
        args.get(2).to_rust_string_lossy(scope)
    } else {
        "hex".to_string()
    };

    let seed: &[u8; 32] = match priv_bytes.get(..32) {
        Some(s) => match s.try_into() {
            Ok(arr) => arr,
            Err(_) => {
                throw_js_error(scope, "ed25519 私钥长度必须至少为 32 字节");
                return;
            }
        },
        None => {
            throw_js_error(scope, "ed25519 私钥长度必须至少为 32 字节");
            return;
        }
    };

    let signing_key = ed25519_dalek::SigningKey::from_bytes(seed);
    let sig = signing_key.sign(&message);
    set_formatted_output(scope, &mut rv, &sig.to_bytes(), &encoding);
}

/// crypto.ed25519Verify(publicKey, message, signature): boolean
pub fn crypto_ed25519_verify_cb(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue<v8::Value>,
) {
    if args.length() < 3 {
        throw_js_error(scope, "ed25519Verify 需要 publicKey, message, signature 三个参数");
        return;
    }
    let pub_bytes = extract_binary_flexible(scope, args.get(0));
    let message = extract_input_bytes(scope, args.get(1));
    let sig_bytes = extract_binary_flexible(scope, args.get(2));

    let pub_arr: &[u8; 32] = match pub_bytes.as_slice().try_into() {
        Ok(arr) => arr,
        Err(_) => {
            rv.set(v8::Boolean::new(scope, false).into());
            return;
        }
    };
    let verifying_key = match ed25519_dalek::VerifyingKey::from_bytes(pub_arr) {
        Ok(vk) => vk,
        Err(_) => {
            rv.set(v8::Boolean::new(scope, false).into());
            return;
        }
    };

    let sig_arr: &[u8; 64] = match sig_bytes.as_slice().try_into() {
        Ok(arr) => arr,
        Err(_) => {
            rv.set(v8::Boolean::new(scope, false).into());
            return;
        }
    };
    let signature = ed25519_dalek::Signature::from_bytes(sig_arr);

    let ok = verifying_key.verify_strict(&message, &signature).is_ok();
    rv.set(v8::Boolean::new(scope, ok).into());
}

// ── 现代椭圆曲线：X25519 共享密钥交换 ──────────────────────────────────

/// crypto.generateX25519KeyPair(): { publicKey: string, privateKey: string } (hex)
pub fn crypto_generate_x25519_keypair_cb(
    scope: &mut v8::PinScope,
    _args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue<v8::Value>,
) {
    let mut rng = rand::rng();
    let mut priv_bytes = [0u8; 32];
    rng.fill(&mut priv_bytes);

    let secret = x25519_dalek::StaticSecret::from(priv_bytes);
    let public = x25519_dalek::PublicKey::from(&secret);

    let priv_hex = bytes_to_hex(&priv_bytes);
    let pub_hex = bytes_to_hex(public.as_bytes());

    let out_obj = v8::Object::new(scope);
    if let Some(k) = v8::String::new(scope, "privateKey")
        && let Some(v) = v8::String::new(scope, &priv_hex)
    {
        out_obj.set(scope, k.into(), v.into());
    }
    if let Some(k) = v8::String::new(scope, "publicKey")
        && let Some(v) = v8::String::new(scope, &pub_hex)
    {
        out_obj.set(scope, k.into(), v.into());
    }
    rv.set(out_obj.into());
}

/// crypto.x25519ComputeSharedSecret(privateKey, peerPublicKey, encoding = 'hex')
pub fn crypto_x25519_compute_shared_secret_cb(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue<v8::Value>,
) {
    if args.length() < 2 {
        throw_js_error(scope, "x25519ComputeSharedSecret 需要 privateKey, peerPublicKey 两个参数");
        return;
    }
    let priv_bytes = extract_binary_flexible(scope, args.get(0));
    let peer_pub_bytes = extract_binary_flexible(scope, args.get(1));
    let encoding = if args.length() > 2 {
        args.get(2).to_rust_string_lossy(scope)
    } else {
        "hex".to_string()
    };

    let priv_arr: &[u8; 32] = match priv_bytes.as_slice().try_into() {
        Ok(arr) => arr,
        Err(_) => {
            throw_js_error(scope, "x25519 私钥长度必须为 32 字节");
            return;
        }
    };
    let peer_arr: &[u8; 32] = match peer_pub_bytes.as_slice().try_into() {
        Ok(arr) => arr,
        Err(_) => {
            throw_js_error(scope, "x25519 对端公钥长度必须为 32 字节");
            return;
        }
    };

    let secret = x25519_dalek::StaticSecret::from(*priv_arr);
    let peer_pub = x25519_dalek::PublicKey::from(*peer_arr);
    let shared = secret.diffie_hellman(&peer_pub);

    set_formatted_output(scope, &mut rv, shared.as_bytes(), &encoding);
}

// ── NIST P-256 (secp256r1) 椭圆曲线 ───────────────────────────────────

fn parse_p256_private_key(bytes: &[u8]) -> Result<p256::SecretKey, String> {
    use p256::pkcs8::DecodePrivateKey as _;
    if let Ok(pem_str) = std::str::from_utf8(bytes)
        && pem_str.contains("-----BEGIN")
    {
        if let Ok(sk) = p256::SecretKey::from_pkcs8_pem(pem_str) {
            return Ok(sk);
        }
        if let Ok(sk) = p256::SecretKey::from_sec1_pem(pem_str) {
            return Ok(sk);
        }
    }
    if let Ok(sk) = p256::SecretKey::from_pkcs8_der(bytes) {
        return Ok(sk);
    }
    if let Ok(sk) = p256::SecretKey::from_slice(bytes) {
        return Ok(sk);
    }
    Err("无法解析 P-256 私钥（支持 PKCS#8 PEM、SEC1 PEM、DER 或 32 字节标量）".to_string())
}

fn parse_p256_public_key(bytes: &[u8]) -> Result<p256::PublicKey, String> {
    use p256::pkcs8::DecodePublicKey as _;
    if let Ok(pem_str) = std::str::from_utf8(bytes)
        && pem_str.contains("-----BEGIN")
        && let Ok(pk) = p256::PublicKey::from_public_key_pem(pem_str)
    {
        return Ok(pk);
    }
    if let Ok(pk) = p256::PublicKey::from_public_key_der(bytes) {
        return Ok(pk);
    }
    if let Ok(pk) = p256::PublicKey::from_sec1_bytes(bytes) {
        return Ok(pk);
    }
    Err("无法解析 P-256 公钥（支持 SPKI PEM、DER 或 SEC1 编码）".to_string())
}

/// crypto.generateP256KeyPair(): { publicKey: string, privateKey: string } (PEM)
pub fn crypto_generate_p256_keypair_cb(
    scope: &mut v8::PinScope,
    _args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue<v8::Value>,
) {
    let mut rng = rand::rng();
    #[allow(deprecated)]
    let secret = p256::SecretKey::random(&mut rng);
    let public = secret.public_key();

    let priv_pem = match secret.to_pkcs8_pem(p256::pkcs8::LineEnding::LF) {
        Ok(p) => p.to_string(),
        Err(e) => {
            throw_js_error(scope, &format!("导出 P-256 私钥 PEM 失败: {e}"));
            return;
        }
    };
    let pub_pem = match public.to_public_key_pem(p256::pkcs8::LineEnding::LF) {
        Ok(p) => p.to_string(),
        Err(e) => {
            throw_js_error(scope, &format!("导出 P-256 公钥 PEM 失败: {e}"));
            return;
        }
    };

    let out_obj = v8::Object::new(scope);
    if let Some(k) = v8::String::new(scope, "privateKey")
        && let Some(v) = v8::String::new(scope, &priv_pem)
    {
        out_obj.set(scope, k.into(), v.into());
    }
    if let Some(k) = v8::String::new(scope, "publicKey")
        && let Some(v) = v8::String::new(scope, &pub_pem)
    {
        out_obj.set(scope, k.into(), v.into());
    }
    rv.set(out_obj.into());
}

/// crypto.p256Sign(privateKey, message, encoding = 'hex') -> DER signature
pub fn crypto_p256_sign_cb(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue<v8::Value>,
) {
    if args.length() < 2 {
        throw_js_error(scope, "p256Sign 需要 privateKey, message 两个参数");
        return;
    }
    let priv_bytes = extract_binary_flexible(scope, args.get(0));
    let message = extract_input_bytes(scope, args.get(1));
    let encoding = if args.length() > 2 {
        args.get(2).to_rust_string_lossy(scope)
    } else {
        "hex".to_string()
    };

    let secret = match parse_p256_private_key(&priv_bytes) {
        Ok(s) => s,
        Err(e) => {
            throw_js_error(scope, &e);
            return;
        }
    };

    let signing_key = p256::ecdsa::SigningKey::from(&secret);
    let signature: p256::ecdsa::Signature = signing_key.sign(&message);
    let der = signature.to_der();
    set_formatted_output(scope, &mut rv, der.as_bytes(), &encoding);
}

/// crypto.p256Verify(publicKey, message, signature): boolean
pub fn crypto_p256_verify_cb(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue<v8::Value>,
) {
    if args.length() < 3 {
        throw_js_error(scope, "p256Verify 需要 publicKey, message, signature 三个参数");
        return;
    }
    let pub_bytes = extract_binary_flexible(scope, args.get(0));
    let message = extract_input_bytes(scope, args.get(1));
    let sig_bytes = extract_binary_flexible(scope, args.get(2));

    let public_key = match parse_p256_public_key(&pub_bytes) {
        Ok(pk) => pk,
        Err(_) => {
            rv.set(v8::Boolean::new(scope, false).into());
            return;
        }
    };

    let verifying_key = p256::ecdsa::VerifyingKey::from(&public_key);
    let signature = match p256::ecdsa::Signature::from_der(&sig_bytes) {
        Ok(s) => s,
        Err(_) => match p256::ecdsa::Signature::from_slice(&sig_bytes) {
            Ok(s) => s,
            Err(_) => {
                rv.set(v8::Boolean::new(scope, false).into());
                return;
            }
        },
    };

    let ok = verifying_key.verify(&message, &signature).is_ok();
    rv.set(v8::Boolean::new(scope, ok).into());
}

/// crypto.p256ComputeSharedSecret(privateKey, peerPublicKey, encoding = 'hex')
pub fn crypto_p256_compute_shared_secret_cb(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue<v8::Value>,
) {
    if args.length() < 2 {
        throw_js_error(scope, "p256ComputeSharedSecret 需要 privateKey, peerPublicKey 两个参数");
        return;
    }
    let priv_bytes = extract_binary_flexible(scope, args.get(0));
    let peer_pub_bytes = extract_binary_flexible(scope, args.get(1));
    let encoding = if args.length() > 2 {
        args.get(2).to_rust_string_lossy(scope)
    } else {
        "hex".to_string()
    };

    let secret = match parse_p256_private_key(&priv_bytes) {
        Ok(s) => s,
        Err(e) => {
            throw_js_error(scope, &e);
            return;
        }
    };
    let peer_pub = match parse_p256_public_key(&peer_pub_bytes) {
        Ok(pk) => pk,
        Err(e) => {
            throw_js_error(scope, &e);
            return;
        }
    };

    let shared = p256::ecdh::diffie_hellman(secret.to_nonzero_scalar(), peer_pub.as_affine());
    let raw_secret = shared.raw_secret_bytes();
    set_formatted_output(scope, &mut rv, raw_secret.as_slice(), &encoding);
}

// ── RSA 系列（加解密、签名验签、密钥对生成） ─────────────────────────

fn parse_rsa_private_key(bytes: &[u8]) -> Result<rsa::RsaPrivateKey, String> {
    use rsa::pkcs8::DecodePrivateKey as _;
    if let Ok(pem_str) = std::str::from_utf8(bytes)
        && pem_str.contains("-----BEGIN")
    {
        if let Ok(sk) = rsa::RsaPrivateKey::from_pkcs8_pem(pem_str) {
            return Ok(sk);
        }
        use rsa::pkcs1::DecodeRsaPrivateKey as _;
        if let Ok(sk) = rsa::RsaPrivateKey::from_pkcs1_pem(pem_str) {
            return Ok(sk);
        }
    }
    if let Ok(sk) = rsa::RsaPrivateKey::from_pkcs8_der(bytes) {
        return Ok(sk);
    }
    use rsa::pkcs1::DecodeRsaPrivateKey as _;
    if let Ok(sk) = rsa::RsaPrivateKey::from_pkcs1_der(bytes) {
        return Ok(sk);
    }
    Err("无法解析 RSA 私钥（支持 PKCS#8 或 PKCS#1 PEM/DER 格式）".to_string())
}

fn parse_rsa_public_key(bytes: &[u8]) -> Result<rsa::RsaPublicKey, String> {
    use rsa::pkcs8::DecodePublicKey as _;
    if let Ok(pem_str) = std::str::from_utf8(bytes)
        && pem_str.contains("-----BEGIN")
    {
        if let Ok(pk) = rsa::RsaPublicKey::from_public_key_pem(pem_str) {
            return Ok(pk);
        }
        use rsa::pkcs1::DecodeRsaPublicKey as _;
        if let Ok(pk) = rsa::RsaPublicKey::from_pkcs1_pem(pem_str) {
            return Ok(pk);
        }
    }
    if let Ok(pk) = rsa::RsaPublicKey::from_public_key_der(bytes) {
        return Ok(pk);
    }
    use rsa::pkcs1::DecodeRsaPublicKey as _;
    if let Ok(pk) = rsa::RsaPublicKey::from_pkcs1_der(bytes) {
        return Ok(pk);
    }
    Err("无法解析 RSA 公钥（支持 SPKI 或 PKCS#1 PEM/DER 格式）".to_string())
}

fn get_object_string_prop(
    scope: &mut v8::PinScope,
    obj_val: v8::Local<v8::Value>,
    prop_name: &str,
) -> Option<String> {
    if obj_val.is_object()
        && let Ok(obj) = v8::Local::<v8::Object>::try_from(obj_val)
        && let Some(k) = v8::String::new(scope, prop_name)
        && let Some(v) = obj.get(scope, k.into())
        && v.is_string()
    {
        Some(v.to_rust_string_lossy(scope))
    } else {
        None
    }
}

/// crypto.generateRsaKeyPair(bits = 2048): { publicKey: string, privateKey: string }
pub fn crypto_generate_rsa_keypair_cb(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue<v8::Value>,
) {
    let bits = if args.length() > 0 && args.get(0).is_number() {
        args.get(0).to_integer(scope).map(|i| i.value().clamp(1024, 4096) as usize).unwrap_or(2048)
    } else {
        2048
    };

    let mut rng = rand::rng();
    let priv_key = match rsa::RsaPrivateKey::new(&mut rng, bits) {
        Ok(k) => k,
        Err(e) => {
            throw_js_error(scope, &format!("生成 RSA 密钥对失败: {e}"));
            return;
        }
    };
    let pub_key = rsa::RsaPublicKey::from(&priv_key);

    let priv_pem = match priv_key.to_pkcs8_pem(rsa::pkcs8::LineEnding::LF) {
        Ok(p) => p.to_string(),
        Err(e) => {
            throw_js_error(scope, &format!("导出 RSA 私钥 PEM 失败: {e}"));
            return;
        }
    };
    let pub_pem = match pub_key.to_public_key_pem(rsa::pkcs8::LineEnding::LF) {
        Ok(p) => p.to_string(),
        Err(e) => {
            throw_js_error(scope, &format!("导出 RSA 公钥 PEM 失败: {e}"));
            return;
        }
    };

    let out_obj = v8::Object::new(scope);
    if let Some(k) = v8::String::new(scope, "privateKey")
        && let Some(v) = v8::String::new(scope, &priv_pem)
    {
        out_obj.set(scope, k.into(), v.into());
    }
    if let Some(k) = v8::String::new(scope, "publicKey")
        && let Some(v) = v8::String::new(scope, &pub_pem)
    {
        out_obj.set(scope, k.into(), v.into());
    }
    rv.set(out_obj.into());
}

/// crypto.rsaEncrypt(publicKey, data, options?: { padding?: 'oaep', hash?: 'sha256' | 'sha1' | 'sha512' }, encoding?: 'hex' | 'base64')
pub fn crypto_rsa_encrypt_cb(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue<v8::Value>,
) {
    if args.length() < 2 {
        throw_js_error(scope, "rsaEncrypt 需要 publicKey, data 至少 2 个参数");
        return;
    }
    let pub_bytes = extract_binary_flexible(scope, args.get(0));
    let data = extract_input_bytes(scope, args.get(1));

    let hash_algo = if args.length() > 2 {
        get_object_string_prop(scope, args.get(2), "hash").unwrap_or_else(|| "sha256".to_string())
    } else {
        "sha256".to_string()
    };
    let encoding = if args.length() > 3 {
        args.get(3).to_rust_string_lossy(scope)
    } else {
        "hex".to_string()
    };

    let pub_key = match parse_rsa_public_key(&pub_bytes) {
        Ok(pk) => pk,
        Err(e) => {
            throw_js_error(scope, &e);
            return;
        }
    };

    let mut rng = rand::rng();
    let norm_hash = hash_algo.to_ascii_lowercase().replace('-', "");
    let enc_res = match norm_hash.as_str() {
        "sha1" => pub_key.encrypt(&mut rng, rsa::Oaep::<rsa::sha2::Sha256>::new(), &data),
        "sha512" => pub_key.encrypt(&mut rng, rsa::Oaep::<rsa::sha2::Sha512>::new(), &data),
        _ => pub_key.encrypt(&mut rng, rsa::Oaep::<rsa::sha2::Sha256>::new(), &data),
    };

    match enc_res {
        Ok(ct) => set_formatted_output(scope, &mut rv, &ct, &encoding),
        Err(e) => throw_js_error(scope, &format!("RSA-OAEP 加密失败（数据过长或公钥异常）: {e}")),
    }
}

/// crypto.rsaDecrypt(privateKey, ciphertext, options?: { padding?: 'oaep', hash?: 'sha256' | 'sha1' | 'sha512' }, outputEncoding?: 'utf8' | 'bytes')
pub fn crypto_rsa_decrypt_cb(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue<v8::Value>,
) {
    if args.length() < 2 {
        throw_js_error(scope, "rsaDecrypt 需要 privateKey, ciphertext 至少 2 个参数");
        return;
    }
    let priv_bytes = extract_binary_flexible(scope, args.get(0));
    let ct_bytes = extract_binary_flexible(scope, args.get(1));

    let hash_algo = if args.length() > 2 {
        get_object_string_prop(scope, args.get(2), "hash").unwrap_or_else(|| "sha256".to_string())
    } else {
        "sha256".to_string()
    };
    let output_encoding = if args.length() > 3 {
        args.get(3).to_rust_string_lossy(scope)
    } else {
        "utf8".to_string()
    };

    let priv_key = match parse_rsa_private_key(&priv_bytes) {
        Ok(sk) => sk,
        Err(e) => {
            throw_js_error(scope, &e);
            return;
        }
    };

    let norm_hash = hash_algo.to_ascii_lowercase().replace('-', "");
    let dec_res = match norm_hash.as_str() {
        "sha1" => priv_key.decrypt(rsa::Oaep::<rsa::sha2::Sha256>::new(), &ct_bytes),
        "sha512" => priv_key.decrypt(rsa::Oaep::<rsa::sha2::Sha512>::new(), &ct_bytes),
        _ => priv_key.decrypt(rsa::Oaep::<rsa::sha2::Sha256>::new(), &ct_bytes),
    };

    match dec_res {
        Ok(plaintext) => {
            if output_encoding.eq_ignore_ascii_case("bytes") {
                let len = plaintext.len();
                let ab = v8::ArrayBuffer::new(scope, len);
                if len > 0 {
                    let store = ab.get_backing_store();
                    if let Some(ptr) = store.data() {
                        unsafe {
                            std::ptr::copy_nonoverlapping(plaintext.as_ptr(), ptr.as_ptr() as *mut u8, len);
                        }
                    }
                }
                if let Some(arr) = v8::Uint8Array::new(scope, ab, 0, len) {
                    rv.set(arr.into());
                }
            } else {
                let text = String::from_utf8_lossy(&plaintext);
                if let Some(s) = v8::String::new(scope, &text) {
                    rv.set(s.into());
                }
            }
        }
        Err(e) => throw_js_error(scope, &format!("RSA-OAEP 解密失败（密文损坏或私钥不匹配）: {e}")),
    }
}

/// crypto.rsaSign(privateKey, data, options?: { padding?: 'pss' | 'pkcs1', hash?: 'sha256' | 'sha384' | 'sha512' | 'sha1' }, encoding?: 'hex' | 'base64')
pub fn crypto_rsa_sign_cb(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue<v8::Value>,
) {
    if args.length() < 2 {
        throw_js_error(scope, "rsaSign 需要 privateKey, data 至少 2 个参数");
        return;
    }
    let priv_bytes = extract_binary_flexible(scope, args.get(0));
    let data = extract_input_bytes(scope, args.get(1));

    let mut padding = "pss".to_string();
    let mut hash_algo = "sha256".to_string();
    if args.length() > 2 {
        if let Some(p) = get_object_string_prop(scope, args.get(2), "padding") {
            padding = p;
        }
        if let Some(h) = get_object_string_prop(scope, args.get(2), "hash") {
            hash_algo = h;
        }
    }
    let encoding = if args.length() > 3 {
        args.get(3).to_rust_string_lossy(scope)
    } else {
        "hex".to_string()
    };

    let priv_key = match parse_rsa_private_key(&priv_bytes) {
        Ok(sk) => sk,
        Err(e) => {
            throw_js_error(scope, &e);
            return;
        }
    };

    let mut rng = rand::rng();
    let norm_hash = hash_algo.to_ascii_lowercase().replace('-', "");
    let is_pkcs1 = padding.to_ascii_lowercase().contains("pkcs1");

    let sig_bytes: Result<Vec<u8>, String> = if is_pkcs1 {
        match norm_hash.as_str() {
            "sha384" => {
                let key = rsa::pkcs1v15::SigningKey::<rsa::sha2::Sha384>::new(priv_key);
                Ok(key.sign(&data).to_vec())
            }
            "sha512" => {
                let key = rsa::pkcs1v15::SigningKey::<rsa::sha2::Sha512>::new(priv_key);
                Ok(key.sign(&data).to_vec())
            }
            _ => {
                let key = rsa::pkcs1v15::SigningKey::<rsa::sha2::Sha256>::new(priv_key);
                Ok(key.sign(&data).to_vec())
            }
        }
    } else {
        // PSS padding
        match norm_hash.as_str() {
            "sha384" => {
                let key = rsa::pss::SigningKey::<rsa::sha2::Sha384>::new(priv_key);
                Ok(key.sign_with_rng(&mut rng, &data).to_vec())
            }
            "sha512" => {
                let key = rsa::pss::SigningKey::<rsa::sha2::Sha512>::new(priv_key);
                Ok(key.sign_with_rng(&mut rng, &data).to_vec())
            }
            _ => {
                let key = rsa::pss::SigningKey::<rsa::sha2::Sha256>::new(priv_key);
                Ok(key.sign_with_rng(&mut rng, &data).to_vec())
            }
        }
    };

    match sig_bytes {
        Ok(bytes) => set_formatted_output(scope, &mut rv, &bytes, &encoding),
        Err(e) => throw_js_error(scope, &e),
    }
}

/// crypto.rsaVerify(publicKey, data, signature, options?: { padding?: 'pss' | 'pkcs1', hash?: 'sha256' | 'sha384' | 'sha512' | 'sha1' }): boolean
pub fn crypto_rsa_verify_cb(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue<v8::Value>,
) {
    if args.length() < 3 {
        throw_js_error(scope, "rsaVerify 需要 publicKey, data, signature 至少 3 个参数");
        return;
    }
    let pub_bytes = extract_binary_flexible(scope, args.get(0));
    let data = extract_input_bytes(scope, args.get(1));
    let sig_bytes = extract_binary_flexible(scope, args.get(2));

    let mut padding = "pss".to_string();
    let mut hash_algo = "sha256".to_string();
    if args.length() > 3 {
        if let Some(p) = get_object_string_prop(scope, args.get(3), "padding") {
            padding = p;
        }
        if let Some(h) = get_object_string_prop(scope, args.get(3), "hash") {
            hash_algo = h;
        }
    }

    let pub_key = match parse_rsa_public_key(&pub_bytes) {
        Ok(pk) => pk,
        Err(_) => {
            rv.set(v8::Boolean::new(scope, false).into());
            return;
        }
    };

    let norm_hash = hash_algo.to_ascii_lowercase().replace('-', "");
    let is_pkcs1 = padding.to_ascii_lowercase().contains("pkcs1");

    let ok = if is_pkcs1 {
        let sig: rsa::pkcs1v15::Signature = match sig_bytes.as_slice().try_into() {
            Ok(s) => s,
            Err(_) => {
                rv.set(v8::Boolean::new(scope, false).into());
                return;
            }
        };
        match norm_hash.as_str() {
            "sha384" => {
                let vk = rsa::pkcs1v15::VerifyingKey::<rsa::sha2::Sha384>::new(pub_key);
                vk.verify(&data, &sig).is_ok()
            }
            "sha512" => {
                let vk = rsa::pkcs1v15::VerifyingKey::<rsa::sha2::Sha512>::new(pub_key);
                vk.verify(&data, &sig).is_ok()
            }
            _ => {
                let vk = rsa::pkcs1v15::VerifyingKey::<rsa::sha2::Sha256>::new(pub_key);
                vk.verify(&data, &sig).is_ok()
            }
        }
    } else {
        let sig: rsa::pss::Signature = match sig_bytes.as_slice().try_into() {
            Ok(s) => s,
            Err(_) => {
                rv.set(v8::Boolean::new(scope, false).into());
                return;
            }
        };
        match norm_hash.as_str() {
            "sha384" => {
                let vk = rsa::pss::VerifyingKey::<rsa::sha2::Sha384>::new(pub_key);
                vk.verify(&data, &sig).is_ok()
            }
            "sha512" => {
                let vk = rsa::pss::VerifyingKey::<rsa::sha2::Sha512>::new(pub_key);
                vk.verify(&data, &sig).is_ok()
            }
            _ => {
                let vk = rsa::pss::VerifyingKey::<rsa::sha2::Sha256>::new(pub_key);
                vk.verify(&data, &sig).is_ok()
            }
        }
    };

    rv.set(v8::Boolean::new(scope, ok).into());
}

// ── 随机辅助 ───────────────────────────────────────────────────────────

/// crypto.randomUUID(): string
pub fn crypto_random_uuid_cb(
    scope: &mut v8::PinScope,
    _args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue<v8::Value>,
) {
    let id = uuid::Uuid::new_v4().to_string();
    if let Some(s) = v8::String::new(scope, &id) {
        rv.set(s.into());
    }
}

/// crypto.randomBytes(size: number): Uint8Array
pub fn crypto_random_bytes_cb(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue<v8::Value>,
) {
    let size = if args.length() > 0 && args.get(0).is_number() {
        args.get(0).to_integer(scope).map(|i| i.value().clamp(0, 1024 * 1024) as usize).unwrap_or(32)
    } else {
        32
    };

    let mut buf = vec![0u8; size];
    let mut fill_idx = 0;
    while fill_idx < size {
        let chunk = uuid::Uuid::new_v4();
        let bytes = chunk.as_bytes();
        let take = (size - fill_idx).min(16);
        if let (Some(dest), Some(src)) = (buf.get_mut(fill_idx..fill_idx + take), bytes.get(..take)) {
            dest.copy_from_slice(src);
        }
        fill_idx += take;
    }

    let ab = v8::ArrayBuffer::new(scope, size);
    if size > 0 {
        let store = ab.get_backing_store();
        if let Some(ptr) = store.data() {
            unsafe {
                std::ptr::copy_nonoverlapping(buf.as_ptr(), ptr.as_ptr() as *mut u8, size);
            }
        }
    }
    if let Some(arr) = v8::Uint8Array::new(scope, ab, 0, size) {
        rv.set(arr.into());
    }
}

// ── 注册到 V8 ─────────────────────────────────────────────────────────

pub fn register_crypto_methods(
    scope: &mut v8::PinScope,
    crypto_tmpl: &v8::Local<v8::ObjectTemplate>,
) {
    macro_rules! bind {
        ($name:expr, $cb:expr) => {
            if let Some(k) = v8::String::new(scope, $name) {
                let f = v8::FunctionTemplate::new(scope, $cb);
                crypto_tmpl.set(k.into(), f.into());
            }
        };
    }

    // 哈希与校验
    bind!("md5", crypto_md5_cb);
    bind!("sha1", crypto_sha1_cb);
    bind!("sha256", crypto_sha256_cb);
    bind!("sha384", crypto_sha384_cb);
    bind!("sha512", crypto_sha512_cb);
    bind!("sha3", crypto_sha3_cb);
    bind!("crc32", crypto_crc32_cb);
    bind!("hmac", crypto_hmac_cb);

    // 对称加密
    bind!("aes128CbcEncrypt", crypto_aes128_cbc_encrypt_cb);
    bind!("aes128CbcDecrypt", crypto_aes128_cbc_decrypt_cb);
    bind!("aes256GcmEncrypt", crypto_aes256_gcm_encrypt_cb);
    bind!("aes256GcmDecrypt", crypto_aes256_gcm_decrypt_cb);

    // KDF 密钥派生
    bind!("pbkdf2", crypto_pbkdf2_cb);
    bind!("hkdf", crypto_hkdf_cb);

    // 现代椭圆曲线（Curve25519）
    bind!("generateEd25519KeyPair", crypto_generate_ed25519_keypair_cb);
    bind!("ed25519Sign", crypto_ed25519_sign_cb);
    bind!("ed25519Verify", crypto_ed25519_verify_cb);
    bind!("generateX25519KeyPair", crypto_generate_x25519_keypair_cb);
    bind!("x25519ComputeSharedSecret", crypto_x25519_compute_shared_secret_cb);

    // NIST 椭圆曲线（P-256 / secp256r1）
    bind!("generateP256KeyPair", crypto_generate_p256_keypair_cb);
    bind!("p256Sign", crypto_p256_sign_cb);
    bind!("p256Verify", crypto_p256_verify_cb);
    bind!("p256ComputeSharedSecret", crypto_p256_compute_shared_secret_cb);

    // RSA 系列
    bind!("generateRsaKeyPair", crypto_generate_rsa_keypair_cb);
    bind!("rsaEncrypt", crypto_rsa_encrypt_cb);
    bind!("rsaDecrypt", crypto_rsa_decrypt_cb);
    bind!("rsaSign", crypto_rsa_sign_cb);
    bind!("rsaVerify", crypto_rsa_verify_cb);

    // 随机辅助
    bind!("randomUUID", crypto_random_uuid_cb);
    bind!("randomBytes", crypto_random_bytes_cb);
}
