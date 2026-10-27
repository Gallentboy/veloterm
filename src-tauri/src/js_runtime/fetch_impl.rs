//! JS 脚本运行时（V8）——同步 fetch 宿主桥。
//!
//! 执行模型与安全约束：
//! - 同步语义：无事件循环环境下的直接阻塞调用，调用后立即返回 SyncResponse 对象（非 Promise）。
//! - 线程安全：宿主线程为普通 OS 线程，调用专用 Tokio Runtime 的 `block_on` 安全阻塞，不影响主应用 IPC。
//! - 双重打断保障：`tokio::select!` 结合用户取消信号（AtomicBool）与脚本剩余超时，解决 V8 无法跨线程打断 Rust 原生阻塞 IO 的痛点。
//! - 内存守卫：单次请求响应体限制最大 30MB，防止恶意/超大响应撑爆 V8 堆。

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

/// 响应体最大字节数（30MB）
const MAX_BODY_BYTES: usize = 30 * 1024 * 1024;
/// 默认单次请求超时（10 秒）
const DEFAULT_FETCH_TIMEOUT: Duration = Duration::from_secs(10);

/// 专用于 JS 异步网络调用的 Tokio Runtime 与 HTTP Client
static ASYNC_BRIDGE: OnceLock<(tokio::runtime::Runtime, reqwest::Client)> = OnceLock::new();

fn async_bridge() -> &'static (tokio::runtime::Runtime, reqwest::Client) {
    ASYNC_BRIDGE.get_or_init(|| {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("js-fetch-bridge")
            .enable_all()
            .build()
            .expect("Failed to initialize JS fetch bridge runtime");
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::limited(10))
            .connect_timeout(Duration::from_secs(10))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        (rt, client)
    })
}

/// 注入 Isolate slot 的上下文：携带当前脚本的取消旗标与超时基线
pub struct FetchContext {
    pub cancel: Option<Arc<AtomicBool>>,
    pub started: Instant,
    pub timeout: Duration,
}

/// 全局 `fetch(url, options)` 回调
pub fn sync_fetch_cb(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue<v8::Value>,
) {
    if args.length() == 0 {
        throw_js_error(scope, "fetch 需要至少 1 个 URL 参数");
        return;
    }

    let url_val = args.get(0);
    let url = url_val.to_rust_string_lossy(scope);
    if !url.starts_with("http://") && !url.starts_with("https://") {
        throw_js_error(scope, "fetch 仅支持 http:// 与 https:// 协议 URL");
        return;
    }

    // 解析 options（可选）
    let mut method = "GET".to_string();
    let mut headers: HashMap<String, String> = HashMap::new();
    let mut body_bytes: Option<Vec<u8>> = None;
    let mut req_timeout = DEFAULT_FETCH_TIMEOUT;

    if args.length() > 1 && args.get(1).is_object() {
        let opts = args.get(1).to_object(scope).unwrap();

        // method
        if let Some(k) = v8::String::new(scope, "method")
            && let Some(v) = opts.get(scope, k.into())
            && !v.is_undefined()
            && !v.is_null()
        {
            method = v.to_rust_string_lossy(scope).to_ascii_uppercase();
        }

        // headers
        if let Some(k) = v8::String::new(scope, "headers")
            && let Some(v) = opts.get(scope, k.into())
            && v.is_object()
            && let Some(headers_obj) = v.to_object(scope)
            && let Some(prop_names) = headers_obj.get_own_property_names(scope, Default::default())
        {
            let len = prop_names.length();
            for i in 0..len {
                if let (Some(key_val), Some(val_val)) = (
                    prop_names.get_index(scope, i),
                    prop_names.get_index(scope, i).and_then(|k| headers_obj.get(scope, k)),
                ) {
                    let k_str = key_val.to_rust_string_lossy(scope);
                    let v_str = val_val.to_rust_string_lossy(scope);
                    headers.insert(k_str, v_str);
                }
            }
        }

        // body
        if let Some(k) = v8::String::new(scope, "body")
            && let Some(v) = opts.get(scope, k.into())
            && !v.is_undefined()
            && !v.is_null()
        {
            if v.is_string() {
                let s = v.to_rust_string_lossy(scope);
                body_bytes = Some(s.into_bytes());
            } else if v.is_array_buffer() {
                let ab = v8::Local::<v8::ArrayBuffer>::try_from(v).unwrap();
                let store = ab.get_backing_store();
                if let Some(ptr) = store.data() {
                    let len = ab.byte_length();
                    let slice = unsafe { std::slice::from_raw_parts(ptr.as_ptr() as *const u8, len) };
                    body_bytes = Some(slice.to_vec());
                }
            } else if v.is_uint8_array() {
                let u8arr = v8::Local::<v8::Uint8Array>::try_from(v).unwrap();
                let len = u8arr.byte_length();
                let mut buf = vec![0u8; len];
                u8arr.copy_contents(&mut buf);
                body_bytes = Some(buf);
            } else {
                let s = v.to_rust_string_lossy(scope);
                body_bytes = Some(s.into_bytes());
            }
        }

        // timeout
        if let Some(k) = v8::String::new(scope, "timeout")
            && let Some(v) = opts.get(scope, k.into())
            && v.is_number()
            && let Some(ms) = v.to_integer(scope)
        {
            let ms_val = ms.value();
            if ms_val > 0 {
                req_timeout = Duration::from_millis(ms_val as u64);
            }
        }
    }

    // 从 slot 获取脚本上下文，计算有效超时与取消旗标
    let ctx = scope.get_slot::<FetchContext>();
    let cancel_flag = ctx.and_then(|c| c.cancel.clone());
    let remaining_timeout = if let Some(c) = ctx {
        c.timeout.saturating_sub(c.started.elapsed())
    } else {
        DEFAULT_FETCH_TIMEOUT
    };
    let effective_timeout = req_timeout.min(remaining_timeout);

    let (rt, client) = async_bridge();

    let exec_res = rt.block_on(async {
        let fetch_fut = async {
            let mut req = match method.as_str() {
                "POST" => client.post(&url),
                "PUT" => client.put(&url),
                "DELETE" => client.delete(&url),
                "PATCH" => client.patch(&url),
                "HEAD" => client.head(&url),
                _ => client.get(&url),
            };

            for (k, v) in &headers {
                if let (Ok(hname), Ok(hval)) = (
                    reqwest::header::HeaderName::from_bytes(k.as_bytes()),
                    reqwest::header::HeaderValue::from_str(v),
                ) {
                    req = req.header(hname, hval);
                }
            }

            if let Some(body) = body_bytes {
                req = req.body(body);
            }

            let resp = req.send().await.map_err(|e| format!("连接失败: {e}"))?;
            let status = resp.status().as_u16();
            let status_text = resp.status().canonical_reason().unwrap_or("").to_string();
            let final_url = resp.url().to_string();

            let mut resp_headers = HashMap::new();
            for (k, v) in resp.headers() {
                if let Ok(val_str) = v.to_str() {
                    resp_headers.insert(k.as_str().to_ascii_lowercase(), val_str.to_string());
                }
            }

            let bytes = resp.bytes().await.map_err(|e| format!("读取响应体失败: {e}"))?;
            if bytes.len() > MAX_BODY_BYTES {
                return Err(format!("响应体超出 {}MB 限制", MAX_BODY_BYTES / (1024 * 1024)));
            }

            Ok::<_, String>((status, status_text, final_url, resp_headers, bytes.to_vec()))
        };

        tokio::select! {
            res = fetch_fut => res,
            _ = tokio::time::sleep(effective_timeout) => {
                Err(format!("请求超时（{} ms）", effective_timeout.as_millis()))
            }
            _ = async {
                if let Some(cancel) = cancel_flag {
                    while !cancel.load(Ordering::SeqCst) {
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                } else {
                    std::future::pending::<()>().await;
                }
            } => {
                Err("请求已被取消".to_string())
            }
        }
    });

    let (status, status_text, final_url, resp_headers, body) = match exec_res {
        Ok(data) => data,
        Err(err_msg) => {
            throw_js_error(scope, &format!("fetch 失败: {err_msg}"));
            return;
        }
    };

    // 构造 SyncResponse JS 对象
    let resp_obj = v8::Object::new(scope);

    // 属性: status
    if let (Some(k), Some(v)) = (
        v8::String::new(scope, "status"),
        Some(v8::Integer::new(scope, status as i32)),
    ) {
        resp_obj.set(scope, k.into(), v.into());
    }

    // 属性: statusText
    if let (Some(k), Some(v)) = (
        v8::String::new(scope, "statusText"),
        v8::String::new(scope, &status_text),
    ) {
        resp_obj.set(scope, k.into(), v.into());
    }

    // 属性: ok
    if let (Some(k), Some(v)) = (
        v8::String::new(scope, "ok"),
        Some(v8::Boolean::new(scope, (200..300).contains(&status))),
    ) {
        resp_obj.set(scope, k.into(), v.into());
    }

    // 属性: url
    if let (Some(k), Some(v)) = (
        v8::String::new(scope, "url"),
        v8::String::new(scope, &final_url),
    ) {
        resp_obj.set(scope, k.into(), v.into());
    }

    // 属性: headers (键值对象)
    let headers_obj = v8::Object::new(scope);
    for (hk, hv) in resp_headers {
        if let (Some(k), Some(v)) = (v8::String::new(scope, &hk), v8::String::new(scope, &hv)) {
            headers_obj.set(scope, k.into(), v.into());
        }
    }
    if let Some(k) = v8::String::new(scope, "headers") {
        resp_obj.set(scope, k.into(), headers_obj.into());
    }

    // 挂载响应体数据与方法
    let body_len = body.len();
    let ab = v8::ArrayBuffer::new(scope, body_len);
    if body_len > 0 {
        let store = ab.get_backing_store();
        if let Some(ptr) = store.data() {
            unsafe {
                std::ptr::copy_nonoverlapping(body.as_ptr(), ptr.as_ptr() as *mut u8, body_len);
            }
        }
    }

    // text() 方法
    let text_tmpl = v8::FunctionTemplate::builder(response_text_cb)
        .data(ab.into())
        .build(scope);
    if let (Some(fn_val), Some(name)) = (text_tmpl.get_function(scope), v8::String::new(scope, "text")) {
        resp_obj.set(scope, name.into(), fn_val.into());
    }

    // json() 方法
    let json_tmpl = v8::FunctionTemplate::builder(response_json_cb)
        .data(ab.into())
        .build(scope);
    if let (Some(fn_val), Some(name)) = (json_tmpl.get_function(scope), v8::String::new(scope, "json")) {
        resp_obj.set(scope, name.into(), fn_val.into());
    }

    // bytes() 方法 (返回 Uint8Array)
    let bytes_tmpl = v8::FunctionTemplate::builder(response_bytes_cb)
        .data(ab.into())
        .build(scope);
    if let (Some(fn_val), Some(name)) = (bytes_tmpl.get_function(scope), v8::String::new(scope, "bytes")) {
        resp_obj.set(scope, name.into(), fn_val.into());
    }

    rv.set(resp_obj.into());
}

fn response_text_cb(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue<v8::Value>,
) {
    let ab_val = args.data();
    if let Ok(ab) = v8::Local::<v8::ArrayBuffer>::try_from(ab_val) {
        let len = ab.byte_length();
        let store = ab.get_backing_store();
        if let Some(ptr) = store.data() {
            let slice = unsafe { std::slice::from_raw_parts(ptr.as_ptr() as *const u8, len) };
            let text = String::from_utf8_lossy(slice);
            if let Some(s) = v8::String::new(scope, &text) {
                rv.set(s.into());
                return;
            }
        }
    }
    if let Some(empty) = v8::String::new(scope, "") {
        rv.set(empty.into());
    }
}

fn response_json_cb(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue<v8::Value>,
) {
    let ab_val = args.data();
    if let Ok(ab) = v8::Local::<v8::ArrayBuffer>::try_from(ab_val) {
        let len = ab.byte_length();
        let store = ab.get_backing_store();
        if let Some(ptr) = store.data()
            && let Ok(text) = std::str::from_utf8(unsafe {
                std::slice::from_raw_parts(ptr.as_ptr() as *const u8, len)
            })
            && let Some(json_str) = v8::String::new(scope, text)
            && let Some(parsed) = v8::json::parse(scope, json_str)
        {
            rv.set(parsed);
            return;
        }
    }
    throw_js_error(scope, "JSON 解析失败: 响应体不是合法的 JSON");
}

fn response_bytes_cb(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue<v8::Value>,
) {
    let ab_val = args.data();
    if let Ok(ab) = v8::Local::<v8::ArrayBuffer>::try_from(ab_val) {
        let len = ab.byte_length();
        let u8arr = v8::Uint8Array::new(scope, ab, 0, len);
        if let Some(arr) = u8arr {
            rv.set(arr.into());
            return;
        }
    }
    throw_js_error(scope, "获取 bytes 失败");
}

fn throw_js_error(scope: &mut v8::PinScope, msg: &str) {
    let s = v8::String::new(scope, msg).unwrap_or_else(|| v8::String::empty(scope));
    let err = v8::Exception::error(scope, s);
    scope.throw_exception(err);
}
