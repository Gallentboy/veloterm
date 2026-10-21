//! X.509 证书与 SSL 解析器：解析 PEM / DER 格式证书，提取域名 (SANs)、颁发者、
//! 过期时间与倒计时、序列号与指纹等结构化元数据。

use serde::Serialize;
use sha1::Digest;
use std::time::{SystemTime, UNIX_EPOCH};
use x509_parser::certificate::X509Certificate;
use x509_parser::extensions::{GeneralName, ParsedExtension};
use x509_parser::pem::parse_x509_pem;

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CertificateInfo {
    pub subject_cn: String,
    pub subject_full: String,
    pub issuer_cn: String,
    pub issuer_full: String,
    pub not_before: String,
    pub not_after: String,
    pub not_after_timestamp: i64,
    pub days_remaining: i64,
    pub status: String,
    pub sans: Vec<String>,
    pub serial_number: String,
    pub signature_algorithm: String,
    pub public_key_info: String,
    pub sha256_fingerprint: String,
    pub sha1_fingerprint: String,
    pub is_self_signed: bool,
    pub is_ca: bool,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CertParseResult {
    pub certificates: Vec<CertificateInfo>,
    pub raw_pem: String,
}

fn format_colon_hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

fn parse_single_cert(cert: &X509Certificate<'_>, der_bytes: &[u8]) -> CertificateInfo {
    // 1. Subject 与 Issuer 提取
    let subject_full = cert.subject().to_string();
    let subject_cn = cert
        .subject()
        .iter_common_name()
        .next()
        .and_then(|cn| cn.as_str().ok())
        .map(String::from)
        .unwrap_or_else(|| subject_full.clone());

    let issuer_full = cert.issuer().to_string();
    let issuer_cn = cert
        .issuer()
        .iter_common_name()
        .next()
        .and_then(|cn| cn.as_str().ok())
        .map(String::from)
        .unwrap_or_else(|| issuer_full.clone());

    let is_self_signed = cert.subject() == cert.issuer();

    // 2. 有效期与剩余天数计算
    let not_before = cert.validity().not_before.to_string();
    let not_after = cert.validity().not_after.to_string();
    let not_after_timestamp = cert.validity().not_after.timestamp();

    let now_ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    let sec_remaining = not_after_timestamp - now_ts;
    let days_remaining = sec_remaining / 86400;

    let status = if sec_remaining < 0 {
        "expired".to_string()
    } else if days_remaining <= 30 {
        "expiring_soon".to_string()
    } else {
        "valid".to_string()
    };

    // 3. SANs (Subject Alternative Names)
    let mut sans = Vec::new();
    let mut is_ca = false;

    for ext in cert.iter_extensions() {
        match ext.parsed_extension() {
            ParsedExtension::SubjectAlternativeName(san) => {
                for name in &san.general_names {
                    match name {
                        GeneralName::DNSName(dns) => sans.push(dns.to_string()),
                        GeneralName::IPAddress(ip) => {
                            if let [a, b, c, d] = ip[..] {
                                sans.push(format!("{a}.{b}.{c}.{d}"));
                            } else {
                                sans.push(format_colon_hex(ip));
                            }
                        }
                        GeneralName::URI(uri) => sans.push(uri.to_string()),
                        GeneralName::RFC822Name(mail) => sans.push(mail.to_string()),
                        _ => {}
                    }
                }
            }
            ParsedExtension::BasicConstraints(bc) => {
                is_ca = bc.ca;
            }
            _ => {}
        }
    }

    // 4. 序列号
    let serial_number = format_colon_hex(cert.raw_serial());

    // 5. 签名与公钥算法信息
    let signature_algorithm = cert.signature_algorithm.algorithm.to_string();
    let pub_key = cert.public_key();
    let pub_alg = pub_key.algorithm.algorithm.to_string();
    let pub_bits = pub_key.raw.len() * 8;
    let public_key_info = format!("{pub_alg} (~{pub_bits} bits)");

    // 6. 指纹
    let sha256 = sha2::Sha256::digest(der_bytes);
    let sha256_fingerprint = format_colon_hex(&sha256);

    let sha1 = sha1::Sha1::digest(der_bytes);
    let sha1_fingerprint = format_colon_hex(&sha1);

    CertificateInfo {
        subject_cn,
        subject_full,
        issuer_cn,
        issuer_full,
        not_before,
        not_after,
        not_after_timestamp,
        days_remaining,
        status,
        sans,
        serial_number,
        signature_algorithm,
        public_key_info,
        sha256_fingerprint,
        sha1_fingerprint,
        is_self_signed,
        is_ca,
    }
}

/// 解析 X.509 证书内容（支持单证书、证书链、PEM 与裸 DER）
#[tauri::command]
pub fn parse_certificate(content: String) -> Result<CertParseResult, String> {
    let mut certificates = Vec::new();
    let input = content.trim();

    // 1. 尝试按 PEM 解析
    let mut remaining = input.as_bytes();
    while let Ok((rem, pem)) = parse_x509_pem(remaining) {
        if let Ok((_, cert)) = x509_parser::parse_x509_certificate(&pem.contents) {
            certificates.push(parse_single_cert(&cert, &pem.contents));
        }
        remaining = rem;
        if remaining.is_empty() {
            break;
        }
    }

    // 2. 如果 PEM 未解析出任何证书，尝试作为裸 DER 格式解析
    if certificates.is_empty()
        && let Ok((_, cert)) = x509_parser::parse_x509_certificate(input.as_bytes())
    {
        certificates.push(parse_single_cert(&cert, input.as_bytes()));
    }

    if certificates.is_empty() {
        return Err("无法识别为合法的 X.509 证书格式（支持 .crt / .pem / .cer / .key）".to_string());
    }

    Ok(CertParseResult {
        certificates,
        raw_pem: content,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_invalid_cert() {
        let res = parse_certificate("not a cert".to_string());
        assert!(res.is_err());
    }
}
