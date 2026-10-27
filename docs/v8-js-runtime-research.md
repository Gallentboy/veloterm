# JS 脚本运行时（V8 / rusty_v8）调研方案

> 状态：**调研文档 + P0/P1/P1.5/同步 fetch/同步 fs 沙盒/同步 crypto/同步 sys 已落地**（2026-10-27）。P0：`src-tauri/src/js_runtime.rs`、`v8 = "=152.2.0"`、`src-tauri/vendor/rusty_v8/` 四平台静态库 vendor（零下载构建，SHA-256 清单见其 README；例外一处：**静态库 vendor 进工程而非构建期下载**，`RUSTY_V8_ARCHIVE` 目录权威源模式）。P1：`js_run`/`js_run_cancel` 命令（console 流式 + 取消）+ 侧栏「JS 脚本」页。**P1.5 TypeScript（oxc 后端转译已落地）**：`js_runtime/ts_transpile.rs`（transpile-only，oxc 0.156 系 + oxc_sourcemap 9.x）+ 全链路 sourcemap——异常消息带「TS 第 L 行第 C 列」（V8 Message 行列 → `SourceMap::lookup_token` 回映）、console 行带调用者位置（`StackTrace::current_stack_trace` 栈顶帧回映，"L:C"）；前端 JS/TS 分段切换（模型语言热切，ts.worker 诊断白拿）。**同步 fetch 宿主桥已落地**：`js_runtime/fetch_impl.rs` 提供全局 `fetch` 与 `http.fetch` 别名（纯同步调用，直接返回 SyncResponse 实体而非 Promise，支持 status/headers/url 与 text()/json()/bytes()；reqwest 异步 client + 独立 Tokio Runtime 的 block_on 桥接；select! 结合取消旗标与脚本剩余超时打断原生阻塞 IO，30MB 响应体上限守卫）。**同步 fs 沙盒宿主桥已落地**：`js_runtime/fs_impl.rs` 提供全局 `fs` 对象（纯同步阻塞无异步；严格白名单沙盒目录，仅允许操作 `ProjectDirs::data_dir().join("scripts_workspace")` 和系统临时目录 `veloterm_scripts`，相对路径默认绑定工作区，严防 `..` 越界与符号链接逃逸；提供 readTextFile/readFile/writeTextFile/writeFile/appendFile/exists/mkdir/readDir/remove/stat/copyFile/rename/getWorkspaceDir/getTempDir 及其 Sync 别名）。**同步 crypto 模块已落地**：`js_runtime/crypto_impl.rs` 提供全局 `crypto` 对象（纯同步硬件加速、跨平台原生零外部 DLL 依赖；哈希：md5, sha1, sha256, sha384, sha512, sha3 [SHA3-256]；CRC32：纯 Rust 硬件指令集加速循环冗余校验；HMAC：md5/sha1/sha256/sha384/sha512/sha3；对称加密：aes-128-cbc PKCS7 padding、aes-256-gcm AEAD 硬件指令加速；KDF 密钥派生：pbkdf2、hkdf；现代椭圆曲线：ed25519 密钥对生成/签名/验签、x25519 共享密钥交换；NIST 椭圆曲线：p256 [secp256r1] 密钥对生成/ECDSA 签名验签/ECDH 协商；RSA 系列：RSA-PKCS1-v1_5 / RSA-PSS 签名验签、RSA-OAEP 公钥加密与私钥解密、密钥对生成；辅助：randomUUID, randomBytes；支持 hex / base64 / bytes 输出格式）。**同步 sys 模块已落地**：`js_runtime/sys_impl.rs` 提供全局 `sys` 对象（version: V8 引擎版本、system: OS 名称及版本、arch: CPU 架构、platform: 平台标识；上述静态信息由 `OnceLock` 缓存只探测获取一次；以及 `sys.memory()` 实时获取当前 V8 isolate 堆内存统计 `{ used, total, limit }` 原生数值对象）。前端 Monaco 注入精确 DTS 类型定义。单测 38 项全绿。脚本库 DB 实体与 P2 其它宿主桥（ssh.exec/sftp）未启动；未实测项：Windows 双腿链接。结论若与源码不符，以源码为准并回改本文。

## 0. 结论速览（TL;DR）

- **可行**。rusty_v8（crates.io 包名 `v8`）为官方静态预编译分发，四条发布腿（Windows x64 / Windows ARM64 / macOS x64 / macOS ARM64）的 release 静态库全部现成（每包 gz 约 39–41 MB），构建机零 V8 工具链要求，MIT 许可无商用负担。
- **与「运行期加载」铁律不冲突但也不属于它**：rusty_v8 是「构建期自动下载静态档案 + 链接期静态打入」形态，macOS ARM64 的 pdfium 静态链接（build.rs `link_static_pdfium_if_needed`）已验证过同款 profile（fat LTO + panic=abort + strip）与静态 C++ 库共存。真正的代价是**成品二进制/安装包每平台约 +15–40 MB** 与**发布链接耗时增加**——这是 PoC 要量化的两个硬指标。
- **推荐形态**：P0–P2 用**裸 rusty_v8**（锁死版本）而非 deno_core；JS 执行走**专用宿主线程 + 每次执行独立 isolate**（!Send 约束下的自然形态），超时经 `IsolateHandle::terminate_execution` 可杀死死循环，堆上限经 `CreateParams` 收紧；P0 只做同步脚本（不引事件循环）。
- **主要退出条件**：若 PoC 实测体积增幅不可接受（或 Win ARM64 链接意外受阻），备选 quickjs（rquickjs 绑定）同 API 设计可平移，宿主 API 层先按引擎无关 trait 设计。

## 1. 目标与范围

给 veloterm 增加内置 JS 脚本能力，定位是**本地自动化胶水层**，分期：

| 阶段 | JS 能拿到什么 | 形态 |
| --- | --- | --- |
| P0 PoC | 纯 ECMAScript 计算 + console 输出 + JSON | 后端模块 + 单测，无 UI |
| P1 MVP | 脚本库（veloterm.db 存脚本，命令库/主机同款实体惯例）+ 运行面板（输出走 Tauri Channel 流式，dbQuery/fleet_exec 同款）+ 超时/内存配置 | 前端 UI + `js_run` 命令 |
| P2 | 宿主桥：`http.fetch`（reqwest 桥）、`ssh.exec(hostId, cmd)`（复用 `resolve_host_chain` + `connect_ssh_chain_headless` + `sftp_exec_capture_status`，fleet_exec 同管线）、`sftp.read/write` | 能力白名单按脚本显式开启 |
| P3 | 脚本间 ESM 互引、异步（timer/Promise 事件循环）、DB 会话查询桥 | 届时再评估是否换 deno_core |

与在途的 fleet_exec（批量执行，见 `src/services/fleetExecService.ts` / `src-tauri/src/fleet_exec.rs`）**互不依赖**：fleet_exec 是「同一条命令 × N 主机」的固化形态，JS 脚本是其编程化超集（循环/分支/结果解析）；P2 的 ssh 桥复用同一管线，两者不共享代码耦合点。

## 2. rusty_v8 现状事实（已核实）

### 2.1 版本与许可

- crates.io 包名 **`v8`**（`rusty_v8` 旧名已停发），最新 **152.2.0**（2026-08-20），版本号跟随 V8 里程碑（V8 15.2）。发版节奏约月更（150.3 → 150.4 → 152.0 → 152.1 → 152.2）——**必须锁死精确版本**（项目 oracledb beta 同款惯例 `= "152.2.0"`）。
- 许可 **MIT**（crates.io 核实），无 GPL 系商用确认负担（区别于 libjxl/nmap/agg 先例）。
- MSRV 未声明；edition 2024 与本工程一致，Rust 1.90 工具链按 PoC 实测为准。

### 2.2 分发机制（构建期行为）

- 默认形态：**build.rs 自动从 GitHub Releases 下载对应 target 的预编译静态库**（release 每包 gz 约 40 MB），解压进 OUT_DIR 供链接；V8 源码构建（`V8_FROM_SOURCE=1`）需要完整 Chromium 工具链，**本工程不可走、也不需要**。
- 缓存：`~/.cargo/.rusty_v8`（按 tag + 文件名键控，跨项目/跨 clean 持久）——每台开发机/CI runner 只下载一次。
- 关键环境变量（README 核实）：
  - `RUSTY_V8_ARCHIVE_SHA256`：钉死档案哈希，不匹配强制重拉——**CI 供应链可复现的抓手**。
  - `RUSTY_V8_ARCHIVE` / `RUSTY_V8_MIRROR`：本地档案直指 / 镜像 URL 模板（`{tag}` `{target}` 等占位）。
  - `RUSTY_V8_SKIP_DOWNLOAD=1`：跳过大档案只取绑定文件，**专供 cargo check / clippy / rust-analyzer**（链接仍需真档案）。
- Windows 侧档案名是 `rusty_v8_release_x86_64-pc-windows-msvc.lib.gz` 形态（.lib 无 lib 前缀），clang-cl 产物链入 MSVC 工程——Deno 官方 Windows 出货即此路径。

### 2.3 平台矩阵（对应 publish 四腿，GitHub API 核实 v152.2.0）

| 发布腿 | 预编译资产 | gz 体积 |
| --- | --- | --- |
| Windows x64（windows-latest） | `rusty_v8_release_x86_64-pc-windows-msvc.lib.gz` | ~40.0 MB |
| Windows ARM64（windows-11-arm） | `rusty_v8_release_aarch64-pc-windows-msvc.lib.gz` | ~38.9 MB |
| macOS x64（macos-26-intel） | `librusty_v8_release_x86_64-apple-darwin.a.gz` | ~40.5 MB |
| macOS ARM64（macos-26） | `librusty_v8_release_aarch64-apple-darwin.a.gz` | ~40.6 MB |

四腿全齐，含 Windows ARM64（Deno 自 2024 末起官方出 win-arm64 二进制，同档案）。另有 musl / iOS / Android / riscv64 等资产与本工程无关。`ptrcomp`（指针压缩）变体 Darwin/Linux 有、**Windows 无**——cargo 会按 target 自动选默认（非 ptrcomp）变体，无需干预。

### 2.4 API 形态（docs.rs v152.2.0 核实）

- `v8::Isolate` **`!Send + !Sync`**：isolate 严格单线程归属——执行模型的根本约束（见 §4）。
- 取消：`Isolate::terminate_execution()` / `thread_safe_handle() -> IsolateHandle`（Send）供**看门狗跨线程终止**——死循环/超时的杀手段。`cancel_terminate_execution` / `is_execution_losing`（`is_execution_terminating`）配套。
- 内存：无便捷 `set_memory_limit`；堆上限经 `Isolate::create_params()` 的 heap limits 配置 + `add_near_heap_limit_callback` 逼近回调；`get_heap_statistics()` 可回报用量。
- 平台初始化：进程级一次（`V8::initialize_platform` + `V8::initialize`），platform 生命周期必须覆盖所有 isolate（进程级静态）。README 明确记载：**非主线程初始化时用 `v8::new_unprotected_default_platform` 规避 PKU 崩溃**（放弃 V8 JIT 页的 PKU 硬隔离）。
- V8 只提供 **纯 ECMAScript**：console / setTimeout / fetch / TextEncoder 全部是宿主绑定，需自建（§6）。

### 2.5 体积代价（社区数据点，PoC 实测为准）

- V8 静态链入对 stripped release 二进制的增幅：**约 15–25 MB+**（Deno 体积构成 issue 与社区讨论数据点）；本工程成品当前约数十 MB 级，**预计 +20–40 MB/平台**（gz 后安装包增幅更小，DMG/NSIS 压缩后待实测）。
- 链接耗时：V8 未压缩档案预计数百 MB，fat LTO 发布链接预计每腿 +数十秒（pdfium 先例的放大版，实测记录）。

## 3. 与工程铁律的对照

### 3.1 「运行期加载」铁律的辨析（关键判断）

铁律原文针对的是**链接期动态链接的绑定 crate**（pkg-config 形态：构建机无库 build script panic / 进程启动硬依赖库文件）。rusty_v8 不属于该形态：

- 静态档案由 build.rs **自动下载**，构建机不需要预装任何 V8 库，无库时不是 panic 而是下载（离线可 `RUSTY_V8_ARCHIVE` 直指本地档案）；
- 链入后**进程启动零额外依赖**（无 DLL/dylib 分发与签名问题——比 pdfium 的 Windows/macOS-Intel 动态形态还干净）。

真正的对标先例是 **macOS ARM64 的 pdfium 静态链接**（`build.rs::link_static_pdfium_if_needed`）：fat LTO + `panic = "abort"` + `strip = true` 的 release profile 与静态 C++ 档案共存已被发布验证。rusty_v8 走同一路径，V8 档案由其自身 build.rs 接线（`rustc-link-lib` 等无需我们手写），app 的 build.rs **零改动**。

### 3.2 workspace 放置

- **放 app crate**（新模块 `src-tauri/src/js_runtime/`），不放 coreutils（脚本不是 CLI 命令、不进 argv0 分发），不建独立 member crate（app 反正要链它，多一层无构建收益）。bash_analysis（tree-sitter 静态编入 app）是同型先例。
- relay 独立 workspace 不受影响（不依赖）。
- 版本锁死 `[dependencies] v8 = "=152.2.0"`，升级 = 人工评估 + 回归（oracledb beta 同款约定，写进 Cargo.toml 注释）。

### 3.3 门禁与 CI 影响

| 场景 | 影响 | 对策 |
| --- | --- | --- |
| 本机 `cargo check -p app --lib` / clippy | 首次会触发 40 MB 下载（一次缓存） | check-only 车道可设 `RUSTY_V8_SKIP_DOWNLOAD=1`；本机开发接受一次下载 |
| 本机 `cargo test -p app --lib` | 链接需要真档案 | 同上缓存，无额外动作 |
| `pnpm typecheck` / `build`（前端） | 不涉及 | — |
| publish 四腿 | 每腿一次 40 MB 下载（rust-cache 缓存 target 含 OUT_DIR 产物，热缓存零下载） | publish.yml 设 `RUSTY_V8_ARCHIVE_SHA256` 钉死（供应链可复现）；如 GitHub Releases 访问不稳再挂 `RUSTY_V8_MIRROR` |
| 全量门禁表 | 基线测试数会增长 | 落地时更新 AGENTS.md 基线与本文 |

### 3.4 clippy 约束

app crate `panic = "deny"` + `indexing_slicing = "deny"` 对 V8 胶水层同样生效：宿主回调走返回值/Result 传播错误，不 panic；绑定层数组访问用 get。

## 4. 执行架构设计（推荐方案）

### 4.1 初始化（进程级一次，懒加载）

推荐：**首用懒初始化**。用户第一次运行脚本时：

1. 起一条常驻「JS 宿主线程」（进程内单例，OnceLock 守护）；
2. **该线程上**完成 `V8::initialize_platform` + `V8::initialize`——非主线程初始化按 README 用 `new_unprotected_default_platform`（放弃 PKU 硬隔离；本地脚本属用户信任域，与 Deno 主线程初始化的差额风险可接受，PoC 双态实测确认）；
3. 应用退出时随进程回收（不追求显式 dispose）。

备选：启动期主线程 eager 初始化（PKU 完整，但 V8 平台线程常驻、启动耗时 +ms 级）——仅当 PoC 发现 unprotected 形态有问题时切换。**启动零成本优先**，终端应用不该为可能永远不用的功能付启动代价。

### 4.2 执行模型（每次执行独立 isolate）

```
Tauri async command(js_run)
  ├─ 组装请求（脚本源码、超时 ms、堆上限 MB、能力白名单）→ mpsc 发给 JS 宿主线程
  ├─ 看门狗：tokio::time::timeout 包住等待；超时 → IsolateHandle::terminate_execution()
  └─ 宿主线程循环：
       每请求新建 Isolate（CreateParams 设堆上限，默认 256 MB）
       → Context + 注入宿主绑定（console 等，按白名单）
       → 编译 + 运行；console 输出经 Channel 流式回传（dbQuery 同款 tag=type）
       → perform_microtask_checkpoint 收尾 → 取返回值序列化（JSON）
       → drop isolate（每脚本自然隔离，无跨脚本状态泄漏）
```

- **每次执行新建 isolate**：新建成本 ms 级（快照优化留到有真实 JS 预置库时再评估），换来零泄漏与无限次运行的确定性——脚本库场景正确性优先。
- **取消语义双通道**：用户取消（UI 按钮）与超时同走 `terminate_execution`；运行中的 JIT 死循环可被立即打断（V8 在回边/函数入口检查终止标志）。
- **P0 不做事件循环**：不提供 setTimeout / Promise 驱动（同步脚本的 Promise 链经 microtask checkpoint 自然收尾）；异步是 P3 议题（自建 pump 或换 deno_core）。
- 宿主桥（P2）在同步模型下的形态：`ssh.exec()` 之类的 Rust 异步能力经「请求通道发到 tokio 侧 + 阻塞等待结果」桥接（宿主线程专用，不占用 isolate 并发）——与 fleet_exec 管线复用，语义简单。

### 4.3 与裸 rusty_v8 vs deno_core 的取舍

| 维度 | 裸 rusty_v8 | deno_core（0.413.0，2026-10-21） |
| --- | --- | --- |
| 依赖面 | 仅 v8 crate | + 若干 deno 系 crate（serde_v8、ops 宏等） |
| API 稳定性 | v8 crate 月更但锁死即稳；API 面大而全、贴近 C++ | **401 个版本、高 churn**；版本链三层 `deno_core → deno_v8 → v8`（crates.io 2026-10 核实：deno_core 0.413 → deno_v8 ^0.5.0 → v8 ^150.4.0），v8 升级被动跟随且当前落后独立 crate 两个里程碑（独立已 152.2.0，^150.4 不含 151/152） |
| 能力 | 手写一切（scope/context/绑定/序列化） | 现成 JsRuntime / 事件循环 / ops 宏 / ESM 加载器 / 快照工具链 |
| 适配本方案 | P0–P2 同步脚本足够 | 到 P3 异步/ESM 才显价值 |

> **2026-10 附加核实（Deno 侧正在换轮子）**：`deno_v8` 是 deno 仓库新出的「引擎门面」crate（发布首月内 0.1→0.5 共 5 版），默认 v8 后端、feature 清单已见 quickjs 字样，另有精确锁 `v8x =149.4.0-rc.4` 的 rc 依赖——Deno 正在把引擎层抽象化并为 quickjs 后端铺路。这与本文 §7 的「引擎无关 trait + quickjs 退出路径」是同一个形状（上游动作反证该设计方向）；同时意味着此刻引入 deno_core 是在对方重构引擎层的当口上车，P3 评估时须重看 deno_v8 走向。

**结论：P0–P2 裸 rusty_v8**（绑定面完全自控、升级面只有 v8 crate 一个）；P3 立项时重新评估 deno_core——若届时异步+ESM 是确定需求，接受版本链换取事件循环/模块加载器是合理交换；评估前先核 deno_v8 重构是否落定。

## 5. 宿主 API 与安全模型

- **信任域定性（明示）**：JS 脚本 = 与用户手敲 shell 同级的本地代码执行能力，**不承诺对宿主的沙箱隔离**；「沙箱」仅指默认无 fs/net 直达——能力只能经白名单桥暴露，且每个脚本按需声明（P2 起脚本记录带能力开关，UI 明示）。
- P0 绑定面（最小）：`console.log/warn/error`（Channel 流式）、`btoa/atob`、`TextEncoder/TextDecoder`、结果值 JSON 序列化回传（undefined/函数等给占位串）。
- P2 桥（全部走既有设施，零新连接栈）：`http.fetch`（reqwest 桥、超时透传）、`ssh.exec(hostId, cmd, opts)`（`resolve_host_chain` → `connect_ssh_chain_headless` → `sftp_exec_capture_status`，fleet_exec 同管线，跳板/中继自动生效）、`sftp.readText/write`（SFTP manager 既有会话能力）。
- 每桥一个 Rust 函数 + JS 侧一个绑定，白名单在 `js_run` 请求里按脚本配置过滤注入——不注入即不存在（能力面编译期不可枚举，运行期按需出现）。

## 6. 风险清单（每条带验证动作）

| # | 风险 | 概率 | 验证 / 缓解 |
| --- | --- | --- | --- |
| 1 | 成品体积 +20–40 MB/平台不可接受 | 中 | PoC 首项即量化（release 前后体积对比）；不可接受 → §7 备选 quickjs |
| 2 | 发布链接耗时显著增加（fat LTO × 数百 MB 档案） | 高 | PoC 实测记录；必要时该阶段评估 thin-LTO/关 LTO 的体积/耗时权衡（需单列决策） |
| 3 | Win ARM64 .lib（clang-cl 产）与本工程 MSVC 链接意外 | 低 | Deno 官方出货同路径；PoC 可加 `test_js` tag 车道（windows 两腿 release 构建冒烟）早暴露 |
| 4 | 非主线程初始化 PKU 问题（unprotected 平台的隐性代价） | 低 | PoC 双态（unprotected 专用线程 / 主线程 eager）各跑全量单测 |
| 5 | v8 crate 月更 churn、API 破坏性变更 | 中 | `=` 锁死；升级独立 MR + 全量回归 |
| 6 | clippy deny panic/indexing_slicing 在 unsafe 胶水层 | 中 | 回调 Result 化、get 化访问，写进模块约定 |
| 7 | 下载供应链（GitHub Releases 档案被替换） | 低 | publish 设 `RUSTY_V8_ARCHIVE_SHA256`；档案哈希写进 yml 供人工核对 |
| 8 | ICU/locale 能力面缩水（V8 small-ICU） | 低 | 脚本场景用不到完整 ICU；不做 i18n 承诺，文档记边界 |
| 9 | 死循环/内存炸弹拖垮应用 | 低 | §4.2 双通道终止 + 堆上限 + near-heap-limit 回收，PoC 单测锁定（`while(1){}` 超时被杀、`new Array(1e9)` 触顶） |

## 7. 备选引擎与退出条件

| 引擎 | 绑定 | 体积 | 取舍 |
| --- | --- | --- | --- |
| **V8（rusty_v8）** | 官方、Deno 同源 | +20–40 MB | ES 规范最新最全、JIT 性能、长期维护最强；本项目推荐主选 |
| quickjs-ng | rquickjs（纯 C ~1 MB，cc 现场编译，bc-app 同型） | +1–2 MB | ES2023、解释执行够用；**体积退出条件触发时的平移路径** |
| Boa | 纯 Rust | 零 C | 性能/规范成熟度尚不足以承担主力 |

**宿主 API 层按引擎无关 trait 设计**（`js_runtime/host.rs` 定义桥接口），使 §5 的能力面在引擎层之下不感知——退出条件（体积/链接/平台受阻）触发时换引擎不动产品层。

## 8. PoC 计划（P0）与验收门禁

范围：`src-tauri/src/js_runtime/` 模块 + `v8 = "=152.2.0"` 依赖 + 下列单测，**无前端改动**。

1. 初始化：懒初始化单例（专用线程 + unprotected 平台），重复调用幂等；
2. 同步执行：`1+1`、JSON.parse/stringify 往返、中文字符串无损；
3. console 桥：log/warn/error 经回调收集，顺序保持；
4. 返回值序列化：对象/数组/null/undefined 各态；
5. 终止：`while(true){}` 在 500 ms 看门狗下被杀且返回结构化超时错误；终止后宿主线程仍可服务下一次执行（isolate 可重建性）；
6. 内存：分配风暴触发堆上限，错误可捕获、进程不崩；
7. 双态初始化（unprotected / 主线程 eager）全量单测各跑一遍。

门禁（AGENTS.md 局部口径）：`cargo test -p app --lib js_runtime` 全绿 + `cargo clippy -p app --all-targets` 零警告 + release 构建（`cargo build -p app --release`）成功并**记录体积/链接耗时前后对比**。可选：`test_js` tag 车道（windows-latest + windows-11-arm 各 release 构建冒烟）提前验证两腿链接。

PoC 通过后：P1 立项时同步更新 AGENTS.md（新能力条目 + 门禁基线数字 + 已知边界）。

## 9. 来源

- [denoland/rusty_v8 仓库与 README（分发/环境变量/平台）](https://github.com/denoland/rusty_v8)
- [rusty_v8 release v152.2.0 资产清单（GitHub API）](https://github.com/denoland/rusty_v8/releases) · [crates.io `v8`](https://crates.io/crates/v8)
- [docs.rs v8 152.2.0 — Isolate API](https://docs.rs/v8/latest/v8/struct.Isolate.html)
- [crates.io `deno_core`](https://crates.io/crates/deno_core) · [deno_core releases](https://github.com/denoland/deno_core/releases) · [crates.io `deno_v8`（deno_core 的引擎门面，2026-10 新生）](https://crates.io/crates/deno_v8)
- 体积数据点：[deno#9243 What takes up space in deno](https://github.com/denoland/deno/issues/9243)（V8 为发布二进制最大单一成分）；社区实测静态链入约 15–25 MB+
