// VeloTerm vendor 修改：本文件原为完整 main()（模块声明 + getopts 解析 +
// process::exit 错误处理），已整体迁至 lib.rs（可注入参数入口
// `dog::dog_main_from`），main.rs 保留为独立 dog 二进制的薄壳入口，
// 见 lib.rs 头部说明。
fn main() {
    std::process::exit(dog::dog_main());
}
