//! JS 脚本运行时（V8）——同步系统信息与资源监控桥（sys）。
//!
//! 提供全局 `sys` 对象：
//! - `version`: V8 引擎版本（例如 "13.2.152.20"）
//! - `system`: 操作系统名称及版本（例如 "macOS 14.6.1"、"Windows 10.0.22631"）
//! - `arch`: CPU 架构（"x64" / "arm64" 等）
//! - `platform`: 底层平台标识（"darwin" / "win32" / "linux"）
//! - `memory()`: 实时获取当前 V8 isolate 堆内存统计 `{ used: number, total: number, limit: number }`

use std::sync::OnceLock;

pub struct SysStaticInfo {
    pub version: String,
    pub system: String,
    pub arch: &'static str,
    pub platform: &'static str,
}

static SYS_INFO: OnceLock<SysStaticInfo> = OnceLock::new();

#[cfg(target_os = "macos")]
fn detect_system() -> String {
    let out = std::process::Command::new("sw_vers")
        .arg("-productVersion")
        .output();
    if let Ok(o) = out
        && let Ok(ver) = String::from_utf8(o.stdout)
    {
        let ver = ver.trim();
        if !ver.is_empty() {
            return format!("macOS {ver}");
        }
    }
    "macOS".to_string()
}

#[cfg(target_os = "windows")]
fn detect_system() -> String {
    let out = std::process::Command::new("cmd")
        .args(["/c", "ver"])
        .output();
    if let Ok(o) = out
        && let Ok(ver) = String::from_utf8(o.stdout)
    {
        let ver = ver.trim();
        if let Some(start) = ver.find("[Version ")
            && let Some(sub) = ver.get(start + 9..)
            && let Some(end) = sub.find(']')
            && let Some(num) = sub.get(..end)
        {
            return format!("Windows {num}");
        }
    }
    "Windows".to_string()
}

#[cfg(target_os = "linux")]
fn detect_system() -> String {
    if let Ok(content) = std::fs::read_to_string("/etc/os-release") {
        for line in content.lines() {
            if let Some(rest) = line.strip_prefix("PRETTY_NAME=") {
                let name = rest.trim_matches('"').trim();
                if !name.is_empty() {
                    return name.to_string();
                }
            }
        }
    }
    "Linux".to_string()
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn detect_system() -> String {
    std::env::consts::OS.to_string()
}

fn detect_platform() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "darwin"
    }
    #[cfg(target_os = "windows")]
    {
        "win32"
    }
    #[cfg(target_os = "linux")]
    {
        "linux"
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        std::env::consts::OS
    }
}

fn detect_arch() -> &'static str {
    match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        other => other,
    }
}

pub fn sys_info() -> &'static SysStaticInfo {
    SYS_INFO.get_or_init(|| SysStaticInfo {
        version: v8::V8::get_version().to_string(),
        system: detect_system(),
        arch: detect_arch(),
        platform: detect_platform(),
    })
}

/// sys.memory(): { used: number, total: number, limit: number }
pub fn sys_memory_cb(
    scope: &mut v8::PinScope,
    _args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue<v8::Value>,
) {
    let stats = scope.get_heap_statistics();

    let obj = v8::Object::new(scope);
    if let Some(k) = v8::String::new(scope, "used") {
        let v = v8::Number::new(scope, stats.used_heap_size() as f64);
        obj.set(scope, k.into(), v.into());
    }
    if let Some(k) = v8::String::new(scope, "total") {
        let v = v8::Number::new(scope, stats.total_heap_size() as f64);
        obj.set(scope, k.into(), v.into());
    }
    if let Some(k) = v8::String::new(scope, "limit") {
        let v = v8::Number::new(scope, stats.heap_size_limit() as f64);
        obj.set(scope, k.into(), v.into());
    }
    rv.set(obj.into());
}

/// 将 `sys` 对象挂载到 V8 全局对象上
pub fn inject_sys_object(
    scope: &mut v8::PinScope,
    global: &v8::Local<v8::Object>,
) {
    let info = sys_info();
    let sys_obj = v8::Object::new(scope);

    if let (Some(k), Some(v)) = (v8::String::new(scope, "version"), v8::String::new(scope, &info.version)) {
        sys_obj.set(scope, k.into(), v.into());
    }
    if let (Some(k), Some(v)) = (v8::String::new(scope, "system"), v8::String::new(scope, &info.system)) {
        sys_obj.set(scope, k.into(), v.into());
    }
    if let (Some(k), Some(v)) = (v8::String::new(scope, "arch"), v8::String::new(scope, info.arch)) {
        sys_obj.set(scope, k.into(), v.into());
    }
    if let (Some(k), Some(v)) = (v8::String::new(scope, "platform"), v8::String::new(scope, info.platform)) {
        sys_obj.set(scope, k.into(), v.into());
    }

    if let (Some(k), Some(f)) = (
        v8::String::new(scope, "memory"),
        v8::Function::new(scope, sys_memory_cb),
    ) {
        sys_obj.set(scope, k.into(), f.into());
    }

    if let Some(sys_key) = v8::String::new(scope, "sys") {
        let _ = global.set(scope, sys_key.into(), sys_obj.into());
    }
}
