//! coreutils 多呼叫二进制的链接期设置。
//!
//! Windows/MSVC：把主线程栈保留空间扩到 8MB。jq 官方 Windows 构建显式传
//! `-Wl,--stack,8388608`（见 jqlang/jq 的 CI），因为 jq 编译器（builtin.jq
//! 编译、block_bind 递归）栈消耗大——我们默认 1MB 栈在深嵌套 jq 程序下有
//! 栈溢出风险。仅改 reserve（虚拟地址空间），commit 仍按需增长，无实内存
//! 代价。仅 MSVC 链接器认 /STACK，非 Windows 目标不加。

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
    {
        println!("cargo:rustc-link-arg-bins=/STACK:8388608");
    }
    println!("cargo:rerun-if-changed=build.rs");
}
