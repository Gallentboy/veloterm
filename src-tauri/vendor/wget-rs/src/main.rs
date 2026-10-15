/*!
The main entry point into wget-rs.

VeloTerm vendor 修改：实际主逻辑已移至 `src/lib.rs`（暴露为
`wget_rs::wget_main()`）；本文件保留为独立 `wget-rs` 二进制的薄壳入口。
*/

fn main() {
    std::process::exit(wget_rs::wget_main());
}
