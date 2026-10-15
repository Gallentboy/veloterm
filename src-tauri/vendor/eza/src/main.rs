// SPDX-FileCopyrightText: 2024 Christina Sørensen
// SPDX-License-Identifier: EUPL-1.2
//
// SPDX-FileCopyrightText: 2023-2024 Christina Sørensen, eza contributors
// SPDX-FileCopyrightText: 2014 Benjamin Sago
// SPDX-License-Identifier: MIT
#![warn(future_incompatible)]
#![warn(trivial_casts, trivial_numeric_casts)]
#![warn(clippy::all)]
#![allow(clippy::non_ascii_literal)]

/*!
The main entry point into eza.

VeloTerm vendor 修改：原 main.rs 的主逻辑已 librarify 到 `src/lib.rs`
（`eza::eza_main()` / `eza::eza_main_from()`），本文件保留为独立 eza
二进制的薄壳入口。SIGPIPE 信号处理留在本薄壳中（独立进程语义），
不进入 lib，以免影响嵌入 coreutils 的宿主进程。
*/

fn main() {
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }

    std::process::exit(eza::eza_main());
}
