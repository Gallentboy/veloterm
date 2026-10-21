/*
This file is part of jpegxl-sys.

jpegxl-sys is free software: you can redistribute it and/or modify
it under the terms of the GNU General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.

jpegxl-sys is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
GNU General Public License for more details.

You should have received a copy of the GNU General Public License
along with jpegxl-sys.  If not, see <https://www.gnu.org/licenses/>.
*/

//! JPEG XL 参考实现的 FFI 绑定 —— **veloterm vendor 修补版：运行期加载分支**。
//!
//! 上游原版在 build.rs 里用 pkg-config 做**链接期**动态链接：构建机没有
//! libjxl（或没有 pkg-config，如 Windows runner）时 build script 直接 panic；
//! 链接期依赖还意味着进程启动时必须能找到 jxl.dll/libjxl.dylib，用户机器
//! 缺库会拖死整个宿主程序。本分支改为 pdfium-render `bind_to_library` 同型
//! 的**运行期加载**：
//!
//! - 宿主先按自己的分发约定定位库文件，调用 [`bind_to_library`] 绑定（进程
//!   内一次，库句柄永不卸载）；
//! - 各模块的 FFI 声明块由 [`jxl_ffi!`](crate::jxl_ffi) 宏原地转换：块内每条
//!   `pub fn Name(args) -> Ret;` 声明**原文不动**，宏生成「函数指针字段 +
//!   同名同签名的包装函数」，上游调用点（jpegxl-rs）零改动；
//!   符号表按模块惰性解析——未用到的模块（encoder/threads 等）不触发解析，
//!   因此只带 jxl.dll（不带 jxl_threads.dll）也能正常解码；
//! - 绑定/解析失败只影响 JXL 功能本身，宿主进程其余部分无感。
//!
//! 补丁清单见本树 README.md「本地修补」。

use std::path::Path;
use std::sync::OnceLock;

#[cfg(windows)]
use libloading::os::windows as os;
#[cfg(not(windows))]
use libloading::os::unix as os;

pub mod decode;

pub mod color;
pub mod common;
pub mod encoder;
pub mod metadata;
pub mod threads;

/// 已绑定的 libjxl 库句柄（进程内一次；`Box::leak` 永不卸载——libjxl 的
/// LCMS 线程上下文等全局状态使 FreeLibrary/dlclose 都不安全）。
static LIBRARY: OnceLock<Result<&'static os::Library, String>> = OnceLock::new();

/// 按路径加载并绑定 libjxl（pdfium-render 的 `bind_to_library` 同型入口）。
///
/// 进程内幂等：首次调用生效；成功后再次调用（含换路径）直接返回成功，
/// 失败结果同样被缓存——宿主应以 [`Result`] 决定是否继续 JXL 功能。
///
/// # Errors
/// 库文件打不开（路径不存在/架构不符/依赖缺失等）时返回带路径的错误描述。
pub fn bind_to_library(path: &Path) -> Result<(), String> {
    LIBRARY
        .get_or_init(|| match unsafe { open_library(path) } {
            Ok(lib) => Ok(Box::leak(Box::new(lib))),
            Err(e) => Err(format!("加载 {} 失败：{e}", path.display())),
        })
        .as_ref()
        .map(|_| ())
        .map_err(Clone::clone)
}

/// 取已绑定的库句柄（供宏生成的符号解析使用）。
fn ffi_library() -> Result<&'static os::Library, String> {
    match LIBRARY.get() {
        Some(Ok(lib)) => Ok(lib),
        Some(Err(e)) => Err(e.clone()),
        None => Err("尚未绑定 libjxl（需先成功调用 bind_to_library）".to_string()),
    }
}

/// 打开动态库。
///
/// Windows 上必须用 `LOAD_WITH_ALTERED_SEARCH_PATH`：jxl.dll 依赖同目录的
/// hwy/brotli/lcms 等运行时 DLL，而普通 LoadLibrary 的依赖搜索序（exe 目录/
/// System32/PATH）不包含被加载 DLL 自身所在目录——DLL 放在子目录里会报
/// 「找不到依赖模块」。该旗标让依赖优先从 jxl.dll 自己的目录解析。
///
/// # Safety
/// `path` 必须指向合法的 libjxl 动态库：符号随后按声明逐一解析，错误的库
/// 会导致未定义行为。
#[cfg(windows)]
unsafe fn open_library(path: &Path) -> Result<os::Library, libloading::Error> {
    os::Library::load_with_flags(path, libloading::os::windows::LOAD_WITH_ALTERED_SEARCH_PATH)
}

#[cfg(not(windows))]
unsafe fn open_library(path: &Path) -> Result<os::Library, libloading::Error> {
    os::Library::new(path)
}

/// 解析单个符号并转成裸函数指针。
///
/// # Safety
/// `T` 必须与 libjxl 导出符号的实际签名一致；返回指针的生命周期与库句柄
/// 一致（进程内永不卸载，故终身有效）。
unsafe fn resolve_symbol<T>(lib: &os::Library, name: &str) -> Result<T, String> {
    let sym = lib
        .get::<T>(name.as_bytes())
        .map_err(|e| format!("解析符号 {name} 失败：{e}"))?;
    // Symbol<T> 是目标指针的 repr(transparent) 包装（libloading 文档认可
    // 转出裸指针的用法）；库句柄永不卸载，指针终身有效。用 transmute_copy
    // 是因为泛型目标 T 在定义处无法向 transmute 证明定长。
    Ok(std::mem::transmute_copy(&sym))
}

/// 把上游 `extern "C-unwind" { pub fn Name(args) -> Ret; ... }` 形态的声明块
/// 原地转换为运行期加载形态。调用侧的改动只有块头一行：
/// `extern "C-unwind" {` → `crate::jxl_ffi! {`，块内声明原文不动。
///
/// 为每条声明生成：
/// 1. 一个函数指针字段（进本模块的符号表 `JxlFfi`，惰性解析、模块级缓存）；
/// 2. 一个同名同签名的 `pub unsafe fn` 包装（供 jpegxl-rs 的调用点原样使用）。
#[macro_export]
macro_rules! jxl_ffi {
    // 入口臂：返回值可选（`-> ($($ret)?)` 归一化——有返回值时是括号包裹的
    // 类型、无返回值时是 unit 类型 `()`，两者都是合法的 ty）
    (
        $(
            $(#[$meta:meta])*
            pub fn $name:ident($($arg:ident: $ty:ty),* $(,)?) $(-> $ret:ty)?;
        )*
    ) => {
        $crate::jxl_ffi!(
            @impl
            $( $(#[$meta])* $name($($arg: $ty),*) -> ($($ret)?); )*
        );
    };
    // 实现臂（归一化：全部带显式返回值）
    (@impl $( $(#[$meta:meta])* $name:ident($($arg:ident: $ty:ty),*) -> $ret:ty; )*) => {
        /// 本模块的 FFI 符号表（首次包装函数调用时从已绑定的 libjxl 解析）。
        #[allow(unused_parens, non_snake_case)]
        pub(crate) struct JxlFfi {
            $(
                #[allow(dead_code)]
                $(#[$meta])*
                pub(crate) $name: extern "C-unwind" fn($($ty),*) -> $ret,
            )*
        }

        impl JxlFfi {
            fn resolve() -> Result<Self, String> {
                let lib = $crate::ffi_library()?;
                // SAFETY：符号类型与上游 extern 声明逐条一致
                unsafe {
                    Ok(Self {
                        $( $name: $crate::resolve_symbol(lib, stringify!($name))?, )*
                    })
                }
            }
        }

        static JXL_FFI: ::std::sync::OnceLock<Result<JxlFfi, String>> =
            ::std::sync::OnceLock::new();

        #[inline]
        fn ffi() -> &'static JxlFfi {
            match JXL_FFI.get_or_init(JxlFfi::resolve) {
                Ok(fns) => fns,
                Err(e) => panic!("jpegxl-sys: FFI 未就绪（{e}），需先成功调用 bind_to_library"),
            }
        }

        $(
            $(#[$meta])*
            #[inline]
            #[allow(unused_parens, non_snake_case)]
            // 包装保持 extern "C-unwind" ABI：上游既有直接调用（unsafe 块内），
            // 也把部分函数（如 JxlThreadParallelRunner）当函数指针值传参——
            // Rust ABI 的 fn item 无法退化成 extern fn 指针，ABI 必须一致。
            pub unsafe extern "C-unwind" fn $name($($arg: $ty),*) -> $ret {
                (ffi().$name)($($arg),*)
            }
        )*
    };
}
