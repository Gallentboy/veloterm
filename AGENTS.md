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
| Rust 测试 | `cargo test --workspace` | 全绿（app 67 + coreutils 50 + tools-catalog 1 为当前基线） |
| Lint | `cargo clippy --workspace --all-targets` | 自有代码零警告 |

冒烟构建用 **debug**（`cargo build -p coreutils`），release 太慢。已知例外：在 src-tauri 根用 `-p openssl-app` 跑测试会触发 cargo 对 `cfg(windows)` 门控成员的 resolver panic（cargo 的 bug），改在 `vendor/openssl-app` 目录内独立跑（jq-app / bc-app 同此约定）。

## 平台约束（硬性）

- **严禁在 macOS 上交叉编译 Windows 目标**。Windows 包只能由 GitHub Actions 的 windows-latest runner（或真实 Windows 机器）出。`scripts/build-tools.mjs` 已内置两道闸：非 Windows target 跳过、非 Windows 主机请求 Windows target 硬拒。
- 工具链（openssl-app / putty-telnet / jq-app / bc-app 的 C 编译，vcpkg 静态库，yq 与 fzf 的 Go c-shared、libqalculate 的 MSYS2 MinGW64 autotools、nmap 三件套的 MSVC+CMake、vim 的官方 MSVC nmake VIMDLL）只在 Windows 编译；macOS 走桩模块，保证本机测试通过即视为完成。jq-app 与 openssl-app/putty-telnet 不同：纯 C 自包含（vendor/jq 1.8.2 + 捆绑 oniguruma 6.9.10），零 vcpkg/cmake/perl 依赖，CI 无额外安装步骤。vendor/jq 带十处本地修补（清单与依据见其树内 README，升级 jq 需逐条复核；1.7.1→1.8.2 升级时 jv_file.c 的 VLA 补丁上游已修、移除，新增 jv.c 文件头 pthread.h 门控与 builtin.c void* 算术两处，decNumber/oniguruma 随上游移到 vendor/jq/vendor/ 子目录），其中 main.c 删除 WIN32 的 `_O_U8TEXT` 流模式——MSVC/UCRT 下窄字符输出（fprintf/fwrite）写该模式流会被 CRT 静默吞，jq 的 stderr 报错与重定向输出全部丢失（上游官方构建是 MinGW 不触发；tty 正常输出走 WriteConsoleW 不受影响）；jv.c 的 WIN32 TSD shim（Heimdal 风格）删除「替换旧值时调用析构器」的非 POSIX 行为——jq 的退出清理是先 free 再 `pthread_setspecific(key, NULL)` 清槽，保留该调用即二次 free，MSVC 静态 CRT 下进程退出期以 0xC0000374（STATUS_HEAP_CORRUPTION）终止、WER 崩溃事后处理致每次 `jq -n 1+1` 多耗 ~450ms（`--version` 不建 dec/dtoa 上下文故不受影响；官方 MinGW 走真 pthread 分支不触发）。bc-app 更简：上游 bc 7.1.0 自带官方 MSVC 工程（vendor/bc/vs/bc.vcxproj，本地零修补），仅链 bcrypt，宿主生成器 strgen 由 build.rs 的 Rust 复刻替代（byte-diff 验证）。yq 亦零额外依赖：Go 构建只发生在 Windows（build-tools.mjs），macOS/测试机不碰 Go 工具链与 DLL。fzf（junegunn/fzf）同 yq 形态：零本地修补，仅树内 veloterm_ffi.go 复刻上游 main() 的分支逻辑（返回退出码不 os.Exit，调用期间临时替换 os.Args 让 --popup/winpty 再 exec 路径回到 tools\fzf.exe）；依赖经 go modules 拉取，c-shared 胶水需要 cgo/gcc（同 yq 的 MinGW 探测，CI 零额外安装步骤）；交互式 TUI 在宿主 coreutils.exe 进程内直跑、stdio 是继承的 ConPTY 句柄，交互行为只能靠 Windows 真机/CI 回归。qalc 需 CI 预装 MSYS2 MinGW64 依赖（pacman 装 gcc/make/pkg-config/libxml2/gmp/mpfr，windows-latest 预装 C:\msys2 只补包），macOS/测试机同样不碰 MSYS2 与 DLL。nmap 三件套走 MSVC + CMake（vendor/nmap/veloterm-win/CMakeLists.txt 复刻官方 vcxproj 出 x64，依赖仅 vcpkg OpenSSL——与 publish 同源）：直接链 Npcap SDK 的 wpcap.lib/packet.lib + 原生 /DELAYLOAD 延迟导入，未装 Npcap 的机器 nmapffi.dll 仍可加载、nmap 降级 connect 扫描（test-nmap 车道在无 Npcap 的 runner 上跑 --version/-sT 冒烟正好覆盖该路径；ncat/nping 各有独立 --only 段与车道）。vim 走上游官方 VIMDLL 形态（`nmake -f Make_mvc.mak VIMDLL=yes CPU=AMD64 vim.exe`，见 vendor/vim/README.md）：零外部依赖（只需 windows-latest 自带 VS 2022），产出 vim64.dll（导出 VimMain）+ 冒烟用瘦启动器（不随包）；构建在 src-tauri/vim-build/ 的源码副本里进行，vendor 树零修补零写脏；macOS/测试机不碰 MSVC 与 DLL。urlencode/urldecode（urlcoding_impl.rs）与 data-encoding 家族同属 Windows 专属命令里的纯 Rust 形态：前者用 crates.io `urlencoding` 库（kornelski/rust_urlencoding，已是 wry 传递依赖，lock 零新增）+ cfg 门控臂，无 vendor、无构建步骤，其参数解析/行拆分管道为无平台差异代码，macOS 门禁直接测；后者见「Windows 工具链」节的 data-encoding 条目。
- 所有 Windows 专属逻辑用 `#[cfg(target_os = "windows")]` / `[target.'cfg(windows)'.dependencies]` 门控，不留运行时判断。
- Windows 本地终端固定 cmd.exe，并经 `cmd /k` 注入「跨盘符 cd 适配层」（`terminal/local_pty.rs` 的 `WINDOWS_CMD_CD_COMPAT`）：doskey 宏把交互输入的 `cd`/`chdir` 自动补 `/d`（cmd 的 cd 不带 /d 只记「该盘当前目录」、不切盘，与 PowerShell/Git Bash 直觉相悖）。宏定义跨平台编译 + macOS 单测锁定形态；只作用于交互式命令行（批处理与 `cmd /c` 保持原生语义），不写注册表 AutoRun。cmd 语义只能靠 Windows 真机/CI 回归——已知边界：`cd /d /d X` 重复开关会报错（ReactOS cmd_chdir 只消费一次 `/D`，余量直接当路径），适配后无需再手写 `/d`。
- CI 规范：**一个 yml 一个目标**。`publish.yml` 是唯一常驻门禁（push release 全量出包，由 worktree 分支合并触发，严禁直接推 release）；原生编译段的快速车道各自独立 yml、**tag 触发**——推 `test_jq` / `test_qalc` / `test_goffi`（历史习惯 `test_yq` / `test_fzf` 同车道）/ `test_nmap` / `test_ncat` / `test_nping` / `test_vim` / `test_cmds` 即触发（`test_<段>*` 前缀可带任意后缀），验证完删 tag（`git push origin :refs/tags/test_jq`）；迭代流程：改动落 worktree 分支（zcode-wt/*）→ 推分支 + 推 tag 验证 → 车道绿后把 worktree 分支合并进 release → 删 tag。现役车道：`test-jq.yml`（coreutils.exe 全量产物，覆盖 jq/bc/openssl 等随 cargo 编译的全部命令；兼作 DLL 宿主验证车道——verify-dll-layout.ps1 实跑 yq 的 lib/ 布局，并经 -OwnArgv 探针实跑 vim 的短名布局）、`test-qalc.yml`（qalcffi.dll，-O0 -g debug，仅 MSYS2 依赖）、`test-yq-fzf.yml`（veloterm_goffi.dll——yq+fzf 合并的 Go 车道，test_goffi/test_yq/test_fzf 同车道）、`test-nmap.yml`（nmapffi.dll + nmap-data/，MSVC+CMake，仅 vcpkg OpenSSL 依赖；段内含无 Npcap 环境的 --version/-sT 冒烟，正好回归 /DELAYLOAD 延迟导入降级路径）、`test-ncat.yml` / `test-nping.yml`（同 nmap 工程，--only 单目标增量验证）、`test-vim.yml`（vim64.dll + vim-runtime/，官方 MSVC nmake，**零外部依赖安装——最轻车道**）、`test-commands.yml`（**全命令验证车道**——全量构建后 verify-win-commands.ps1 搭安装形状，对每个内置命令做常用功能验证（短名 + 子命令双模式，清单由受测 coreutils.exe 自述解析）并出 markdown 报告，依赖面与 publish 同源）。`build-tools.mjs --only <段>`（coreutils/goffi/qalc/vim/nmap/ncat/nping）是局部构建入口，不传时全量（publish 路径不变）；新增原生构建段时同样加 `--only` 段与独立 tag 触发的 yml。qalc 段必须编译期定义 `LIBXML_STATIC`（静态链 libxml2 的符号形态，脚本内已内置）。

## 架构不变量

### Cargo workspace 拆分

`src-tauri` 是 workspace 根（根包 = Tauri 主程序 `app`），三个成员职责不可混：

- **app 不依赖任何工具 crate**——uutils/ripgrep/openssl-app/putty-telnet/jq-app/bc-app 的依赖图只进 `crates/coreutils`。这是「主程序瘦身 + 增量构建互不牵连」的前提：改 UI 只重链 app，改工具只重链 coreutils。
- **tools-catalog 保持零依赖**——命令名清单是 app（建硬链接）与 coreutils（argv0 分发白名单）的单一事实来源；任何依赖都会经它传染给主程序。
- `vendor/` 被 workspace `exclude`：vendor 内 coreutils/sed/ripgrep/sd 自带 workspace（onefetch 亦自带 workspace，根清单 members 为 ascii/image/manifest 三个 path 子 crate；dog 同样自带 workspace，members 为 dns/dns-transport 两个 path 子 crate），其余为无继承键的独立清单；不 exclude 则 uucore 的 `edition.workspace` 会误朝根清单取值而报错。
- 裸 `cargo build` / `cargo test` 只作用于根包 app；工具链必须显式 `-p coreutils`。

### Windows 工具链（BusyBox 模式）

- 单一 `coreutils.exe` 多呼叫二进制；`rg`/`fd`/`awk`/`bat`/`onefetch`/`parallel`/`vim` 七个命令的解析器直读自身进程命令行（vim 经上游 VimMain 内部的 GetCommandLineW 自解析，见 vim 条目），**仅硬链接短名模式可用**，子命令模式给出提示。onefetch（o2sh/onefetch 3.0.0，纯 Rust：gix + tokei + askalono，升级＝换 vendor 树 + 复核 lib.rs 尾部的 veloterm 入口补丁；build.rs 需 assets/onefetch.ico 与 languages.yaml，license 检测的 resources/license.cache.zstd 为编译期 include_bytes）。
- **yq、fzf（Go）与 qalc（libqalculate C++）是 DLL 例外形态**：Go 官方不支持 MSVC 链接 c-archive（golang/go#74189），libqalculate 则是 autotools 体系的 C++ 库（GMP/MPFR/libxml2 硬依赖，上游无 MSVC 工程、官方 Windows 版即 MinGW 构建），都无法像 openssl-app 那样在 MSVC 下静态链入。yq/fzf 合并为**单一 `veloterm_goffi.dll`**（宿主 = vendor/fzf 的 package main——上游 fzf 逻辑在根 main 包无法被 import，且自带 FFI 入口与 shell 资源 embed；经 `vendor/go.work` 跨模块引 yq 的 cmd 包，fzf 树内 `veloterm_goffi.go`（tag veloterm_goffi）追加导出 yq 入口并在 init 关闭常规 GC——单次 CLI 语义，1GB 软上限兜底；构建 `-tags "veloterm_ffi veloterm_goffi"`、`GOAMD64=v3`（AVX2 基线，基线以下老 CPU 非法指令崩溃，回退删环境变量即可）；两份 Go runtime 合一，体积省约三成）；qalc：MSYS2 MinGW64 环境 VPATH 构建 + `qalc.cc -Dmain=qalc_main` 改名编译后 `-static` 链自包含 `qalcffi.dll`（定义 XML 经 `--enable-compiled-definitions` 编进二进制、运行时零数据文件）。coreutils 的 `goffi_impl.rs`（yq/fzf 按符号分发，库句柄 OnceLock 缓存永不 FreeLibrary——Go 运行时持有后台线程）与 `qalc_impl.rs` 运行时 LoadLibrary 调 C 导出（无链接期依赖，macOS/CI 测试机无需 Go/MSYS2 工具链）；DLL 随包走 resources 放安装根 **`lib/` 子目录**（映射见 `tauri.windows.conf.json` 的 `bundle.resources`），候选路径由共享模块 `crates/coreutils/src/dll_paths.rs` 统一生成：本级 `lib/` → 上级 `lib/` → 同目录平铺兜底（前两条分别服务 coreutils.exe 直跑与 `tools\yq.exe` 硬链接短名启动；`lib/` 一律优先于平铺，防旧版残留的平铺 DLL 遮蔽新装文件）。该模块跨平台编译 + macOS 单测覆盖（Windows 车道不跑 cargo test），运行期真机验证见 test-jq 车道的 `scripts/verify-dll-layout.ps1`（搭安装形状实跑 yq），打包映射核验见 publish 的 `scripts/verify-installer-layout.ps1`。升级：换 vendor 源码 + 改 `VELOTERM_VERSION`（fzf 还需按 veloterm_ffi.go 头注释复核复刻的 main() 分支集合；yq 需同步复核 vendor/yq/veloterm_ffi.go 与 vendor/fzf/veloterm_goffi.go 两处同源入口；qalc 还需按树内 README 复核两处 UTF-8 输出补丁）。qalc 的 MinGW 构建依赖由 CI 的 pacman 步骤安装（publish.yml），安装包内的 qalcffi.dll 依赖全静态打入、终端用户不需要 MSYS2。
- **bc 是 jq-app 的同型简化版**：vendor/bc（gavinhoward/bc 7.1.0，BSD-2-Clause）+ vendor/bc-app sys-crate，cc 现场编译静态链入，`-Dmain=veloterm_bc_main` 改名后 FFI 直调（main.c 按 argv[0] 首字符分发 bc/dc，恒传 "bc"）。上游自带官方 MSVC 工程零修补；4 个 strgen 生成物（lib/lib2/help×2）由 build.rs 的 Rust 复刻渲染（与宿主 strgen byte-diff 验证）；仅链 bcrypt。升级 bc：换 vendor 源码 + 同步 build.rs 的 `BC_VERSION` 与 vendor/bc/VERSION.txt（build.rs 启动时校验双源一致）+ 复核 vs/bc.vcxproj 的宏集合。
- **data-encoding 家族（base32hex/base64url/hex/hexupper）是纯 Rust 的 Windows 专属命令**：编解码内核为 crates.io 的 data-encoding（与 uutils 家族既有依赖共享同一份，不走 vendor 树以免同版本双份编译），GNU 风格 CLI 壳在 `crates/coreutils/src/encodings_impl.rs`（`-d/-i/-w/[FILE]`，解码无条件容忍 `\r\n`，对齐 GNU base64）。门控方式与 openssl/yq 相反：仅 tools-catalog 的 Windows 清单门控，实现与分发臂跨平台编译——macOS 单测全量覆盖行为，`data_encoding_family_dispatch` 直连 run_uutils 验证接线。base32/base64/basenc 已由 uutils 跨平台提供，不重复收录。
- **zoxide 是 eza 同型的 vendor 纯 Rust 应用命令**：vendor/zoxide（ajeetdsouza/zoxide 0.10.0）整树入源，静态链入 coreutils.exe（非 DLL、无 build-tools 段、无 CI 依赖安装）。两处本地修补：① 新增 `src/lib.rs` 承接原 main.rs 的模块声明与主逻辑，暴露 `zoxide_main_from`（可注入入口，clap 错误 `e.print()` + 返回退出码、不 `process::exit`，同 eza 的 `eza_main_from`）；② 上游 build.rs 把补全脚本生成到**包目录**（非 OUT_DIR），作为路径依赖构建会写脏 vendor 树，已去完成化并删 [build-dependencies]。门控同 data-encoding 家族：仅 Windows 清单，分发臂跨平台编译，`zoxide_dispatch_and_behavior` 在 macOS 直接覆盖 add→query 数据库闭环（测试必须显式设 `_ZO_EXCLUDE_DIRS`：上游默认排除 $HOME，Windows 的 TEMP 在用户目录内，不设会排除掉测试目录）。升级：换 vendor 源码 + 复核两处补丁仍适用。上游不支持 cmd.exe 钩子（templates 无 cmd），本地 cmd 里 `z` 直跳需 doskey 适配（未做），PowerShell 内 `zoxide init powershell` 可用。
- **dog（命令行 DNS 客户端，dig 的现代化替代）是 eza/zoxide 同型的 vendor 纯 Rust 应用命令**：vendor/dog（ogham/dog，锁 master tip 721440b，包版本 0.2.0-pre——上游 2021-10 起停更、0.2.0 未发版，唯一 tag v0.1.0 缺 master 的 Windows 名字服务器探测改进，故锁 commit 而非 tag）整树入源，静态链入 coreutils.exe，非 DLL、无 build-tools 段、无 CI 依赖安装；自带 dns / dns-transport 两个 path 子 crate（树内 [workspace]，随 vendor exclude 隔离，onefetch 同型）。三处本地修补：① 新增 `src/lib.rs` 承接原 main.rs 的模块声明与主逻辑，暴露 `dog_main_from`（可注入入口；上游 `Options::getopts` 期望不含 argv[0] 的参数表，入口内部 skip(1)；全部 `process::exit` 改为返回退出码：0 成功 / 1 网络错误 / 2 Short 模式无结果 / 3 参数错误 / 4 系统配置错误）；② 根清单补 `[lib]` 目标（上游仅 bin，无 lib 无法作为 path 依赖）；③ build.rs 去 git 化——上游 pre-release/debug 构建把 `git rev-parse` 哈希与编译日期（经 build-dep `datetime`）嵌进版本串，非密封构建且取到的是外层 veloterm 仓库哈希，已删 datetime 与该分支，所有构建统一 release 形态版本串。TLS/DoT/DoH 默认特性走 native-tls（macOS security-framework / Windows schannel，两平台均无 OpenSSL 依赖；security-framework 已被 tauri 栈预置，lock 零重复编译；with_rustls 特性依赖 2019 年的 rustls 0.19，勿启用）。门控同 zoxide 家族：仅 Windows 清单，分发臂跨平台编译，`dog_dispatch_and_behavior` 在 macOS 覆盖 --version/--help/参数错误（退出码 3）与回环 TCP 拒连（退出码 1；output.rs 的 print 对空响应除 Short 模式外恒返回 true）；真实 DNS 解析由上游 xtests 覆盖，Windows 的系统默认名字服务器发现（ipconfig 0.2，winapi 栈，仅 Windows 编译）只能靠 Windows 真机/CI 回归。升级：换 vendor 源码 + 复核三处补丁仍适用。
- **nmap 三件套（nmap / ncat / nping）是 DLL 例外形态里最重的一员（C/C++，走官方 MSVC 路径）**：vendor/nmap（nmap 7.992，已裁 zenmap GUI）由 `veloterm-win/CMakeLists.txt` 经 **MSVC x64 + CMake** 一次构建出三个 DLL——`nmapffi`（扫描器）、`ncatffi`（netcat 替代）、`npingffi`（报文探测）。关键：**照搬官方 mswin32 的 MSVC 路径**（直接链 Npcap SDK 的 `wpcap.lib`/`packet.lib` + 原生 `/DELAYLOAD:packet.dll;wpcap.dll`），故**不编译 bundled libpcap**（官方 sln 只有 Win32 平台、产物 32 位载不进 64 位宿主，故用 CMake 复刻官方 vcxproj 的源清单/宏/include/库清单并 `-A x64`）。三个特殊点（细节全在 vendor/nmap/README.md）：① 未装 Npcap 时三个 DLL 仍可加载（延迟导入），nmap 降级 connect 扫描；② 数据文件：nmap 的 NSE 数据树（含构建期预生成 script.db）随包放安装根 `nmap-data/`，运行期由 nmap_impl.rs 设 NMAPDIR 指过去（上游 Windows 下默认找 exe 同目录，DLL 形态取到宿主 coreutils.exe 路径）；ncat 的 `ca-bundle.crt` 同层放安装根；③ 三个 FFI 壳（veloterm_ffi.cc / ncat_ffi.cc / nping_ffi.cc）各以 `#define main` 改名包含上游入口文件，导出 C 入口；三者 main 内的 exit() 对 coreutils 单命令生命周期进程等价于返回退出码，无需改造。构建期另产 runner exe（不随包分发）用于 script.db 生成与无 Npcap 冒烟（test-nmap 车道在无 Npcap 的 runner 上跑 --version/-sT，正好回归该路径）。许可证是 NPSL（GPLv2+附加条款，§3 衍生定义很宽、商用需 OEM）——商用发布前需项目方确认，详见 vendor 树 README。升级：换 vendor 源码 + 改 VELOTERM_VERSION + 重放补丁清单（winconfig 的 NMAP_PLATFORM/HAVE_LIBSSH2、nse_fs.cc 的 x64 句柄修复）+ 复核 Npcap SDK 版本与 build-tools.mjs 旗标。
- **vim 是唯一「零修补、官方自带 DLL 拆分」的例外形态（C，官方 MSVC nmake 路径）**：vendor/vim（vim 9.2.1202，裁 testdir/po/GvimExt 与 GUI 专属 runtime 位）走上游 `VIMDLL=yes` 官方形态（src/INSTALLpc.txt）：`nmake -f Make_mvc.mak VIMDLL=yes CPU=AMD64 vim.exe` 产出主体 `vim64.dll`（`__declspec(dllexport) int VimMain(int, char **)`，静态 CRT、FEATURES=HUGE、零外部依赖）+ 瘦启动器 vim.exe（留作 CI 冒烟 runner，不随包）。三个关键机制：① MSWIN 下 VimMain **无条件**用 GetCommandLineW 自解析进程命令行（入口参数被忽略）→ vim 归 OWN_ARGV 家族，仅硬链接短名模式可用；② GUI/控制台形态由 `mch_is_gui_executable` 按宿主进程 exe 的 PE subsystem 决定（coreutils.exe 是控制台程序，恒为控制台 vim）；③ 运行文件树（syntax/doc 等，上游自带 doc/tags）随包放安装根 `vim-runtime/`，运行期由 vim_impl.rs 设 VIMRUNTIME 指过去（仅当用户未设）；`vimrun.exe` 不随包（上游按 exe 同目录探测、找不到自动走直调 shell 路径）。分发臂 `vim_impl.rs` 运行时 LoadLibrary 调 VimMain(0, NULL)（与上游启动器同参，无链接期依赖），DLL 候选走共享模块 `crate::dll_paths`（vim64.dll 随包放安装根 `lib/` 子目录），`vim-runtime/` 数据树用 dll_paths 的 `flat_dir_candidates`（平铺不进 lib/，同 nmap-data）；vim 的部分退出路径直接 exit()，对单命令生命周期进程等价于返回退出码（同 nmap 约定）；交互 TUI 手感只能靠 Windows 真机回归（同 fzf）。构建在 src-tauri/vim-build/ 源码副本里进行（nmake 需在源内构建，不写脏 vendor 树）；vswhere 定位 vcvars64，无需 vcpkg/MSYS2。运行期宿主验证在 test-jq 车道（verify-dll-layout.ps1 的 -OwnArgv 探针——子命令模式按设计不支持，布局 2 跳过、3/4/5 经短名硬链接调用）。升级：换 vendor 源码 + 改 VELOTERM_VERSION + 重放裁剪清单 + 复核 VimMain 导出/`mch_is_gui_executable`/`get_cmd_argsW` 三处引用点仍在。
- 硬链接安装在应用启动钩子（`src/tools/hardlinks.rs`，仅 Windows 编译）完成：幂等判据是**文件身份**（Unix inode 对；Windows 退化为大小 + mtime），不是文件大小——同体积升级曾因此误判。
- 依赖 uutils 家族统一走 vendor 的 uucore 0.12.0（各 vendor 清单已重定向），避免 lock 出现双版本。
- 新增命令：改 `tools-catalog` 清单 → `crates/coreutils/src/coreutils.rs` 加分发臂 → 补 command_table_consistency 与 version sweep 的预期（平台不对称的命令要么条件化 skip，要么如 data-encoding 家族把实现做成跨平台编译、仅清单门控，则无需 skip）。

### OpenSSL 锁版本

`vendor/openssl-src` 只含 apps + include/internal 最小子集。版本锁定是**双源手维护**（升级时两处必须同步改）：`VELOTERM_VERSION`（源码锁版本）+ `src-tauri/vcpkg.json` 的 overrides（CI 安装侧）——vcpkg 版本控制仅在 manifest 模式可用（classic 模式没有命令行版本语法），CI 以 manifest 精确安装锁定版本（当前 **3.6.4#1**，即 vcpkg 上游版本数据库可得的最高版）。曾踩的坑：classic 模式跟随 runner ports 快照装出过 versions 数据库里不存在的 3.6.5（上游从未收录），manifest 解析直接报 no version database entry——**锁版本必须以 vcpkg 版本数据库实有版本为准**，升级前先查 `vcpkg search openssl` 或 versions/o-/openssl.json；builtin-baseline 指向的 SHA 需先 `git fetch` 补齐旧镜像历史。apps 的 `cmp.c` 与 `lib/cmp_mock_srv.c` 是上游 build.info 里的成对项，必须一起排除。CI 的 manifest 安装用 `--x-install-root` 把产物固定装进 classic 目录（`<vcpkg根>\installed\<triplet>`）而非默认的 `<清单目录>\vcpkg_installed\`：OPENSSL_LIB_DIR 字符串必须与 rust-cache 恢复的旧产物里烘焙的链接搜索路径一致——libsqlite3-sys 的 bundled 路径未声明 `rerun-if-env-changed=OPENSSL_LIB_DIR`，改环境变量 cargo 也不重跑其 build script，只重放缓存的旧 `rustc-link-search`，路径失效即 LNK1104（CI 实测）。

### 前端约定

- 弹层（下拉/右键菜单）一律 portal 到 `document.body` + fixed 定位（共享 `useAnchoredPanel`：视口夹取、空间不足上翻）；关闭判定用 `useDismissOnOutsidePointer` 的多 ref 排除区（触发器与弹层不同子树）。tooltip 同一约定走共享 `useTooltipPlacement`（`side` 是期望侧，空间不足自动翻对侧、两侧都放不下视口夹取）——两套 tooltip 组件（`ControlHintTooltip` / `ui/tooltip`）都已接上，禁止再写内联 absolute 定位的提示层（会被 tab 栏/底部工具栏的 overflow 裁掉）。
- 表单字段容器 `FormField` 用 div 不用 label——WebKit 会把 label 内点击转发给第一个可标签化后代，曾致「取消新建命令后下拉自动展开」。
- 颜色/字号全部走语义 token（`bg-selected` / `border-hover` / `text-ui-*` 随 `--ui-font-size` 缩放）；选中态不用主色三件套（primary 边框+底色+文字），主色只留给活动指示。
- Monaco 经 `import * as monaco from 'monaco-editor'` 全量入口（81 语言高亮全离线）；worker 用 `new Worker(new URL(...), { type: 'module' })` 让 Rspack 产独立 chunk，`new URL` 必须内联在 `new Worker` 里。
- 终端内提示（断开/重连快捷键）直接写 xterm 缓冲，不叠 DOM 覆盖层；就绪钩子命令等首屏输出静止 500ms 再注入（早发会被内核回显 + readline 重绘各显示一次，看到两遍）。
- **拖放一律走统一管线**（`services/dragDropService.ts` + `hooks/useDropTarget.ts`）：两平台 `dragDropEnabled: true`（原生接管，携带文件绝对路径），wry 会整体接管 WebView 的 DnD——页内 HTML5 dragover/drop **不再触发**，页内拖拽（SFTP 面板互拷、Tab 分栏）同样必须走该管线（dragstart 时 `beginInternalDrag` 存载荷，Drop 按坐标命中注册目标），**严禁新增依赖 HTML5 dataTransfer 读取的 drop 处理**。终端拖入文件 → 路径按目标 shell 格式化（`lib/shellPath.ts`：Windows 按需双引号 / POSIX 反斜杠转义）后 `term.paste` 写入。终端复制/粘贴快捷键：macOS ⌘C/⌘V；Windows Ctrl+Shift+C/V 且裸 Ctrl+V 亦粘贴、Ctrl+C 有选区时复制（conhost/Windows Terminal 语义），右键菜单项带键位指示。

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
