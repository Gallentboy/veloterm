//! libgif2mp4 的 Rust 绑定：C ABI 直decl + 进程级安全包装。
//!
//! 库本体见 `vendor/ffmpeg/libgif2mp4/`（ffmpeg 树内库，gif→mp4 转码，
//! GPL 许可警示见其 README）；链接经本 crate 的 build.rs 静态接入，
//! 「库预编译好提供」——本 crate 不编译任何 C 代码。
//!
//! 安全层语义（对应 C 头文件的约定）：
//! - 转码阻塞执行（内部自带进程级单飞，并发第二次调用返回 Busy）；
//! - 日志经 [`install_log_bridge`] 一次性安装到 `log` crate（级别映射
//!   ERROR/WARN/INFO/VERBOSE/DEBUG → error!/warn!/info!/debug!/trace!）；
//!   未安装时 C 侧回落 ffmpeg 默认回调（stderr），不崩溃；
//! - 进度回调返回 `false` 即取消（映射 C 侧返回非 0）；
//! - 任何失败返回 [`Error`]，携带 C 侧错误码与 `gif2mp4_last_error`
//!   文本（UTF-8 有损转换，C 侧恒为 UTF-8）。

use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::sync::OnceLock;

/// 与 C 侧 gif2mp4.h 的错误码一一对应
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum Code {
    Ok = 0,
    Arg = -1,
    Input = -2,
    Decode = -3,
    Encode = -4,
    Output = -5,
    Cancelled = -6,
    Busy = -7,
    Internal = -8,
    Unknown = i32::MIN,
}

impl Code {
    fn from_raw(raw: c_int) -> Self {
        match raw {
            0 => Code::Ok,
            -1 => Code::Arg,
            -2 => Code::Input,
            -3 => Code::Decode,
            -4 => Code::Encode,
            -5 => Code::Output,
            -6 => Code::Cancelled,
            -7 => Code::Busy,
            -8 => Code::Internal,
            _ => Code::Unknown,
        }
    }
}

/// 与 C 侧 gif2mp4_progress 同构（serde 化由 app 侧做，本层保持纯数据）
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct Progress {
    pub decoded_frames: i64,
    pub total_frames: i64,
    /// 0-100；total 未知时为 -1
    pub percent: f64,
    pub out_time_sec: f64,
    /// 未知时为 -1
    pub duration_sec: f64,
}

/// 与 C 侧 gif2mp4_options 同构（struct_size ABI 校验用）
#[repr(C)]
struct OptionsRaw {
    struct_size: u32,
    fps: c_int,
    crf: c_int,
    preset: *const c_char,
    loop_: c_int,
    on_progress: Option<extern "C" fn(*mut c_void, *const Progress) -> c_int>,
    progress_userdata: *mut c_void,
}

extern "C" {
    fn gif2mp4_run(opt: *const OptionsRaw, input_path: *const c_char, output_path: *const c_char) -> c_int;
    fn gif2mp4_set_log_callback(
        cb: Option<extern "C" fn(*mut c_void, c_int, *const c_char)>,
        userdata: *mut c_void,
        min_level: c_int,
    );
    fn gif2mp4_last_error() -> *const c_char;
    fn gif2mp4_abi_version() -> c_int;
}

/// ABI 版本探测（宿主编译期常量与库运行期比对；当前恒为 1）
pub fn abi_version() -> i32 {
    unsafe { gif2mp4_abi_version() }
}

#[derive(Debug, Clone)]
pub struct Error {
    pub code: Code,
    pub message: String,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "gif2mp4 {:?}: {}", self.code, self.message)
    }
}

impl std::error::Error for Error {}

fn last_error_text() -> String {
    unsafe {
        let ptr = gif2mp4_last_error();
        if ptr.is_null() {
            String::new()
        } else {
            CStr::from_ptr(ptr).to_string_lossy().into_owned()
        }
    }
}

fn make_error(raw: c_int) -> Error {
    Error {
        code: Code::from_raw(raw),
        message: {
            let text = last_error_text();
            if text.is_empty() {
                format!("code {raw}")
            } else {
                text
            }
        },
    }
}

// ─── 日志桥：C 回调 → log crate（一次安装，进程级） ───────────────

extern "C" fn log_trampoline(_userdata: *mut c_void, level: c_int, message: *const c_char) {
    if message.is_null() {
        return;
    }
    let text = unsafe { CStr::from_ptr(message) }.to_string_lossy();
    match level {
        0 => log::error!("{text}"),
        1 => log::warn!("{text}"),
        2 => log::info!("{text}"),
        3 => log::debug!("{text}"),
        _ => log::trace!("{text}"),
    }
}

static LOG_BRIDGE: OnceLock<()> = OnceLock::new();

/// 安装 C→Rust 日志桥（幂等；min_level 传 DEBUG 收全部）。
/// 未安装时 C 侧回落 ffmpeg 默认回调写 stderr，功能不受影响。
pub fn install_log_bridge() {
    LOG_BRIDGE.get_or_init(|| unsafe {
        // min_level=DEBUG(4)：C 侧全转发，过滤交给 log crate 的级别配置
        gif2mp4_set_log_callback(Some(log_trampoline), std::ptr::null_mut(), 4);
    });
}

// ─── 进度蹦床：把 C 回调折进闭包（Box 挂在 userdata 上） ──────────

type ProgressFn<'a> = dyn FnMut(&Progress) -> bool + 'a;

extern "C" fn progress_trampoline(userdata: *mut c_void, progress: *const Progress) -> c_int {
    if userdata.is_null() || progress.is_null() {
        return 0;
    }
    let callback = unsafe { &mut *(userdata as *mut Box<ProgressFn<'_>>) };
    if callback(unsafe { &*progress }) {
        0
    } else {
        1 // 请求取消
    }
}

/// 转码参数（preset 为 None 时 C 侧取 medium；fps ≤ 0 跟随源时基；
/// crf 为 None 时 C 侧取 23）
#[derive(Debug, Clone, Default)]
pub struct ConvertParams {
    pub fps: Option<i32>,
    pub crf: Option<i32>,
    pub preset: Option<String>,
}

/// 阻塞执行 GIF→MP4 转码。`on_progress` 返回 false 取消（得到
/// `Code::Cancelled`）。路径按 UTF-8 传 C（两侧约定一致）。
pub fn convert<P, F>(input_path: P, output_path: P, params: &ConvertParams, mut on_progress: F) -> Result<(), Error>
where
    P: AsRef<str>,
    F: FnMut(&Progress) -> bool,
{
    let input = CString::new(input_path.as_ref()).map_err(|_| Error {
        code: Code::Arg,
        message: "input path contains NUL".into(),
    })?;
    let output = CString::new(output_path.as_ref()).map_err(|_| Error {
        code: Code::Arg,
        message: "output path contains NUL".into(),
    })?;
    let preset = match &params.preset {
        Some(p) => Some(CString::new(p.as_str()).map_err(|_| Error {
            code: Code::Arg,
            message: format!("preset contains NUL: {p}"),
        })?),
        None => None,
    };

    let mut boxed: Box<Box<ProgressFn<'_>>> = Box::new(Box::new(&mut on_progress));
    let raw = OptionsRaw {
        struct_size: std::mem::size_of::<OptionsRaw>() as u32,
        fps: params.fps.unwrap_or(0),
        crf: params.crf.unwrap_or(-1),
        preset: preset.as_ref().map_or(std::ptr::null(), |p| p.as_ptr()),
        loop_: 0,
        on_progress: Some(progress_trampoline),
        progress_userdata: &mut *boxed as *mut Box<ProgressFn<'_>> as *mut c_void,
    };

    let code = unsafe {
        gif2mp4_run(&raw, input.as_ptr(), output.as_ptr())
    };
    // boxed 在回调期间被 C 侧借用，此处收回所有权释放
    drop(boxed);
    if code == 0 {
        Ok(())
    } else {
        Err(make_error(code))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abi_version_is_one() {
        assert_eq!(abi_version(), 1);
    }

    #[test]
    fn code_mapping_roundtrip() {
        assert_eq!(Code::from_raw(-6), Code::Cancelled);
        assert_eq!(Code::from_raw(-7), Code::Busy);
        assert_eq!(Code::from_raw(-99), Code::Unknown);
    }

    /// ABI 布局断言：OptionsRaw 的 sizeof 必须与 C 侧 gif2mp4_options 一致
    /// （C 侧在 gif2mp4_run 入口用 `struct_size != sizeof(gif2mp4_options)`
    /// 做前向兼容校验——两边算出的值不等会直接返回 E_ARG）。
    /// 布局：u32 + int + int + ptr + int + fnptr + ptr，64 位下 48 字节。
    #[test]
    fn options_layout_matches_c() {
        assert_eq!(std::mem::size_of::<OptionsRaw>(), 48, "OptionsRaw 大小与 C 侧 gif2mp4_options 不符");
        assert_eq!(std::mem::align_of::<OptionsRaw>(), 8);
        // 关键字段偏移（与 C 侧逐字段核对，防 #[repr(C)] 之外的意外重排）
        assert_eq!(std::mem::offset_of!(OptionsRaw, struct_size), 0);
        assert_eq!(std::mem::offset_of!(OptionsRaw, fps), 4);
        assert_eq!(std::mem::offset_of!(OptionsRaw, crf), 8);
        assert_eq!(std::mem::offset_of!(OptionsRaw, preset), 16);
        assert_eq!(std::mem::offset_of!(OptionsRaw, loop_), 24);
        assert_eq!(std::mem::offset_of!(OptionsRaw, on_progress), 32);
        assert_eq!(std::mem::offset_of!(OptionsRaw, progress_userdata), 40);
    }

    /// 进度结构布局：i64 + i64 + f64 ×3 = 40 字节（C 侧 gif2mp4_progress 同构）
    #[test]
    fn progress_layout_matches_c() {
        assert_eq!(std::mem::size_of::<Progress>(), 40);
        assert_eq!(std::mem::align_of::<Progress>(), 8);
        assert_eq!(std::mem::offset_of!(Progress, decoded_frames), 0);
        assert_eq!(std::mem::offset_of!(Progress, total_frames), 8);
        assert_eq!(std::mem::offset_of!(Progress, percent), 16);
        assert_eq!(std::mem::offset_of!(Progress, out_time_sec), 24);
        assert_eq!(std::mem::offset_of!(Progress, duration_sec), 32);
    }
}
