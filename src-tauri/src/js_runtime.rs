//! JS 脚本运行时（V8 / rusty_v8）——P0 同步执行层。
//!
//! 执行模型（调研与决策记录见 docs/v8-js-runtime-research.md）：
//! - 进程内单例「JS 宿主线程」：首次运行脚本时惰性启动，V8 平台在该线程上
//!   初始化一次。非主线程初始化必须用 `new_unprotected_default_platform`
//!   （rusty_v8 README 给出的 PKU 崩溃规避，放弃 V8 JIT 页的 PKU 硬隔离——
//!   本地脚本属用户信任域，与手敲 shell 同级）。
//! - 每次执行新建独立 isolate（`CreateParams::heap_limits` 收紧堆上限 + console
//!   状态槽），跑完即弃：脚本间零状态泄漏，新建成本 ms 级。
//! - 双通道终止：调用方超时经 `IsolateHandle` 跨线程 `terminate_execution`
//!   （V8 在循环回边/函数入口检查终止标志，死循环可杀）；堆触顶经
//!   near-heap-limit 回调走同一路径——该回调**没有 isolate 参数**，经 data
//!   指针携带 IsolateHandle，并抬升上限给终止展开留余量。
//! - P0 仅同步脚本：无 timer/Promise 事件循环（微任务在脚本返回后经
//!   `perform_microtask_checkpoint` 一次清空）；异步/ESM 是后续阶段议题
//!   （届时重新评估 deno_core，见调研文档 §4.3）。
//! - P1 命令层：`js_run`（console 逐行经 Tauri Channel 流式 + 终态事件，
//!   dbQuery/fleet_exec 同款 tag=type 模式）与 `js_run_cancel`（运行注册表 +
//!   取消旗标，db_query_cancel 同型）。取消与超时在宿主侧不可区分（都是外部
//!   终止），由命令等待方按取消旗标改判 Cancelled。
//! - P1.5 TypeScript：oxc 后端转译（transpile-only，见 ts_transpile.rs 模块头）；
//!   异常消息带「TS 第 L 行第 C 列」（sourcemap 回映，JS 则「第 L 行」），
//!   console 行带调用者位置（栈顶帧回映，格式 "L:C"）。
//!
//! 本函数为阻塞式（等待至多 timeout）：P1 的 tauri 命令需 spawn_blocking 包裹。
//! 已知边界：终止只作用于 JS 执行期；若调用方在等待中途放弃且脚本为死循环，
//! 宿主线程将被占用直至有人再次发起并触发终止（P0 由测试纪律约束）。
//!
//! 静态库形态：四平台 release 档案 vendor 于 src-tauri/vendor/rusty_v8/，
//! 经 .cargo/config.toml 的 RUSTY_V8_ARCHIVE 构建期零下载（见该目录 README）。

use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use oxc_sourcemap::SourceMap;
use serde::Serialize;

mod ts_transpile;

/// TS sourcemap 的共享形态：isolate 槽（console 回映）与错误路径（异常回映）
/// 各持一份 Arc
type SharedTsMap = Arc<SourceMap<'static>>;

/// 默认脚本执行超时
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
/// 默认 JS 堆上限（MB）
pub const DEFAULT_HEAP_LIMIT_MB: usize = 256;
/// 堆触顶回调的上限抬升量：给终止展开（GC / 回边检查）留余量
const HEAP_LIMIT_BUMP: usize = 64 * 1024 * 1024;
/// 调用方触发终止后等待宿主回包的总宽限
const TERMINATION_GRACE: Duration = Duration::from_secs(10);
/// 宽限期内重试终止信号的间隔（isolate 可能尚未建好、句柄槽还空）
const TERMINATION_RETRY: Duration = Duration::from_millis(200);
/// 宿主线程栈：V8 的 JS 调用栈消耗宿主线程栈空间，Rust 默认 2MB 偏紧
const HOST_STACK_SIZE: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ConsoleLevel {
    Log,
    Warn,
    Error,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConsoleLine {
    pub level: ConsoleLevel,
    pub text: String,
    /// 调用者位置 "L:C"（TS 经 sourcemap 回映到原始行列；栈不可用时为 None）
    pub position: Option<String>,
}

#[derive(Debug, Clone)]
pub struct JsRunOptions {
    pub timeout: Duration,
    pub heap_limit_mb: usize,
    /// 脚本语言（Typescript 时先经 oxc 转译再执行，见 ts_transpile.rs）
    pub language: JsLanguage,
}

impl Default for JsRunOptions {
    fn default() -> Self {
        Self {
            timeout: DEFAULT_TIMEOUT,
            heap_limit_mb: DEFAULT_HEAP_LIMIT_MB,
            language: JsLanguage::Javascript,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum JsLanguage {
    #[default]
    Javascript,
    Typescript,
}

#[derive(Debug, Clone)]
pub struct JsRunOutcome {
    /// 脚本运行期经 console 桥收集的输出（保持顺序）
    pub console: Vec<ConsoleLine>,
    /// 末表达式值的 JSON 序列化；undefined 为 None；JSON 不可表达的值
    /// （函数等）退回 toString 文本
    pub result_json: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum JsRunError {
    /// 超时被调用方看门狗终止（死循环等）
    Timeout { elapsed_ms: u128 },
    /// JS 堆触顶被终止（分配风暴等）
    HeapLimit,
    /// 语法错误或运行时异常
    Script { message: String },
    /// 调用方主动取消（宿主侧与超时同型，由等待方按取消旗标改判）
    Cancelled,
    /// 宿主线程/通道不可用（初始化失败、通道断开）
    HostUnavailable(String),
}

impl std::fmt::Display for JsRunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Timeout { elapsed_ms } => write!(f, "脚本执行超时（{elapsed_ms} ms）已被终止"),
            Self::HeapLimit => write!(f, "脚本内存超限（JS 堆触顶）已被终止"),
            Self::Script { message } => write!(f, "脚本错误: {message}"),
            Self::Cancelled => write!(f, "脚本运行已被取消"),
            Self::HostUnavailable(msg) => write!(f, "JS 运行时不可用: {msg}"),
        }
    }
}

impl std::error::Error for JsRunError {}

/// 同步执行一段 JS 脚本（阻塞至多 timeout；无流式、无取消）。
pub fn run_script(source: &str, options: &JsRunOptions) -> Result<JsRunOutcome, JsRunError> {
    run_script_with_sink(source, options, None, None)
}

/// console 流式出口：宿主线程内逐行调用（Arc 闭包需 Send+Sync 以随 Job 跨线程）；
/// 第三参为回映后的调用者位置 "L:C"
pub type ConsoleSink = Arc<dyn Fn(ConsoleLevel, &str, Option<String>) + Send + Sync>;

/// 完整执行入口：console 逐行流式出口 + 调用方取消旗标（js_run 命令层使用）。
pub fn run_script_with_sink(
    source: &str,
    options: &JsRunOptions,
    sink: Option<ConsoleSink>,
    cancel: Option<Arc<AtomicBool>>,
) -> Result<JsRunOutcome, JsRunError> {
    let host = host()?;
    let (reply_tx, reply_rx) = channel();
    let handle_slot: Arc<Mutex<Option<v8::IsolateHandle>>> = Arc::new(Mutex::new(None));
    host.tx
        .send(Job {
            source: source.to_string(),
            heap_limit_mb: options.heap_limit_mb,
            language: options.language,
            reply: reply_tx,
            handle_slot: handle_slot.clone(),
            sink,
        })
        .map_err(|_| JsRunError::HostUnavailable("JS 宿主通道已关闭".into()))?;

    let cancelled = || cancel.as_ref().is_some_and(|c| c.load(Ordering::SeqCst));
    let deadline = Instant::now() + options.timeout.max(Duration::from_millis(50));
    let outcome = loop {
        let now = Instant::now();
        if now >= deadline || cancelled() {
            break None;
        }
        // 小步轮询（200ms 粒度）：取消旗标在等待中途也能及时生效——否则一个
        // 长超时的死循环 job 会占住宿主线程直到 deadline，堵死后续所有执行
        let wait = (deadline - now).min(TERMINATION_RETRY);
        match reply_rx.recv_timeout(wait) {
            Ok(reply) => break Some(reply),
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => {
                return Err(JsRunError::HostUnavailable("JS 宿主线程已退出".into()));
            }
        }
    };

    let result = match outcome {
        Some(reply) => reply,
        None => {
            // 超时/取消：跨线程终止（宿主侧按「guard 未命中」分类为 Timeout）。
            // isolate 可能尚未建好（句柄槽空）——宽限期内周期重试终止信号。
            let grace_end = Instant::now() + TERMINATION_GRACE;
            'grace: loop {
                if let Some(handle) = handle_slot.lock().ok().and_then(|s| s.clone()) {
                    handle.terminate_execution();
                }
                let wait = grace_end.saturating_duration_since(Instant::now());
                if wait.is_zero() {
                    break 'grace Err(JsRunError::HostUnavailable(
                        "JS 执行终止后未收到回包".into(),
                    ));
                }
                match reply_rx.recv_timeout(wait.min(TERMINATION_RETRY)) {
                    Ok(reply) => break 'grace reply,
                    Err(RecvTimeoutError::Timeout) => continue,
                    Err(RecvTimeoutError::Disconnected) => {
                        break 'grace Err(JsRunError::HostUnavailable(
                            "JS 宿主线程已退出".into(),
                        ));
                    }
                }
            }
        }
    };
    // 外部终止在宿主侧统一分类为 Timeout：取消旗标已置时改判 Cancelled
    if cancelled() && matches!(result, Err(JsRunError::Timeout { .. })) {
        return Err(JsRunError::Cancelled);
    }
    result
}

// ── 宿主线程（进程单例） ─────────────────────────────────────────────

struct Job {
    source: String,
    heap_limit_mb: usize,
    language: JsLanguage,
    reply: Sender<Result<JsRunOutcome, JsRunError>>,
    /// 调用方看门狗跨线程终止用的句柄槽（isolate 建好后由宿主填充）
    handle_slot: Arc<Mutex<Option<v8::IsolateHandle>>>,
    /// console 流式出口（逐行转发给 UI 的 Channel 适配器）
    sink: Option<ConsoleSink>,
}

struct Host {
    tx: Sender<Job>,
}

static HOST: OnceLock<Result<Host, String>> = OnceLock::new();

fn host() -> Result<&'static Host, JsRunError> {
    let init = HOST.get_or_init(|| {
        let (tx, rx) = channel::<Job>();
        match std::thread::Builder::new()
            .name("js-runtime-host".to_string())
            .stack_size(HOST_STACK_SIZE)
            .spawn(move || host_entry(rx))
        {
            Ok(_) => Ok(Host { tx }),
            Err(e) => Err(format!("JS 宿主线程启动失败: {e}")),
        }
    });
    init.as_ref()
        .map_err(|msg| JsRunError::HostUnavailable(msg.clone()))
}

fn host_entry(rx: Receiver<Job>) {
    // V8 平台初始化（进程级一次，之后不可逆）；平台随进程存续、不显式 dispose。
    // 执行体刻意不引入 panic 路径（工程 clippy deny panic）——catch_unwind
    // 跨 V8 C++ 帧展开不可靠，以纪律代替兜底。
    let platform = v8::new_unprotected_default_platform(0, false).make_shared();
    v8::V8::initialize_platform(platform);
    v8::V8::initialize();
    for job in rx {
        execute_job(job);
    }
}

fn execute_job(job: Job) {
    let started = Instant::now();
    // 禁用项预拦截（模块语义解析 + AST 遍历，见 ts_transpile.rs）：async/
    // await、事件循环类/网络/Node API 在纯 V8 同步沙箱里要么不存在、要么
    // 存在也永不触发——统一换成带位置与原因的中文报错；Promise 链（微任务）
    // 仍在脚本返回后清空，不受影响
    if let Some(found) = ts_transpile::detect_forbidden(
        &job.source,
        matches!(job.language, JsLanguage::Typescript),
    ) {
        let _ = job.reply.send(Err(JsRunError::Script {
            message: found.message(),
        }));
        return;
    }
    // TS 先转译（oxc）：失败直接回错误（诊断自带原始行列）；JS 原样执行。
    // 转译在宿主线程内做（脚本规模小，微秒级），sourcemap 随后用于
    // 异常与 console 位置回映
    let (js_source, ts_map): (String, Option<SharedTsMap>) = match job.language {
        JsLanguage::Javascript => (job.source.clone(), None),
        JsLanguage::Typescript => match ts_transpile::transpile(&job.source) {
            Ok(t) => (t.code, Some(Arc::new(t.map))),
            Err(message) => {
                let _ = job.reply.send(Err(JsRunError::Script { message }));
                return;
            }
        },
    };
    let heap_max = job.heap_limit_mb.clamp(8, 4096) * 1024 * 1024;
    let heap_initial = heap_max / 4;
    let params = v8::Isolate::create_params().heap_limits(heap_initial, heap_max);
    let mut isolate = v8::Isolate::new(params);
    let handle = isolate.thread_safe_handle();
    if let Ok(mut slot) = job.handle_slot.lock() {
        *slot = Some(handle.clone());
    }

    // 堆触顶守卫：near-heap-limit 回调无 isolate 参数，经 data 指针携带句柄
    let guard = HeapGuard {
        handle: Some(handle),
        hit: AtomicBool::new(false),
    };
    isolate.add_near_heap_limit_callback(
        near_heap_limit_callback,
        std::ptr::from_ref(&guard).cast::<c_void>().cast_mut(),
    );
    isolate.set_slot(ConsoleState {
        lines: Vec::new(),
        sink: job.sink.clone(),
        ts_map: ts_map.clone(),
    });

    let reply: Result<JsRunOutcome, JsRunError> = 'run: {
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);

        // console 桥（log/warn/error 三态共用收集槽）
        let global = context.global(scope);
        let console_tmpl = v8::ObjectTemplate::new(scope);

        let log_fn = v8::FunctionTemplate::new(scope, console_log_cb);
        let warn_fn = v8::FunctionTemplate::new(scope, console_warn_cb);
        let err_fn = v8::FunctionTemplate::new(scope, console_error_cb);

        if let Some(k) = v8::String::new(scope, "log") {
            console_tmpl.set(k.into(), log_fn.into());
        }
        if let Some(k) = v8::String::new(scope, "warn") {
            console_tmpl.set(k.into(), warn_fn.into());
        }
        if let Some(k) = v8::String::new(scope, "error") {
            console_tmpl.set(k.into(), err_fn.into());
        }

        if let (Some(console_obj), Some(name_key)) =
            (console_tmpl.new_instance(scope), v8::String::new(scope, "console"))
        {
            let _ = global.set(scope, name_key.into(), console_obj.into());
        }

        let Some(source_str) = v8::String::new(scope, js_source.as_str()) else {
            break 'run Err(JsRunError::Script {
                message: "脚本源码超出 V8 字符串长度上限".into(),
            });
        };

        v8::tc_scope!(let scope, scope);

        let script = match v8::Script::compile(scope, source_str, None) {
            Some(s) => s,
            None => {
                if scope.has_terminated() {
                    break 'run Err(classify_termination(&guard, started));
                }
                let msg_local = scope.message();
                let mut message = msg_local
                    .map(|m| m.get(scope).to_rust_string_lossy(scope))
                    .unwrap_or_else(|| "脚本编译失败".into());
                if let Some(m) = msg_local {
                    message.push_str(&position_suffix(
                        m.get_line_number(scope),
                        m.get_start_column(),
                        ts_map.as_deref(),
                    ));
                }
                break 'run Err(JsRunError::Script { message });
            }
        };

        let result = script.run(scope);
        if scope.has_terminated() {
            break 'run Err(classify_termination(&guard, started));
        }
        let Some(value) = result else {
            let msg_local = scope.message();
            let mut message = msg_local
                .map(|m| m.get(scope).to_rust_string_lossy(scope))
                .or_else(|| {
                    scope
                        .exception()
                        .map(|e| e.to_rust_string_lossy(scope))
                })
                .unwrap_or_else(|| "脚本执行失败".into());
            if let Some(m) = msg_local {
                message.push_str(&position_suffix(
                    m.get_line_number(scope),
                    m.get_start_column(),
                    ts_map.as_deref(),
                ));
            }
            break 'run Err(JsRunError::Script { message });
        };

        // 微任务收尾：同步脚本的 Promise 链在此一次清空（P0 无事件循环）
        scope.perform_microtask_checkpoint();
        if scope.has_terminated() {
            break 'run Err(classify_termination(&guard, started));
        }

        let result_json = if value.is_undefined() {
            None
        } else {
            match v8::json::stringify(scope, value) {
                Some(s) => Some(s.to_rust_string_lossy(scope)),
                None => Some(value.to_rust_string_lossy(scope)),
            }
        };

        let console = scope
            .get_slot_mut::<ConsoleState>()
            .map(|st| std::mem::take(&mut st.lines))
            .unwrap_or_default();
        break 'run Ok(JsRunOutcome { console, result_json });
    };

    // isolate 弃置前解除回调，避免 data 指针悬垂（guard 即将离开作用域）
    isolate.remove_near_heap_limit_callback(near_heap_limit_callback, heap_max);
    let _ = job.reply.send(reply);
}

fn classify_termination(guard: &HeapGuard, started: Instant) -> JsRunError {
    if guard.hit.load(Ordering::SeqCst) {
        JsRunError::HeapLimit
    } else {
        JsRunError::Timeout {
            elapsed_ms: started.elapsed().as_millis(),
        }
    }
}

/// 错误消息的位置后缀：TS 经 sourcemap 回映为「TS 第 L 行第 C 列」，
/// JS（或回映未命中）为「第 L 行」。入参：Message 的行 1-based / 列 0-based。
fn position_suffix(
    line: Option<usize>,
    column: usize,
    ts_map: Option<&SourceMap<'static>>,
) -> String {
    let Some(line) = line.filter(|l| *l > 0) else {
        return String::new();
    };
    if let Some((tl, tc)) = ts_map.and_then(|m| ts_transpile::map_position(m, line, column)) {
        return format!("（TS 第 {tl} 行第 {tc} 列）");
    }
    format!("（第 {line} 行）")
}

// ── V8 回调 ──────────────────────────────────────────────────────────

/// isolate 槽里的 console 收集状态（宿主线程单线程访问）：
/// lines 供终态结果携带，sink 供运行期逐行流式转发，ts_map 供位置回映
#[derive(Default)]
struct ConsoleState {
    lines: Vec<ConsoleLine>,
    sink: Option<ConsoleSink>,
    ts_map: Option<SharedTsMap>,
}

/// near-heap-limit 回调的 data 载荷：句柄 + 命中标记
struct HeapGuard {
    handle: Option<v8::IsolateHandle>,
    hit: AtomicBool,
}

/// 堆逼近上限：置命中标记、请求终止执行、抬升上限给终止展开留余量。
/// 终止标志置位后 JS 在最近的循环回边停下；若分配持续，回调可再次触发继续抬升。
unsafe extern "C" fn near_heap_limit_callback(
    data: *mut c_void,
    current_heap_limit: usize,
    _initial_heap_limit: usize,
) -> usize {
    let guard = unsafe { &*(data as *const HeapGuard) };
    guard.hit.store(true, Ordering::SeqCst);
    if let Some(handle) = &guard.handle {
        handle.terminate_execution();
    }
    current_heap_limit.saturating_add(HEAP_LIMIT_BUMP)
}

fn console_log_cb(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    _rv: v8::ReturnValue<v8::Value>,
) {
    console_dispatch(scope, args, ConsoleLevel::Log);
}

fn console_warn_cb(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    _rv: v8::ReturnValue<v8::Value>,
) {
    console_dispatch(scope, args, ConsoleLevel::Warn);
}

fn console_error_cb(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    _rv: v8::ReturnValue<v8::Value>,
) {
    console_dispatch(scope, args, ConsoleLevel::Error);
}

fn console_dispatch(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    level: ConsoleLevel,
) {
    let mut text = String::new();
    for i in 0..args.length() {
        if i > 0 {
            text.push(' ');
        }
        let piece = args.get(i).to_rust_string_lossy(scope);
        text.push_str(&piece);
    }
    // 调用者位置：栈顶帧即 console 调用点（native 回调不入栈）。StackFrame
    // 的行列均 1-based。先取帧再借槽（避免 &mut 槽借用与 scope 不可变借用交叉）
    let frame_pos: Option<(usize, usize)> = v8::StackTrace::current_stack_trace(scope, 1)
        .and_then(|st| st.get_frame(scope, 0))
        .map(|frame| (frame.get_line_number(), frame.get_column()));
    let state = scope.get_slot_mut::<ConsoleState>();
    if let Some(state) = state {
        // TS 时经 sourcemap 回映为原始行列，格式 "L:C"
        let position: Option<String> = frame_pos.and_then(|(line, column)| {
            if line == 0 {
                return None;
            }
            Some(match state.ts_map.as_deref() {
                Some(map) => {
                    match ts_transpile::map_position(map, line, column.saturating_sub(1)) {
                        Some((tl, tc)) => format!("{tl}:{tc}"),
                        None => format!("{line}:{column}"),
                    }
                }
                None => format!("{line}:{column}"),
            })
        });
        if let Some(sink) = &state.sink {
            sink(level, &text, position.clone());
        }
        state.lines.push(ConsoleLine { level, text, position });
    }
}

// ── Tauri 命令层（P1 UI 接线） ──────────────────────────────

/// 前端事件流（dbQuery/fleet_exec 同款 tag=type 模式；字段逐变体 camelCase——
/// 枚举级 rename 不吃字段，每个变体单独标）
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum JsRunEvent {
    #[serde(rename_all = "camelCase")]
    Console {
        level: ConsoleLevel,
        text: String,
        position: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    Done {
        result_json: Option<String>,
        elapsed_ms: u128,
    },
    #[serde(rename_all = "camelCase")]
    Error {
        /// timeout / heapLimit / script / cancelled / host
        kind: String,
        message: String,
        elapsed_ms: u128,
    },
}

/// 进行中运行的取消旗标注册表（db_query_cancel 同型；runId 由前端生成）
static ACTIVE_RUNS: OnceLock<Mutex<HashMap<String, Arc<AtomicBool>>>> = OnceLock::new();

fn active_runs() -> &'static Mutex<HashMap<String, Arc<AtomicBool>>> {
    ACTIVE_RUNS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 运行一段 JS 脚本：console 逐行流式经 Channel 回传，终态（done/error）
/// 亦经同一 Channel；invoke 本身在结束时 resolve。阻塞执行放 spawn_blocking，
/// 不占 Tokio worker。取消 = js_run_cancel 置旗标 → 等待方终止 isolate。
#[tauri::command]
pub async fn js_run(
    run_id: String,
    source: String,
    timeout_ms: Option<u64>,
    heap_limit_mb: Option<usize>,
    language: Option<String>,
    on_event: tauri::ipc::Channel<JsRunEvent>,
) -> Result<(), String> {
    let cancel = Arc::new(AtomicBool::new(false));
    if let Ok(mut runs) = active_runs().lock() {
        runs.insert(run_id.clone(), cancel.clone());
    }
    // Mutex 包 Channel：Channel 保证 Send，Mutex 再补 Sync，使 sink 闭包满足
    // Arc<dyn Fn + Send + Sync> 随 Job 跨线程进宿主线程
    let stream_ch = std::sync::Mutex::new(on_event.clone());
    let sink: ConsoleSink = Arc::new(move |level, text, position| {
        if let Ok(ch) = stream_ch.lock() {
            let _ = ch.send(JsRunEvent::Console {
                level,
                text: text.to_string(),
                position,
            });
        }
    });
    let options = JsRunOptions {
        timeout: Duration::from_millis(
            timeout_ms
                .unwrap_or(DEFAULT_TIMEOUT.as_millis() as u64)
                .clamp(100, 600_000),
        ),
        heap_limit_mb: heap_limit_mb.unwrap_or(DEFAULT_HEAP_LIMIT_MB),
        language: match language.as_deref() {
            Some("typescript") | Some("ts") => JsLanguage::Typescript,
            _ => JsLanguage::Javascript,
        },
    };
    let started = Instant::now();
    let src = source;
    let result = tokio::task::spawn_blocking(move || {
        run_script_with_sink(&src, &options, Some(sink), Some(cancel))
    })
    .await
    .map_err(|e| format!("JS 运行任务失败: {e}"))?;
    if let Ok(mut runs) = active_runs().lock() {
        runs.remove(&run_id);
    }
    let elapsed_ms = started.elapsed().as_millis();
    let terminal = match result {
        Ok(outcome) => JsRunEvent::Done {
            result_json: outcome.result_json,
            elapsed_ms,
        },
        Err(err) => {
            let kind = match &err {
                JsRunError::Timeout { .. } => "timeout",
                JsRunError::HeapLimit => "heapLimit",
                JsRunError::Script { .. } => "script",
                JsRunError::Cancelled => "cancelled",
                JsRunError::HostUnavailable(_) => "host",
            };
            JsRunEvent::Error {
                kind: kind.to_string(),
                message: err.to_string(),
                elapsed_ms,
            }
        }
    };
    let _ = on_event.send(terminal);
    Ok(())
}

/// 取消一次运行；runId 已结束（不存在）返回 false 而非报错。
#[tauri::command]
pub fn js_run_cancel(run_id: String) -> bool {
    if let Ok(runs) = active_runs().lock()
        && let Some(flag) = runs.get(&run_id)
    {
        flag.store(true, Ordering::SeqCst);
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn js_eval_and_chinese_roundtrip() {
        let out = run_script("1 + 1", &JsRunOptions::default()).unwrap();
        assert_eq!(out.result_json.as_deref(), Some("2"));

        let src = r#"({名称:"中文值", list:[1,"a",null]})"#;
        let out = run_script(src, &JsRunOptions::default()).unwrap();
        assert_eq!(
            out.result_json.as_deref(),
            Some(r#"{"名称":"中文值","list":[1,"a",null]}"#)
        );
    }

    #[test]
    fn js_console_bridge_order_and_text() {
        let out = run_script(
            r#"console.log("a", 1, true); console.warn("w"); console.error("e"); 42"#,
            &JsRunOptions::default(),
        )
        .unwrap();
        assert_eq!(out.result_json.as_deref(), Some("42"));
        assert_eq!(out.console.len(), 3);
        let first = out.console.first().unwrap();
        assert_eq!(first.level, ConsoleLevel::Log);
        assert_eq!(first.text, "a 1 true");
        assert_eq!(out.console.get(1).unwrap().level, ConsoleLevel::Warn);
        assert_eq!(out.console.get(2).unwrap().level, ConsoleLevel::Error);
        assert_eq!(out.console.get(2).unwrap().text, "e");
    }

    #[test]
    fn js_result_shapes() {
        assert_eq!(
            run_script("undefined", &JsRunOptions::default())
                .unwrap()
                .result_json,
            None
        );
        assert_eq!(
            run_script("null", &JsRunOptions::default())
                .unwrap()
                .result_json
                .as_deref(),
            Some("null")
        );
        assert_eq!(
            run_script("[1, 2, 3]", &JsRunOptions::default())
                .unwrap()
                .result_json
                .as_deref(),
            Some("[1,2,3]")
        );
    }

    #[test]
    fn js_exception_reporting() {
        let err = run_script(r#"throw new Error("boom 中文")"#, &JsRunOptions::default()).unwrap_err();
        let JsRunError::Script { message } = &err else {
            assert!(matches!(err, JsRunError::Script { .. }), "期望 Script 错误");
            return;
        };
        assert!(message.contains("boom 中文"), "message: {message}");
        // 语法错误同样收敛到 Script 分支
        let err = run_script("function {", &JsRunOptions::default()).unwrap_err();
        assert!(matches!(err, JsRunError::Script { .. }), "实际 {err:?}");
    }

    #[test]
    fn js_timeout_kills_infinite_loop_and_host_survives() {
        let opts = JsRunOptions {
            timeout: Duration::from_millis(300),
            heap_limit_mb: 64,
            ..JsRunOptions::default()
        };
        let started = Instant::now();
        let err = run_script("for(;;) {}", &opts).unwrap_err();
        assert!(matches!(err, JsRunError::Timeout { .. }), "实际 {err:?}");
        assert!(started.elapsed() < Duration::from_secs(5));
        // 终止后宿主线程必须继续服务（每次执行独立 isolate）
        let out = run_script("2 + 2", &JsRunOptions::default()).unwrap();
        assert_eq!(out.result_json.as_deref(), Some("4"));
    }

    #[test]
    fn js_heap_limit_graceful_and_host_survives() {
        let opts = JsRunOptions {
            timeout: Duration::from_secs(30),
            heap_limit_mb: 16,
            ..JsRunOptions::default()
        };
        let err = run_script(
            r#"let a = []; for(;;) { a.push("x".repeat(4096)); }"#,
            &opts,
        )
        .unwrap_err();
        assert!(matches!(err, JsRunError::HeapLimit), "实际 {err:?}");
        // 堆触顶终止后宿主线程继续服务
        let out = run_script("'ok'", &JsRunOptions::default()).unwrap();
        assert_eq!(out.result_json.as_deref(), Some("\"ok\""));
    }

    #[test]
    fn js_cancel_interrupts_infinite_loop_and_host_survives() {
        let opts = JsRunOptions {
            timeout: Duration::from_secs(60),
            heap_limit_mb: 64,
            ..JsRunOptions::default()
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            flag.store(true, Ordering::SeqCst);
        });
        let started = Instant::now();
        let err = run_script_with_sink("for(;;) {}", &opts, None, Some(cancel)).unwrap_err();
        assert!(matches!(err, JsRunError::Cancelled), "实际 {err:?}");
        assert!(started.elapsed() < Duration::from_secs(5));
        // 取消终止后宿主线程继续服务
        let out = run_script("1", &JsRunOptions::default()).unwrap();
        assert_eq!(out.result_json.as_deref(), Some("1"));
    }

    #[test]
    fn js_ts_transpile_enum_namespace_types() {
        let src = r#"
            enum Color { Red, Green = 5, Blue }
            namespace NS { export const v = 42; }
            interface P { name: string; age: number }
            const greet = (p: P): string => `${p.name}-${p.age}`;
            const arr: Array<number> = [1, 2, 3];
            console.log(greet({ name: "中文", age: 1 }), NS.v, Color.Blue);
            [Color.Red, Color.Green, arr.length]
        "#;
        let opts = JsRunOptions {
            language: JsLanguage::Typescript,
            ..JsRunOptions::default()
        };
        let out = run_script(src, &opts).unwrap();
        assert_eq!(out.result_json.as_deref(), Some("[0,5,3]"));
        assert_eq!(
            out.console.first().map(|l| l.text.as_str()),
            Some("中文-1 42 6")
        );
    }

    #[test]
    fn js_ts_error_position_mapped_back() {
        // 类型/接口行会被擦除：转译后 JS 行号 ≠ TS 行号，断言回映到原始 TS 行
        let src = [
            "type A = string;",
            "interface C { x: A }",
            "const a: A = \"x\";",
            "const b: number = 1;",
            "function f(p: C): never {",
            "  throw new Error(\"pos-boom\");",
            "}",
            "f({ x: a });",
        ]
        .join("\n");
        let opts = JsRunOptions {
            language: JsLanguage::Typescript,
            ..JsRunOptions::default()
        };
        let err = run_script(&src, &opts).unwrap_err();
        let JsRunError::Script { message } = &err else {
            assert!(matches!(err, JsRunError::Script { .. }), "期望 Script 错误");
            return;
        };
        assert!(message.contains("pos-boom"), "message: {message}");
        assert!(message.contains("TS 第 6 行"), "message: {message}");
    }

    #[test]
    fn js_ts_console_position_mapped() {
        let src = [
            "type T = number;",
            "const v: T = 1;",
            "console.log(\"here\");",
        ]
        .join("\n");
        let opts = JsRunOptions {
            language: JsLanguage::Typescript,
            ..JsRunOptions::default()
        };
        let out = run_script(&src, &opts).unwrap();
        let pos = out
            .console
            .first()
            .and_then(|l| l.position.as_deref())
            .unwrap_or("<missing position>");
        assert!(pos.starts_with("3:"), "position: {pos}");
    }

    #[test]
    fn js_ts_syntax_error_fails_with_diagnostic() {
        let opts = JsRunOptions {
            language: JsLanguage::Typescript,
            ..JsRunOptions::default()
        };
        let err = run_script("const a: number = ;", &opts).unwrap_err();
        assert!(matches!(err, JsRunError::Script { .. }), "实际 {err:?}");
    }

    #[test]
    fn js_console_position_javascript_raw() {
        // JS 同样带位置（原始行列，无映射）
        let out = run_script("console.log('p');", &JsRunOptions::default()).unwrap();
        let pos = out
            .console
            .first()
            .and_then(|l| l.position.as_deref())
            .unwrap_or("<missing position>");
        assert!(pos.starts_with("1:"), "position: {pos}");
    }

    #[test]
    fn js_toplevel_await_rejected_with_own_message() {
        // 顶层 await：V8 编译期英文报错被预拦截，换成自己的中文报错带位置
        let err = run_script(
            "const v = await Promise.resolve(1);",
            &JsRunOptions::default(),
        )
        .unwrap_err();
        let JsRunError::Script { message } = &err else {
            assert!(matches!(err, JsRunError::Script { .. }), "期望 Script 错误");
            return;
        };
        assert!(message.contains("await不可用"), "message: {message}");
        assert!(message.contains("第 1 行"), "message: {message}");
    }

    #[test]
    fn js_async_function_rejected() {
        // async 全禁：无事件循环，async 函数本身即拦截（英文 ReferenceError/
        // 静默不触发都换掉）
        let err = run_script(
            "async function f() { return 1; } f();",
            &JsRunOptions::default(),
        )
        .unwrap_err();
        let JsRunError::Script { message } = &err else {
            assert!(matches!(err, JsRunError::Script { .. }), "期望 Script 错误");
            return;
        };
        assert!(message.contains("async 函数不可用"), "message: {message}");
    }

    #[test]
    fn js_event_loop_apis_rejected() {
        // 定时器/帧调度/微任务入队类：注册了也永不触发，预拦截
        for (src, name) in [
            ("setTimeout(() => {}, 100)", "setTimeout"),
            ("setInterval(() => {}, 100)", "setInterval"),
            ("setImmediate(() => {})", "setImmediate"),
            ("requestAnimationFrame(() => {})", "requestAnimationFrame"),
            ("queueMicrotask(() => {})", "queueMicrotask"),
        ] {
            let err = run_script(src, &JsRunOptions::default()).unwrap_err();
            let JsRunError::Script { message } = &err else {
                assert!(matches!(err, JsRunError::Script { .. }), "{name} 期望 Script 错误");
                return;
            };
            assert!(message.contains(name), "message: {message}");
            assert!(message.contains("不可用"), "message: {message}");
        }
    }

    #[test]
    fn js_network_and_node_apis_rejected() {
        for (src, name) in [
            ("fetch('http://x')", "fetch"),
            ("new WebSocket('ws://x')", "WebSocket"),
            ("process.env.PATH", "process"),
            ("Buffer.from('x')", "Buffer"),
            ("require('fs')", "require"),
        ] {
            let err = run_script(src, &JsRunOptions::default()).unwrap_err();
            let JsRunError::Script { message } = &err else {
                assert!(matches!(err, JsRunError::Script { .. }), "{name} 期望 Script 错误");
                return;
            };
            assert!(message.contains(name), "message: {message}");
        }
    }

    #[test]
    fn js_toplevel_for_await_rejected() {
        let src = "async function* g() { yield 1; }\nfor await (const x of g()) {}\n";
        let err = run_script(src, &JsRunOptions::default()).unwrap_err();
        let JsRunError::Script { message } = &err else {
            assert!(matches!(err, JsRunError::Script { .. }), "期望 Script 错误");
            return;
        };
        // async 生成器声明在前，首个命中是 async 函数（第 1 行）
        assert!(message.contains("async 函数不可用"), "message: {message}");
    }

    #[test]
    fn js_await_as_identifier_still_runs() {
        // await 作标识符：模块语义解析失败 → 检测跳过；经典脚本合法照常执行
        let out = run_script("var await = 5; await + 1", &JsRunOptions::default()).unwrap();
        assert_eq!(out.result_json.as_deref(), Some("6"));
    }

    #[test]
    fn js_ts_toplevel_await_rejected_at_ts_line() {
        let src = [
            "type A = string;",
            "const a: A = 'x';",
            "await Promise.resolve(a);",
        ]
        .join("\n");
        let opts = JsRunOptions {
            language: JsLanguage::Typescript,
            ..JsRunOptions::default()
        };
        let err = run_script(&src, &opts).unwrap_err();
        let JsRunError::Script { message } = &err else {
            assert!(matches!(err, JsRunError::Script { .. }), "期望 Script 错误");
            return;
        };
        assert!(message.contains("await不可用"), "message: {message}");
        assert!(message.contains("第 3 行"), "message: {message}");
    }

    #[test]
    fn js_microtasks_drained_after_sync_return() {
        // 同步脚本的 Promise 微任务在脚本返回后一次清空（P0 无事件循环）
        let out = run_script(
            r#"let x = 0; Promise.resolve().then(() => { console.log("micro", ++x); }); x"#,
            &JsRunOptions::default(),
        )
        .unwrap();
        assert_eq!(out.result_json.as_deref(), Some("0"));
        assert_eq!(
            out.console.first().map(|l| l.text.as_str()),
            Some("micro 1")
        );
    }
}
