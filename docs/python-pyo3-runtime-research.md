# Python 脚本运行时（PyO3 + CPython）调研方案

> 状态：**调研文档，能力未落地**。所有「已核实」事实附来源；体积/耗时数字除注明外均为 PoC 待实测项。
> 本文是 `docs/v8-js-runtime-research.md`（JS/V8 路线）的姊妹篇：两者共同回答「内置脚本引擎选谁」，本文只回答 Python/PyO3 路线的可行性、形态与代价。结论若与后续源码不符，以源码为准并回改本文。

## 0. 结论速览（TL;DR）

- **技术可行，但形态平台不对称**——这是与 V8 方案（四腿全静态、单一形态）最大的结构性差异：
  - PyO3 是**链接期依赖**形态（pyo3-ffi 的 extern 全部链接期解析），没有也不接受 pdfium/libjxl 式 `bind_to_library` 运行期分支；odbc_engine 式「手写 21 个入口 + libloading」的路子对 CPython 不成立（嵌入所需 C API 面大，且手写即放弃 PyO3 的全部安全层）。**选 PyO3 即接受链接期链接 libpython。**
  - **macOS 双腿 = 静态链入**：python-build-standalone 完整包含 `libpython .a`，静态打入 app 二进制（macOS ARM64 pdfium 静态先例、rusty_v8 同型），无 dylib 分发与未签名拒载问题。
  - **Windows 双腿 = 动态 + /DELAYLOAD**：CPython 官方不出静态库（PyO3 官方文档明言 Windows 静态「almost never done」），唯一 PyO3 路径是链接期动态 `python314.dll`。为守住「缺库不拖累进程启动」铁律，走 **/DELAYLOAD + 延迟加载钩子**（nmap 三件套对 wpcap.dll 的 /DELAYLOAD 先例 + `dll_paths` 候选路径复用）：DLL 缺失/被杀毒误删时应用照常启动，Python 功能报「运行时缺失」的可诊断错误（ODBC 缺驱动管理器同款文案形态）。
- **解释器来源**：python-build-standalone（astral 维护，uv 同源），四腿资产全齐、近乎日更（核实 tag 20261026，Python 3.11–3.16a1）；推荐锁 **3.14 线**（第二位 patch 版本，生态最熟，freethreaded 已稳定）。install_only_stripped 压缩包实测 22–27 MB/腿。
- **执行模型**：进程级单解释器 + 专用宿主线程 + GIL（PyO3 `attach`/`detach`），初始化一次、**永不 finalize**。隔离性（sys.modules 跨脚本共享）、取消（只能打断 Python 字节码，C 调用不可中断）、内存（无堆硬上限）三项均**弱于** V8 isolate 方案——这些是 CPython 的结构性边界，不是可修问题，需产品层接受。
- **许可无商用确认负担**（Python PSF-2.0 + 捆绑第三方全 permissive；PBS 刻意以 libedit 替 readline 规避 GPL）——区别于 libjxl/nmap/agg 先例；需随包带第三方许可清单文本。
- **与 V8 的取舍是产品决策**：veloterm 用户画像（运维/DBA）语言亲和 Python，但工程干净度（全静态、每执行新 isolate、terminate_execution、堆上限）V8 全面占优。宿主桥层按引擎无关 trait 设计（v8 调研 §7 已约定），两引擎可并存（体积代价 ×2），也可二选一。

## 1. 目标与范围

定位与 v8 调研一致：**本地自动化胶水层**，分期：

| 阶段 | Python 能拿到什么 | 形态 |
| --- | --- | --- |
| P0 PoC | 纯解释器执行 + stdout/stderr 桥 + 标准库 | 后端模块 + 单测，无 UI |
| P1 MVP | 脚本库（veloterm.db 存脚本）+ 运行面板（Channel 流式，dbQuery/fleet_exec 同款）+ 超时配置 | 前端 UI + `python_run` 命令 |
| P2 | 宿主桥：`http.fetch`、`ssh.exec(hostId, cmd)`（复用 `resolve_host_chain` + `connect_ssh_chain_headless` + `sftp_exec_capture_status`，fleet_exec 同管线）、`sftp.read/write` | 能力白名单按脚本显式开启 |
| P3 | pip --target 三方包（仅 Windows 形态语义成立，见 §7）、free-threaded 评估 | 届时再议 |

**明确非目标**：
- 交互式 REPL 面板——终端里直接 spawn 系统 `python`（portable-pty 既有设施）即可，不需要嵌入；
- 数据库工作台内嵌 pandas 级分析——依赖 pip + C 扩展，与 §7 边界 4 冲突，不承诺。

与 fleet_exec 互不依赖（同 v8 调研 §1 的表述）；与 js_runtime 调研共享产品层与宿主桥 trait 设计。

## 2. PyO3 现状事实（已核实）

### 2.1 版本与 API

- 最新 **0.30.1**（2026-10-14 发版），MSRV 1.85（本工程 rust-version 1.90 ✓）；0.29 线并行维护。锁死 `pyo3 = "=0.30.1"`（oracledb beta 同款惯例）。
- 生命周期 API（docs.rs 0.30.1 核实）：`Python::initialize()`（信号处理默认禁用——正合 GUI 进程需求，no-op 幂等）、`Python::attach`（`with_gil` 的改名）/ `try_attach` / `detach`（`allow_threads` 的改名）。`Python<'py>` 非 Send——专用宿主线程是自然形态（v8 isolate 同款约束同款解法）。
- **无 config 化的初始化 API**：0.30 没有 `initialize_with(config)` 之类入口；自定义 PyConfig（PYTHONHOME、isolated 等）需在首次 `attach` 前直接走 `pyo3::ffi::Py_InitializeFromConfig`（PyO3 官方支持这条嵌入惯例，ffi 层就是为此暴露的）。
- `auto-initialize` feature：首次使用自动初始化；**static-only 安装上被构建期刻意拒绝**（build.rs 检出后报错建议禁用，来源 1）——我们本就走手动初始化，无影响。

### 2.2 链接形态（本方案的关键判断点）

- pyo3-build-config 按目标解释器安装自动选**静态/动态**链接（有无共享库决定）；`PYO3_PYTHON`（指向 PBS 安装的解释器）、`PYO3_CONFIG_FILE`（喂手写配置，可强制 `shared=false` 走静态链）、`PYO3_PRINT_CONFIG=1`（打印解析结果）。
- **静态嵌入是「能用但次等」**：first-class 支持在 [PyO3 issue #416](https://github.com/PyO3/pyo3/issues/416)（2019 年开至今）。官方文档列出的已知坑：加载扩展模块需宿主导出符号（`-Wl,--export-dynamic`）、构建 libpython.a 的 C 编译器/旗标需与 Rust 工具链兼容（LTO/PIE 形态冲突案例）。
- **Windows 无静态路**：官方文档原话「on Windows static linking is almost never done, so Python distributions don't usually include a static library」。
- **运行期加载与 PyO3 互斥**：pyo3-ffi 无 bind_to_library 分支；odbc_engine 式手写 dlopen（21 个入口）对 CPython 不成立——嵌入执行（配置/初始化/对象协议/异常/导入/unicode/capsule…）所需符号面大一个数量级，且手写 FFI 即放弃 PyO3 的全部安全封装，等于「引入 PyO3 方案」自我否定。**因此本调研把问题从「能否运行期加载」改写为「链接期形态里如何让缺运行时不拖累进程」**（§4.1）。

### 2.3 free-threaded（3.14t+）

- PyO3 0.30 完整支持 `Py_GIL_DISABLED` 构建（config 输出 `lib_name=python314t`、`build_flags=Py_GIL_DISABLED`）；abi3t（PEP 803 稳定 ABI）已立。
- PBS 四腿均有 `+freethreaded` 资产（§3.1 核实）。**P0 不选**：单宿主线程模型下 GIL 足够，三方生态的 FT ABI 兼容仍在收敛；记为 P3 议题。

## 3. CPython 分发来源：python-build-standalone（已核实）

### 3.1 基本盘与四腿资产

- astral 维护（2024-12 接管，uv 同源），GitHub Releases 近乎日更；核实最新 tag **20261026**，Python 版本 3.11 / 3.12 / 3.13 / 3.14 / 3.15 / 3.16（3.16 为 `3.16.0a1` alpha 轨道，不选）。
- **推荐锁 3.14 线**（当前 3.14.10，第二位 patch、生态全面跟进；3.15.2 已出但生态追赶中）。
- 四腿资产核对（3.14.10，install_only_stripped 压缩包实测大小，GitHub API）：

| 发布腿 | 资产 | gz 体积 |
| --- | --- | --- |
| Windows x64 | `...x86_64-pc-windows-msvc-install_only_stripped.tar.gz` | 23.3 MB |
| Windows ARM64 | `...aarch64-pc-windows-msvc-install_only_stripped.tar.gz` | 22.3 MB |
| macOS x64 | `...x86_64-apple-darwin-install_only_stripped.tar.gz` | 26.9 MB |
| macOS ARM64 | `...aarch64-apple-darwin-install_only_stripped.tar.gz` | 26.8 MB |

  解压后为完整可执行发行（python + stdlib + DLL），约 90–120 MB 量级（PoC 实测）。未 strip 的 Windows 包 ~48 MB（含调试符号）——一律取 stripped。
- 变体语义：`install_only*` = 可执行发行（无开发产物）；`*-full.tar.zst` = 附 `install/` 开发产物（头文件、静态库、导入库等）；优化变体 `pgo+lto`（默认）与 `debug`。

### 3.2 静态库的平台覆盖（硬边界）

- `+static-full` 变体**只有 Linux musl**（全部扩展 builtin；官方 quirk：无法加载编译型扩展模块）。**Windows 与 macOS 无 `+static` 资产**（GitHub API 全量核对）。
- **macOS 完整包含 libpython 静态库**：官方 quirks 文档设有「macOS 静态库链接」专节——静态链 libpython 需要 libclang_rt 的 `___isOSVersionAtLeast`（Python 3.10+ 的 `__builtin_available`），**档案内附一份 libclang_rt.a**。同时 PyTauri 教程证实 macOS 发行含 dylib（其 install_name 缺 `@rpath` 需 install_name_tool 修补）。.a 与 .dylib 在 full 档案中的具体共存形态与 stdlib 扩展是 builtin 还是 lib-dynload `.so`，为 **PoC 首项核对内容**（§9.1）。
- **Windows 只有动态形态**：`python314.dll` + `Lib/` + `DLLs/`（.pyd 扩展模块）+ 导入库 `libs/python314.lib`（full 档案内，PoC 核对）。quirks：不带 `pip.exe`（用 `python -m pip`）。

### 3.3 许可与供应链

- Python 本体 **PSF-2.0**（宽松，类 BSD）；捆绑第三方全 permissive：OpenSSL（Apache-2.0）、libffi（MIT）、zlib/bzip2/xz/sqlite/mpdecimal/libedit 等；**刻意以 libedit 替代 readline 规避 GPL**。工具仓库本身 MPL-2.0，与产物分发无关。→ **无 libjxl/nmap/agg 式商用确认负担**；随包需附第三方许可清单（安装包 resources 放 `python-licenses/`，PoC 定）。
- 锁版本：同时钉 PBS tag 与 CPython 版本（如 `3.14.10+20261026`），CI 下载脚本钉 SHA256（rusty_v8 的 `RUSTY_V8_ARCHIVE_SHA256` 同款供应链思路）；升级 = 换 tag + 人工评估（Cargo.toml 注释惯例）。

## 4. 与工程铁律的对照

### 4.1 「运行期加载」铁律的辨析（关键判断）

铁律打击的是**链接期动态链接的绑定 crate**（缺库 = 构建失败 + 进程启动硬依赖）。PyO3 满足不了铁律的字面形态（§2.2），因此问题改写为：**链接期形态下，如何让「Python 运行时缺失」不拖累进程启动与其它功能**。三条路径：

| 路径 | macOS | Windows | 铁律相容性 |
| --- | --- | --- | --- |
| A. 全静态链入 | ✅ PBS full 的 .a（pdfium macOS ARM64 静态先例 + rusty_v8 同型） | ❌ 无官方静态库，不可行 | 完全相容（启动零依赖） |
| B. 链接期动态 + 随包 DLL/dylib | ⚠️ ARM64 未签名 dylib 拒载（pdfium「静态化是根治、严禁改」同源问题）；需 ad-hoc 签名破先例 | ✅ 自然形态（PyTauri 即此路线） | 字面冲突：缺库 = 进程启动失败 |
| C. 动态 + /DELAYLOAD 延迟加载 | —（macOS 无 delayload 机制，不走此路） | ✅ /DELAYLOAD + 延迟钩子 | 首用软依赖：缺 DLL 优雅降级 ✅ |

**推荐平台分流**：
- **macOS 双腿 = 路径 A**：`PYO3_CONFIG_FILE` 强制 `shared=false` 静态链 `libpython3.14.a`（+ libclang_rt.a），stdlib 以资源文件随包。零运行时分发、无签名议题、v8 同型。若 PoC 发现 PBS macOS 的 stdlib 扩展是 lib-dynload `.so`（非 builtin），则需随包 `.so` 并给最终链接加 `-Wl,-export_dynamic`（PyO3 文档的静态嵌入标准操作）——PoC 验证。
- **Windows 双腿 = 路径 C**：导入库链接 + `/DELAYLOAD:python314.dll` + 覆写 `__delayLoadHelper2` 钩子（先按 `dll_paths` 候选路径 LoadLibrary 安装根 `lib\python314.dll`，失败回落标准搜索）。效果等价「运行期加载」：DLL 被 AV 误删/安装残缺时应用照常启动，首次用 Python 才报带安装指引的中文错误（ODBC 缺管理器文案同型）。nmap 三件套的 /DELAYLOAD:wpcap.dll 是直接先例。**PoC 必验**：delayload 与 pyo3-build-config 链接产物（raw-dylib 生成 vs 真 .lib）的组合是否成立（必要时 `PYO3_USE_RAW_DYLIB=0`）。
- stdlib 两平台都是**资源文件**（静态 libpython 只含解释器 + 内建模块，纯 Python stdlib 必须随包）：走 tauri bundle resources；macOS 可评估 zipimport（stdlib 打单个 zip 进一步压体积），Windows 的 `DLLs/*.pyd` 必须平铺真文件。资源目录复用 `dll_paths` 的 flat_dir_candidates 约定（nmap-data/vim-runtime 同型）。

### 4.2 workspace 放置

- 放 **app crate**（新模块 `src-tauri/src/python_runtime/`，与 v8 调研规划的 js_runtime 对称），不放 coreutils（脚本不是 CLI 命令）；relay 独立 workspace 不受影响。`pyo3 = "=0.30.1"` 锁死 + 注释写明升级约定。
- PBS 产物**不入 vendor 树**（full 档案上百 MB，vendor 惯例是源码树）：构建机经脚本下载钉哈希到 git-ignored 目录（`scripts/` 新增下载脚本或 build-tools.mjs 新段），`PYO3_PYTHON`/`PYO3_CONFIG_FILE` 指过去。本机 macOS 开发一次下载长期复用（rusty_v8 的 ~/.cargo 缓存同思想）。

### 4.3 门禁与 CI 影响

| 场景 | 影响 | 对策 |
| --- | --- | --- |
| 本机 `cargo check/test -p app --lib` | 需先跑下载脚本放 PBS（一次性） | 脚本钉 tag+SHA256；README/AGENTS.md 记步骤 |
| 本机 clippy / rust-analyzer | 零额外负担（无构建期大下载链，区别于 v8 crate 的 build.rs 自动下载） | — |
| publish 四腿 | mac 腿需 full 档案（.a + include + libclang_rt），win 腿需 install + 导入库 | publish.yml 新增下载步骤钉 SHA256 + actions/cache |
| 资源随包 | stdlib 树（+ win 的 DLL/）进 bundle.resources | 对齐 nmap-data/vim-runtime 的 staging 惯例（verify-installer-layout.ps1 加核对项） |
| 测试基线 | 新增 python_runtime 单测 | 落地时更新 AGENTS.md 基线 |

### 4.4 clippy 约束

app crate `panic = "deny"` + `indexing_slicing = "deny"` 对胶水层同样生效：FFI 边界错误走 Result 传播（CPython API 返回 NULL/-1 全部检查），数组访问 get 化——与 v8 方案 §3.4 同约定。

## 5. 执行架构设计（推荐方案）

### 5.1 初始化（进程级一次，首用懒初始化，永不 finalize）

1. 首次运行脚本时起常驻「Python 宿主线程」（OnceLock 单例，v8 方案同型）；
2. 该线程上经 `pyo3::ffi` 组 `PyConfig` 并 `Py_InitializeFromConfig`：
   - `home` = 随包 stdlib 根（资源目录解析出的绝对路径）；
   - `isolated` + `use_environment=0`——密封形态，屏蔽用户机器上的 `PYTHONHOME/PYTHONPATH` 等环境变量（oracle 双驱动「零环境依赖」同哲学）；
   - `install_signal_handlers=0`——GUI 进程不装 SIGINT 处理器；
   - `program_name` 设为应用名（sys.executable 语义兜底）；
3. 初始化后立即装桥：`sys.stdout`/`sys.stderr` 替换为回调对象（Rust 侧闭包 → Tauri Channel 流式，dbQuery 的 tag=type + camelCase 惯例）、`sys.path` 规整（stdlib 资源目录 + 应用脚本目录）；
4. **永不 `Py_Finalize`**：CPython 反复 init/finalize 与 C 扩展共存的历史不可靠，进程级单例随进程退出回收（v8 方案「不追求显式 dispose」同款）。

### 5.2 执行模型（每请求独立 globals，解释器进程级共享）

```
Tauri async command(python_run)
  ├─ 组装请求（源码、超时 ms、能力白名单）→ mpsc 发给 Python 宿主线程
  ├─ 看门狗：tokio::time::timeout 包住等待；超时 → PyErr_SetInterrupt（跨线程安全）
  └─ 宿主线程循环：
       Python::attach → 每请求新建 __main__ globals dict（__name__="__main__"）
       → exec/eval 分离（语句 vs 表达式求值）
       → stdout/stderr 桥经 Channel 流式回传
       → 未捕获异常取 traceback 结构化回收（类型/消息/栈帧）
       → 结果值序列化（JSON；不可序列化类型给占位串）
```

- **取消 = `PyErr_SetInterrupt`**：在脚本下一个字节码边界 raise `KeyboardInterrupt`，宿主捕获后转结构化「已超时/已取消」结果。**边界**：阻塞在 C 扩展或系统调用里不可及时打断（ODBC in-flight SQLFetch 同款已知边界）——超时兜底语义为「结果作废 + 警告」，进程不等待不崩溃。
- 单宿主线程 + GIL 下不做 `detach` 让出（无并发 Python 调用方）；P2 桥调用（ssh.exec 等）在宿主线程内同步阻塞等待 tokio 侧结果（v8 方案同款桥接形态）。
- 安全模型与 v8 调研 §5 完全一致：脚本 = 用户手敲 shell 同级的信任域，不承诺对宿主沙箱；能力白名单桥，不注入即不存在。

### 5.3 体积与启动

- macOS：静态 .a 链入二进制（增量 PoC 实测，量级参照 v8 的 +15–40 MB）+ stdlib 资源（压缩后 ~10–20 MB，PoC 实测）。
- Windows：`python314.dll`（~5–6 MB）+ `DLLs/*.pyd` + `Lib/` 资源树，安装包压缩后预计 **+20–25 MB**（install_only_stripped 资产 23.3 MB 佐证）。
- 启动：懒初始化，未用 Python 的会话零成本；delayload 使 Windows 启动期零 DLL 加载。

## 6. 与 V8 方案（rusty_v8）对照——决策表

| 维度 | V8（rusty_v8） | CPython（PyO3 + PBS） |
| --- | --- | --- |
| 链接形态 | 四腿全静态，v8 crate 自动下载，单一形态 | **平台不对称**：mac 静态 .a / win 动态 + delayload 变通 |
| 引擎分发 | 纯档案，无数据文件 | 解释器 + **stdlib 数据树随包**（两平台都要） |
| 体积 | +15–40 MB/平台（v8 调研估） | 同量级或略高（§5.3） |
| 脚本隔离 | 每执行新 isolate，零泄漏 | 进程级单解释器，sys.modules 跨脚本共享（明确降级） |
| 取消/超时 | terminate_execution 可杀 JIT 死循环 | 字节码边界 Interrupt；C 调用不可杀 |
| 内存上限 | CreateParams 堆上限 + 逼近回调 | **无硬上限**（结构性） |
| 语言/生态 | ES 语法（无 npm 运行面） | Python 标准库全量；三方包受限（§7.4） |
| 用户画像 | 前端背景 | **运维/DBA/数据**——veloterm 目标用户更亲和 |
| 铁律相容 | 完全同型 | Windows 腿需 delayload 变通守住原则 |
| 维护 churn | v8 crate 月更、锁死即稳 | pyo3 0.2x→0.30 API 已历 with_gil→attach 等改名；锁死即稳 |

**结论**：工程干净度 V8 全面占优（静态、隔离、可杀、可限额）；Python 换来的是用户语言亲和与标准库（json/re/sqlite3/csv…）即战力。若二选一，是产品判断题；宿主桥 trait 化（v8 调研 §7 的 engine-agnostic host 层）后双引擎可并存，代价是体积 ×2 与两套维护面。

## 7. 已知边界（明示清单）

1. **进程级单解释器**：`sys.modules` 与解释器全局状态跨脚本共享，脚本 A 污染可影响脚本 B（globals dict 每请求隔离只是部分缓解）。V8 方案每执行新 isolate 是明确更优。
2. **取消粒度 = Python 字节码边界**：纯 Python 死循环可打断；C 扩展/长阻塞系统调用内不可中断，超时只作废结果。
3. **无堆硬上限**：内存炸弹（大列表/大 bytes）无法像 V8 顶死回收；`sys.setrecursionlimit` 只管递归。终极隔离只有 sidecar 进程形态（§8）。
4. **pip / 三方 C 扩展平台不对称**：Windows 动态形态语义上是「真 Python 安装」，`pip --target <app-data>` + sys.path 注入可行；macOS 静态形态宿主不导出符号（除非 -export_dynamic + PoC 证明），三方 wheel 基本不可用（PyOxidizer 同限）。P0–P2 **不承诺 pip**；若 pip 成为硬需求 → 直接转 sidecar 形态。
5. **GIL 串行**：脚本执行全局互斥（本地胶水场景可接受；free-threaded 为 P3 议题）。
6. **崩溃面**：CPython 本体 bug 或（未来）三方扩展段错误直接击穿进程——信任域内接受，与 v8 同级风险。
7. **Windows AV 误报 `python314.dll`** 是常见运营事件：delayload 消除其后果（应用不死）但不消除概率；文案需指引重装/加白名单。
8. **stdlib 版本漂移**：锁 PBS tag 即锁 stdlib；用户脚本依赖新版本特性时随整体升级走。

## 8. 备选形态与退出条件

| 形态 | 说明 | 取舍 |
| --- | --- | --- |
| **PyO3 链接期嵌入（本文主推）** | mac 静态 + win delayload 动态 | 宿主桥（P2）深度集成的唯一路径；隔离/取消/内存三边界接受 |
| sidecar 子进程 | 随包 `python.exe`（PBS install），spawn + stdio/RPC 协议，无 PyO3 | 进程隔离/可硬杀/pip 与 C 扩展全支持；宿主桥经协议重建（复杂度转移）；**退出条件触发时的平移目标，也是 pip 硬需求的正解** |
| RustPython（纯 Rust） | 见下方专段 | 工程形态全面优于 CPython 路线，但语言真实性打折——**判据是「脚本谁写」**，见专段 |
| 继续 V8 路线 | 见姊妹篇 | 产品层接受 JS 时的首选（工程干净度最优） |

**退出条件**：① macOS 静态链接 PoC 失败（libclang_rt/LTO/PIE 不可解）；② Windows delayload 与 PyO3 链接产物组合不可行且「整包动态」不被接受 → 转 sidecar（宿主 trait 与产品层不变）。

**RustPython 专段（2026-10 补充核实）**：工程形态上它全面优于 CPython 路线——纯 Rust 零 C 依赖，§4 的链接形态问题整个消失（四腿同型静态链入、无 PBS 分发、无 delayload、无 AV 误报、无未签名 dylib 议题，铁律天然满足）；`freeze-stdlib` feature（rustpython-pylib crate）把 stdlib 字节码编进二进制，**零运行时数据文件**（CPython 路线两平台都要随包 stdlib 树）；`Interpreter` 支持进程内多实例，**每执行新建解释器**、可干净 drop（CPython finalize 不可靠的问题不存在），隔离性追平 V8 isolate。已知 caveat：解释器隔离非完美（#8765：ContextVar 走线程本地存储，同线程多解释器会泄漏——专用线程规避）；asyncio 在嵌入模式有问题（#4699）；无 GIL 全局锁但实例间线程本地态需注意。**但代价在产品侧**：stdlib 覆盖不完整（官方 `scripts/whats_left.py` 可量化缺口；对齐 CPython 3.12/3.14 的移植仍在逐模块进行，#5104/#6839）、无 C 扩展（maturin 不识别）、性能慢约一个量级——「内置 Python」的用户预期是 CPython 行为，运维/DBA 按真 Python 心智写脚本会以微妙方式失败，形成支持负担。**判据：脚本主要用户自写、预期「这就是 Python」→ CPython 主选不变；脚本主要是官方内置/模板、用户只运行微调 → RustPython 反转胜出（此时它不是 Boa 位而是 quickjs 位的加强版：体积小、隔离好、零数据文件）**。若考虑此路线，第一步 PoC 就是拿 `whats_left.py` 对照目标用户常用模块清单（json/re/csv/sqlite3/ssl/subprocess/asyncio…），覆盖率不可接受即一票否决。

## 9. PoC 计划（P0）与验收门禁

范围：`src-tauri/src/python_runtime/` 模块 + `pyo3 = "=0.30.1"` + PBS 下载脚本，**无前端改动**。

1. **PBS full 档案布局核对**（mac 双腿 + win x64）：.a/.dylib/include/libclang_rt 共存形态、stdlib 扩展 builtin vs lib-dynload、Windows 导入库路径——决定 §4.1 的配置细节；
2. **macOS 静态链接**：`PYO3_CONFIG_FILE`（shared=false）+ fat LTO + panic=abort + strip 的 release 构建成功，记录体积/链接耗时前后对比；
3. **初始化**：PyConfig 手动初始化幂等；sealed（外部 PYTHONHOME/PYTHONPATH 免疫）；stdlib 从资源目录加载（`json`/`re`/`sqlite3` 可导入）；
4. **执行**：`print(1+1)`、中文字符串往返、`json.dumps` 中文无损；
5. **stdout/stderr 桥**：顺序保持、stdout/stderr 分级正确；
6. **结果与异常序列化**：dict/list/None/嵌套结构、`ZeroDivisionError` 带 traceback 结构化回收；
7. **取消**：`while True: pass` 在 500 ms 看门狗下转结构化超时结果；取消后宿主线程仍可服务下一次执行；
8. **Windows delayload 双态**：DLL 在位全流程冒烟 + DLL 缺位应用照常启动、Python 功能报中文指引错误（可先借 tag 车道 `test_python` 提前暴露）。

门禁（AGENTS.md 局部口径）：`cargo test -p app --lib python_runtime` 全绿 + `cargo clippy -p app --all-targets` 零警告 + `cargo build -p app --release` 成功并记录体积/链接耗时对比。PoC 通过后 P1 立项时同步更新 AGENTS.md（能力条目 + 基线 + 边界）。

## 10. 来源

- [PyO3 Building and Distribution（链接形态/auto-initialize/静态嵌入坑/PYO3_* 环境变量）](https://pyo3.rs/main/building-and-distribution) · [PyO3 FAQ](https://pyo3.rs/main/faq)
- [PyO3 issue #416 — Static Build of pyo3（2019 年至今的静态嵌入跟踪）](https://github.com/PyO3/pyo3/issues/416)
- [docs.rs pyo3 0.30.1 — Python 生命周期 API（initialize/attach/detach）](https://docs.rs/pyo3/latest/pyo3/marker/struct.Python.html) · [crates.io pyo3（版本与 MSRV，API 核实）](https://crates.io/crates/pyo3)
- [astral-sh/python-build-standalone 仓库与文档](https://github.com/astral-sh/python-build-standalone) · [quirks（macOS 静态库链接/libclang_rt、libedit、Windows 无 pip.exe）](https://docs.astral.sh/python-build-standalone/quirks/) · [releases tag 20261026 资产清单与体积（GitHub API 核实）](https://github.com/astral-sh/python-build-standalone/releases)
- [PyTauri standalone 构建教程（Tauri + PyO3 + PBS 的动态链接先例、macOS install_name 修补、tauri dev 资源隔离坑）](https://pytauri.github.io/pytauri/0.8/usage/tutorial/build-standalone)
- [rye #1050 — PBS 标准构建的 libpython 形态讨论](https://github.com/astral-sh/rye/issues/1050) · [Astral 接管 PBS 的公告](https://astral.sh/blog/python-build-standalone)
- [RustPython 现状（兼容面/C 扩展缺失/maturin 不识别）](https://github.com/RustPython/RustPython)
- RustPython 补充核实：[rustpython-pylib freeze-stdlib（stdlib 字节码内嵌）](https://docs.rs/rustpython-pylib) · [#8765 ContextVar 跨解释器泄漏（隔离 caveat）](https://github.com/RustPython/RustPython/issues/8765) · [#4699 asyncio 嵌入模式问题](https://github.com/RustPython/RustPython/discussions/4699) · [#5104/#6839 stdlib 对齐 3.12/3.14 的移植跟踪](https://github.com/RustPython/RustPython/issues/5104)（覆盖率用其 `scripts/whats_left.py` 量化）
- 姊妹篇：`docs/v8-js-runtime-research.md`（rusty_v8 路线，宿主桥 trait 与产品分期共用）
