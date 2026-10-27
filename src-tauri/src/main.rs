// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
// 同 lib.rs 顶部说明：macOS 上 V8 与静态 pdfium 的 Abseil 重复符号由
// 新版链接器容忍（警告不报错），压掉链接器消息噪音
#![cfg_attr(target_os = "macos", allow(linker_messages))]

fn main() {
    app_lib::run();
}
