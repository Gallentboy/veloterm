# vendor/rusty_v8

本项目内置 JS 脚本运行时（`src/js_runtime.rs`）所需的 V8 静态库预编译档案与 Rust 绑定生成文件。

## 1. 来源与版本

- **上游仓库**：[denoland/rusty_v8](https://github.com/denoland/rusty_v8)
- **对应 crate**：`v8 = "=152.2.0"`（crates.io 锁死精确版本，与本目录档案严格配对）
- **发布 Release**：[v152.2.0](https://github.com/denoland/rusty_v8/releases/tag/v152.2.0)（2026-08-20）
- **许可证**：**MIT**（Deno 官方，无 GPL 系商用确认负担）

## 2. 零下载接线机制

`src-tauri/.cargo/config.toml` 配置了：

```toml
[env]
RUSTY_V8_ARCHIVE = { value = "vendor/rusty_v8", relative = true }
```

依据 rusty_v8 的 `build.rs` 语义：
- 当 `RUSTY_V8_ARCHIVE` 指向**本地目录**时，该目录同时是静态库（`.a.gz` / `.lib.gz`）和绑定文件（`src_binding_*.rs`）的**唯一权威来源**；
- build.rs 会先检查该目录，解压并直接使用，**绝不回落上游网络**；
- `[env]` 作用于 build script，`relative = true` 相对 `.cargo` 父目录（即 `src-tauri/`）解析为绝对路径，开发机与 CI runner 行为完全恒定。

## 3. 四平台资产清单与 SHA-256

覆盖应用发布的四条腿矩阵（Windows x64 / Windows ARM64 / macOS Intel / macOS Apple Silicon）：

| 平台 Target | 静态库档案（.gz） | SHA-256 |
| --- | --- | --- |
| `aarch64-apple-darwin` | `librusty_v8_release_aarch64-apple-darwin.a.gz` | `a4a5484444ad4b5f10dc7bed141236dd9dbeabf2006d14520f19b935d4db4227` |
| `x86_64-apple-darwin` | `librusty_v8_release_x86_64-apple-darwin.a.gz` | `04cbc1c36c4cd5459c54049775d92889337c69d936fb0b011f9b26f1d454610d` |
| `x86_64-pc-windows-msvc` | `rusty_v8_release_x86_64-pc-windows-msvc.lib.gz` | `0b17ca072bae37dd4ff00e6014d2b413becb031c9342ee11cb8226a5881f62b2` |
| `aarch64-pc-windows-msvc` | `rusty_v8_release_aarch64-pc-windows-msvc.lib.gz` | `cad105f510f59dd38a9cbcc8f6f78263d9bb8f009d0c111f972fbd529de92273` |

配套的绑定文件（纯 Rust 生成代码）：

| 平台 Target | 绑定文件 | SHA-256 |
| --- | --- | --- |
| `aarch64-apple-darwin` | `src_binding_release_aarch64-apple-darwin.rs` | `b9d5652ebca515eb2bf4c2ee4a6551b2567117b5aa720f430d2032616dd7e3b5` |
| `x86_64-apple-darwin` | `src_binding_release_x86_64-apple-darwin.rs` | `b9d5652ebca515eb2bf4c2ee4a6551b2567117b5aa720f430d2032616dd7e3b5` |
| `x86_64-pc-windows-msvc` | `src_binding_release_x86_64-pc-windows-msvc.rs` | `4fbdc0fbb831ff7b26962f0273d77a66fbc3bbb596fbe9e9022c0128f8c88099` |
| `aarch64-pc-windows-msvc` | `src_binding_release_aarch64-pc-windows-msvc.rs` | `4fbdc0fbb831ff7b26962f0273d77a66fbc3bbb596fbe9e9022c0128f8c88099` |

## 4. 升级流程

当需要升级 V8 版本时（与 `oracledb` crate 升级同款严谨口径）：

1. 在 GitHub Releases 查阅目标 tag，下载上述 4 平台 `.a.gz` / `.lib.gz` 以及 4 个 `src_binding_*.rs`（共 8 个文件）替换本目录内容；
2. 运行 `shasum -a 256 *` 重新计算并更新本文档的 SHA-256 表；
3. 修改 `src-tauri/Cargo.toml` 中的 `v8 = "=<新版本>"` 精确锁死；
4. 跑本地单测：`cd src-tauri && cargo test -p app --lib js_runtime`；
5. 跑 clippy 检查：`cargo clippy -p app --all-targets`；
6. 触发跨平台验证（如 `publish` 四腿构建或独立 tag 车道）。
