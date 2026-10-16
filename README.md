# VeloTerm

VeloTerm 是快速、精致的跨平台桌面终端（Tauri 2 + React 19），面向远程运维场景：SSH 终端、SFTP 双栏文件管理、远端文件编辑器、常用命令库与多终端协同广播。Windows 版额外内置完整的 Unix 工具链（coreutils/grep/sed/find/diff/ripgrep/jq/yq 等 100+ 命令），安装后开箱即用，不依赖 Git Bash / MSYS2。

## 功能概览

| 模块 | 能力 |
| --- | --- |
| 终端 | 本地终端（继承系统 Shell Profile）与 SSH 远程终端；标签页 / 九宫格平铺两种布局；⌘T 新建、⌘R 重连断开会话；协同广播（一输入多终端执行、Ctrl+C 中断广播） |
| 文件管理 | SFTP 双栏对照，左右拖拽互拷（跨主机直连不落盘）；上传 / 下载 / 断点续传 / 限速；文件右键「尝试编辑」（内容嗅探，非白名单文本也能打开） |
| 编辑器 | Monaco 内嵌（81 种语言高亮、TS/JS/JSON 智能提示全离线）；远端文件在线编辑、冲突检测（本地 vs 远端 baseHash）、草稿防抖暂存；SQLCipher 加密保存 |
| 主机库 | 分组树 + 跳板机链 + 三种认证（密码 / 私钥 / Agent）；「终端就绪钩子」——连接建立后自动执行命令库中的指定命令 |
| 命令库 | 常用命令收藏，支持 `{{参数}}` 占位符模板、危险级别、Markdown 详细说明；全局命令面板快捷执行 |
| 内置工具链（仅 Windows） | 单一 `coreutils.exe` 多呼叫二进制 + NTFS 硬链接短名入口（`ls` `grep` `rg` `awk` `bat` `bc` `jq` `yq` `qalc` `nmap` `zoxide` `fzf` `dog` `onefetch` … 128 个命令），BusyBox 模式零拷贝分发 |

## 环境要求

- Node.js ≥ 22、pnpm ≥ 11、Rust stable（edition 2024，rustc ≥ 1.90）
- macOS 开发：直接按下方命令运行；Windows 交叉编译**严禁**在 macOS 上进行（见 AGENTS.md 平台约束）

## 开发与运行

```bash
pnpm install        # 安装前端依赖
pnpm veloterm:dev   # Tauri 开发模式（前端热更 + Rust 增量编译）
```

`veloterm:dev` 内部执行 `tauri dev`，只构建主程序 `app`（内置工具链是独立 workspace member，不参与 dev 构建）。默认窗口标题 VeloTerm，开发服务器端口 3000。

常用命令（均在仓库根目录执行）：

| 命令 | 用途 |
| --- | --- |
| `pnpm veloterm:dev` | Tauri 开发模式（macOS 日常开发入口） |
| `pnpm veloterm:build` | 出本机平台安装包；非 Windows 目标自动跳过 coreutils 构建 |
| `pnpm veloterm:build:mac-arm` | 出 macOS arm64 app 包 |
| `pnpm run dev` / `build` / `preview` | 仅前端（Rsbuild）开发 / 构建 / 预览 |
| `pnpm run typecheck` | TypeScript 全量类型检查（提交前必过） |
| `pnpm tools:build` | 手动触发 coreutils 多呼叫二进制构建（仅 Windows 目标生效） |

### Rust 侧（src-tauri）

`src-tauri` 是 Cargo workspace 根，包含三个成员：

| 成员 | 职责 |
| --- | --- |
| 根包 `app` | Tauri 主程序：终端 PTY/SSH、SFTP、SQLCipher 存储与全部 IPC 命令。**刻意不依赖任何工具 crate**，保持轻量 |
| `crates/tools-catalog` | 零依赖的命令名清单（单一事实来源），app 与 coreutils 双侧共用 |
| `crates/coreutils` | Windows 内置工具链多呼叫二进制（BusyBox 模式），全部工具依赖只进这里 |

```bash
cargo test --workspace        # 全部单测（app + coreutils + tools-catalog）
cargo test -p coreutils       # 工具链单测（argv0 分发 / tar / ps / top 等）
cargo clippy --workspace --all-targets
```

注意：`cargo build` / `cargo test` 不带 `-p` 时默认只作用于根包 `app`；工具链需显式 `-p coreutils`。

### 内置工具链（仅 Windows）

工具链在 Windows 安装包中随主程序分发（`binaries/coreutils-<triple>.exe`），应用启动时在主程序同级 `tools/` 目录创建 128 个硬链接短名入口并注入 PATH——用户在 cmd 中直接敲 `ls`、`grep`、`rg` 即用。macOS 上不构建、不分发（系统自带完整工具链）。

本地 cmd 会话启动时还会注入 doskey 宏，把交互输入的 `cd`/`chdir` 自动补上 `/d` 开关——跨盘符直达（`cd D:\proj` 一步到位，无需手写 `cd /d`），批处理脚本与 `cmd /c` 不受影响。

工具实现来源（`src-tauri/vendor/`，全部源码锁定随仓库分发）：

| 来源 | 提供的命令 |
| --- | --- |
| uutils/coreutils 0.12.0 | GNU coreutils 全集（90 命令） |
| uutils/grep · sed · findutils · diffutils | grep、sed、find/xargs、diff/cmp/sdiff |
| ripgrep · fd-find · chmln/sd · frawk | rg、fd、sd、awk（Cranelift JIT） |
| sharkdp/bat | bat（0.26.1：语法高亮 cat，vendor 源码纯 Rust 静态链入，fancy-regex 后端，OWN_ARGV 短名 argv[0] 分发） |
| rust-parallel | parallel（v1.9.1，tokio 并行执行器，GNU parallel 风格接口；短名 argv[0] 分发） |
| onefetch | onefetch（v3.0.0，git 仓库信息面板：语言统计（tokei）/依赖/作者/churn/许可证识别，gix 纯 Rust git 栈；短名 argv[0] 分发，vendor 树自带 ascii/image/manifest 三个 path 子 crate） |
| tar-rs · eza · wget-rs | tar（自研 GNU 兼容 CLI 壳）、eza/tree、wget |
| openssl-app + openssl-src | openssl CLI（vcpkg 3.6.4 静态库 + 锁版本源码，仅 Windows） |
| putty-telnet | telnet（PuTTY 源码内嵌 + C 层 GNU 参数解析，仅 Windows） |
| jq-app | jq（jqlang/jq 1.7.1 + 捆绑 oniguruma 6.9.8，vendor 源码 cc 现场编译 FFI 直调 umain，仅 Windows；vendor 树含本地修补清单，见 `src-tauri/vendor/jq/README.md`） |
| bc-app | bc（gavinhoward/bc 7.1.0，vendor 源码 cc 现场编译 FFI 直调改名 main，上游自带官方 MSVC 工程零修补，仅 Windows） |
| yq + fzf（Go） | 合并为单一 `veloterm_goffi.dll`（yq v4.54.1 + fzf 0.74.5：源码内 vendor + `vendor/go.work` 跨模块聚合、运行时按符号动态导入 FFI，仅 Windows；GOAMD64=v3 以 AVX2 为基线、单次 CLI 关闭常规 GC；Go 官方不支持 MSVC 链接 c-archive，DLL 是 extern 链接的唯一形态；fzf 是交互式模糊查找器，渲染直接落在 veloterm 终端上） |
| libqalculate（C++） | qalc（5.13.1：源码内 vendor + MSYS2 MinGW64 构建 + `qalcffi.dll` 动态导入 FFI，仅 Windows；上游无 MSVC 工程且官方 Windows 版即 MinGW 构建，依赖全静态打入、定义 XML 编进二进制） |
| nmap（C/C++） | nmap（7.992：源码内 vendor（裁掉 zenmap GUI）+ MSYS2 MinGW64 构建 + `nmapffi.dll` 动态导入 FFI，仅 Windows；Npcap SDK 经 bundled libpcap CMake 静态编入、Packet.dll 延迟导入——未装 Npcap 时降级 connect 扫描，装后全功能；NSE 数据文件树随包放安装根 `nmap-data/`；NPSL 许可义务见 `src-tauri/vendor/nmap/README.md`） |
| zoxide | zoxide（0.10.0：ajeetdsouza/zoxide 纯 Rust 源码 vendor 静态链入，lib.rs 可注入入口 eza 同型，仅 Windows 清单；数据库落在 `%LOCALAPPDATA%\zoxide`，上游无 cmd.exe 钩子，`zoxide init powershell` 等可用） |
| dog | dog（ogham/dog 0.2.0-pre @ master tip 721440b：命令行 DNS 客户端（dig 替代），纯 Rust 源码 vendor 静态链入（自带 dns / dns-transport 两个 path 子 crate），lib.rs 可注入入口 eza/zoxide 同型，仅 Windows 清单；TLS/DoT/DoH 走 native-tls（Windows schannel），系统默认名字服务器经 ipconfig crate 探测） |
| 自研 | which、ps、top（sysinfo + crossterm） |

版本锁定与升级流程见 `src-tauri/vendor/openssl-src/README.md`。

## 打包与发布

正式 Windows 安装包由 GitHub Actions 出（`.github/workflows/publish.yml`）：push 到 `release` 分支或手动 dispatch 触发，windows-latest runner 上构建（coreutils 二进制 → Tauri NSIS 安装包），产物为 draft release。vcpkg 依赖（OpenSSL 静态库）走 GitHub Packages NuGet 二进制缓存，首次冷编译后续秒级恢复。

主二进制名为 `VeloTerm.exe`（`mainBinaryName` 配置），任务管理器与 `ps` 可按 `VeloTerm` 检索。

macOS 本地打包：`pnpm veloterm:build`，产物在 `src-tauri/target/release/bundle/`。

## 仓库结构

| 目录 | 职责 |
| --- | --- |
| `src/` | React 前端（AppShell 页面框架 + 各功能模块） |
| `src/components/terminal/` | 终端屏、宫格容器、会话卡（xterm.js + WebGL）、广播栏 |
| `src/components/files/` | SFTP 双栏文件管理、传输队列 |
| `src/components/editor/` | Monaco 编辑器宿主、标签栏 |
| `src/components/hosts/` | 主机库、分组、主机 / 分组编辑对话框 |
| `src/components/commands/` | 命令库、命令面板、命令编辑 |
| `src/services/` | 前端服务层：SSH 生命周期、SFTP、传输队列、命令存储 |
| `src/i18n/locales/` | 中英文案（zh-CN / en-US，结构必须一致） |
| `src-tauri/` | Rust 后端（workspace 根） |
| `src-tauri/src/` | Tauri 主程序：terminal / sftp / commands / db（SQLCipher）等模块 |
| `src-tauri/crates/` | tools-catalog（命令清单）与 coreutils（工具链二进制） |
| `src-tauri/vendor/` | 全部工具链上游源码（锁定版本，见各目录 README） |
| `scripts/` | build-tools（coreutils 构建脚本）、sync-version（版本号同步） |
| `docs/` | Linux 命令行参考手册（按文件 / 文本 / 进程 / 网络 / 包管理分类） |

数据全部落在本机 SQLCipher 加密数据库 `veloterm.db`（应用数据目录），含主机凭据、命令库、编辑器草稿与面板状态。

## 协作规约

AI 协作与工程硬性规约（文件写入规范、验证门禁、平台约束、架构不变量）见 [AGENTS.md](AGENTS.md)。
