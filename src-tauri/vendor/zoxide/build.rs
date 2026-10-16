// VeloTerm vendor 修改：上游 build.rs 用 clap_complete 在**包目录**（而非
// OUT_DIR）生成 contrib/completions/ 下的 shell 补全脚本——被 coreutils 以
// 路径依赖构建时会在 vendor 源码树 / CI 的只读产物缓存里写文件，且内嵌多
// 呼叫形态用不到补全脚本，故整段生成逻辑移除（[build-dependencies] 已一并
// 从 Cargo.toml 删除）。升级 zoxide 时如恢复上游 build.rs，需复核此约束。
fn main() {}
