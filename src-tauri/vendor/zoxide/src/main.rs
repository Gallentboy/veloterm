// VeloTerm vendor 修改：本文件原为完整 main()（模块声明 + clap 解析 + 错误
// 处理），已整体迁至 lib.rs（可注入参数入口 `zoxide::zoxide_main_from`），
// main.rs 保留为独立 zoxide 二进制的薄壳入口，见 lib.rs 头部说明。
fn main() {
    std::process::exit(zoxide::zoxide_main());
}
