# AGENTS.md

## File Editing Rules（文件写入硬性规范）

- **严禁通过 Bash 写入文件**：包括但不限于 `python3 <<EOF`、`echo >`、`tee`、`sed -i`、shell 重定向等任何变体。
- 修改或创建文件**必须使用专用文件写入工具**（Write / Edit / MultiEdit），确保每次改动精确、可审计、可回滚。
- Bash 仅允许执行只读命令与构建/测试（如 git status、grep、cargo check、pnpm build）。唯一例外：vendor 源码树的整目录同步（下载上游 tarball 解压替换），属构建资产操作而非文件编辑。

## 核心原则

- 改动前先读目标模块的现状代码，以当前检出的源码为准；本文件描述的能力与实际不符时，以源码为准并同步更新本文件。
- 定位问题时，未明确要求修改代码就先调查原因，结合源码与运行证据区分「已确认」与「待验证」。
- 涉及多文件的功能改动，先明确接口契约（命令名、参数、prop、i18n key）再动手；两语言文案（zh-CN / en-US）必须同构同步。
- 验证报告真实结果：测试失败就写失败，不把已有失败写成通过。

## 验证门禁

提交前的最低验证口径（macOS 本机）：

| 检查 | 命令 | 要求 |
| --- | --- | --- |
| 前端类型 | `pnpm run typecheck` | 零错误 |
| 前端构建 | `pnpm run build` | 成功 |
| Rust 编译 | `cargo check --workspace --all-targets`（在 `src-tauri/`） | 零错误 |
| Rust 测试 | `cargo test --workspace` | 全绿（app 66 + coreutils 26 + tools-catalog 1 为当前基线） |
| Lint | `cargo clippy --workspace --all-targets` | 自有代码零警告 |

冒烟构建用 **debug**（`cargo build -p coreutils`），release 太慢。已知例外：在 src-tauri 根用 `-p openssl-app` 跑测试会触发 cargo 对 `cfg(windows)` 门控成员的 resolver panic（cargo 的 bug），改在 `vendor/openssl-app` 目录内独立跑（jq-app 同此约定）。

## 平台约束（硬性）

- **严禁在 macOS 上交叉编译 Windows 目标**。Windows 包只能由 GitHub Actions 的 windows-latest runner（或真实 Windows 机器）出。`scripts/build-tools.mjs` 已内置两道闸：非 Windows target 跳过、非 Windows 主机请求 Windows target 硬拒。
- 工具链（openssl-app / putty-telnet / jq-app 的 C 编译、vcpkg 静态库、yq 的 Go c-shared、libqalculate 的 MSYS2 MinGW64 autotools）只在 Windows 编译；macOS 走桩模块，保证本机测试通过即视为完成。jq-app 与前两者不同：纯 C 自包含（vendor/jq 1.7.1 + 捆绑 oniguruma 6.9.8），零 vcpkg/cmake/perl 依赖，CI 无额外安装步骤。yq 亦零额外依赖：Go 构建只发生在 Windows（build-tools.mjs），macOS/测试机不碰 Go 工具链与 DLL。qalc 需 CI 预装 MSYS2 MinGW64 依赖（pacman 装 gcc/make/pkg-config/libxml2/gmp/mpfr，windows-latest 预装 C:\msys2 只补包），macOS/测试机同样不碰 MSYS2 与 DLL。
- 所有 Windows 专属逻辑用 `#[cfg(target_os = "windows")]` / `[target.'cfg(windows)'.dependencies]` 门控，不留运行时判断。

## 架构不变量

### Cargo workspace 拆分

`src-tauri` 是 workspace 根（根包 = Tauri 主程序 `app`），三个成员职责不可混：

- **app 不依赖任何工具 crate**——uutils/ripgrep/openssl-app/putty-telnet/jq-app 的依赖图只进 `crates/coreutils`。这是「主程序瘦身 + 增量构建互不牵连」的前提：改 UI 只重链 app，改工具只重链 coreutils。
- **tools-catalog 保持零依赖**——命令名清单是 app（建硬链接）与 coreutils（argv0 分发白名单）的单一事实来源；任何依赖都会经它传染给主程序。
- `vendor/` 被 workspace `exclude`：vendor 内 coreutils/sed/ripgrep/sd 自带 workspace，其余为无继承键的独立清单；不 exclude 则 uucore 的 `edition.workspace` 会误朝根清单取值而报错。
- 裸 `cargo build` / `cargo test` 只作用于根包 app；工具链必须显式 `-p coreutils`。

### Windows 工具链（BusyBox 模式）

- 单一 `coreutils.exe` 多呼叫二进制；`rg`/`fd`/`awk` 三个命令的解析器直读自身 argv，**仅硬链接短名模式可用**，子命令模式给出提示。
- **yq（Go）与 qalc（libqalculate C++）是 DLL 例外形态**：Go 官方不支持 MSVC 链接 c-archive（golang/go#74189），libqalculate 则是 autotools 体系的 C++ 库（GMP/MPFR/libxml2 硬依赖，上游无 MSVC 工程、官方 Windows 版即 MinGW 构建），两者都无法像 openssl-app 那样在 MSVC 下静态链入——共同形态是：vendor 源码（`vendor/yq/`、`vendor/libqalculate/`，版本各锁在树内 `VELOTERM_VERSION`）由 `build-tools.mjs` 在 Windows 上产出 DLL（yq：`go build -buildmode=c-shared -tags veloterm_ffi`；qalc：MSYS2 MinGW64 环境 VPATH 构建 + `qalc.cc -Dmain=qalc_main` 改名编译后 `-static` 链自包含 `qalcffi.dll`，定义 XML 经 `--enable-compiled-definitions` 编进二进制、运行时零数据文件），coreutils 的 `yq_impl.rs` / `qalc_impl.rs` 运行时 LoadLibrary 调 C 导出（无链接期依赖，macOS/CI 测试机无需 Go/MSYS2 工具链）；DLL 随包走 resources 放安装根目录，硬链接短名（`tools\yq.exe`、`tools\qalc.exe`）启动时按 exe 同目录→上级目录搜索。升级：换 vendor 源码 + 改 `VELOTERM_VERSION`（qalc 升级还需按树内 README 复核两处 UTF-8 输出补丁）。qalc 的 MinGW 构建依赖由 CI 的 pacman 步骤安装（publish.yml），安装包内的 qalcffi.dll 依赖全静态打入、终端用户不需要 MSYS2。
- 硬链接安装在应用启动钩子（`src/tools/hardlinks.rs`，仅 Windows 编译）完成：幂等判据是**文件身份**（Unix inode 对；Windows 退化为大小 + mtime），不是文件大小——同体积升级曾因此误判。
- 依赖 uutils 家族统一走 vendor 的 uucore 0.12.0（各 vendor 清单已重定向），避免 lock 出现双版本。
- 新增命令：改 `tools-catalog` 清单 → `crates/coreutils/src/coreutils.rs` 加分发臂 → 补 command_table_consistency 与 version sweep 的预期（平台不对称的命令要条件化 skip，否则 macOS 侧计数对不上）。

### OpenSSL 锁版本

`vendor/openssl-src` 只含 apps + include/internal 最小子集。版本锁定是**双源手维护**（升级时两处必须同步改）：`VELOTERM_VERSION`（源码锁版本）+ `src-tauri/vcpkg.json` 的 overrides（CI 安装侧）——vcpkg 版本控制仅在 manifest 模式可用（classic 模式没有命令行版本语法），CI 以 manifest 精确安装锁定版本（当前 **3.6.4#1**，即 vcpkg 上游版本数据库可得的最高版）。曾踩的坑：classic 模式跟随 runner ports 快照装出过 versions 数据库里不存在的 3.6.5（上游从未收录），manifest 解析直接报 no version database entry——**锁版本必须以 vcpkg 版本数据库实有版本为准**，升级前先查 `vcpkg search openssl` 或 versions/o-/openssl.json；builtin-baseline 指向的 SHA 需先 `git fetch` 补齐旧镜像历史。apps 的 `cmp.c` 与 `lib/cmp_mock_srv.c` 是上游 build.info 里的成对项，必须一起排除。CI 的 manifest 安装用 `--x-install-root` 把产物固定装进 classic 目录（`<vcpkg根>\installed\<triplet>`）而非默认的 `<清单目录>\vcpkg_installed\`：OPENSSL_LIB_DIR 字符串必须与 rust-cache 恢复的旧产物里烘焙的链接搜索路径一致——libsqlite3-sys 的 bundled 路径未声明 `rerun-if-env-changed=OPENSSL_LIB_DIR`，改环境变量 cargo 也不重跑其 build script，只重放缓存的旧 `rustc-link-search`，路径失效即 LNK1104（CI 实测）。

### 前端约定

- 弹层（下拉/右键菜单）一律 portal 到 `document.body` + fixed 定位（共享 `useAnchoredPanel`：视口夹取、空间不足上翻）；关闭判定用 `useDismissOnOutsidePointer` 的多 ref 排除区（触发器与弹层不同子树）。tooltip 同一约定走共享 `useTooltipPlacement`（`side` 是期望侧，空间不足自动翻对侧、两侧都放不下视口夹取）——两套 tooltip 组件（`ControlHintTooltip` / `ui/tooltip`）都已接上，禁止再写内联 absolute 定位的提示层（会被 tab 栏/底部工具栏的 overflow 裁掉）。
- 表单字段容器 `FormField` 用 div 不用 label——WebKit 会把 label 内点击转发给第一个可标签化后代，曾致「取消新建命令后下拉自动展开」。
- 颜色/字号全部走语义 token（`bg-selected` / `border-hover` / `text-ui-*` 随 `--ui-font-size` 缩放）；选中态不用主色三件套（primary 边框+底色+文字），主色只留给活动指示。
- Monaco 经 `import * as monaco from 'monaco-editor'` 全量入口（81 语言高亮全离线）；worker 用 `new Worker(new URL(...), { type: 'module' })` 让 Rspack 产独立 chunk，`new URL` 必须内联在 `new Worker` 里。
- 终端内提示（断开/重连快捷键）直接写 xterm 缓冲，不叠 DOM 覆盖层；就绪钩子命令等首屏输出静止 500ms 再注入（早发会被内核回显 + readline 重绘各显示一次，看到两遍）。

### 存储

全部数据在 SQLCipher 加密库 `veloterm.db`：表结构变更走「建表批次后的幂等 ALTER（吞 duplicate column）」，新列在 HostRecord / HostNode / mapper / roundtrip 测试四处同步。主机「就绪钩子」只存命令库主键 id，命令删除后钩子静默失效。

## 命令速查

| 用途 | 命令 |
| --- | --- |
| 开发 | `pnpm veloterm:dev` |
| 类型检查 | `pnpm run typecheck` |
| 前端构建 | `pnpm run build` |
| Rust 测试 | `cd src-tauri && cargo test --workspace` |
| 工具链测试 | `cargo test -p coreutils` |
| 工具链冒烟 | `cargo build -p coreutils`（debug） |

## Docs

- Linux 命令参考：`docs/linux-*-reference.md`（内置工具链的行为基准）
- OpenSSL 锁版本流程：`src-tauri/vendor/openssl-src/README.md`
- Rsbuild: https://rsbuild.rs/llms.txt · Rspack: https://rspack.rs/llms.txt
