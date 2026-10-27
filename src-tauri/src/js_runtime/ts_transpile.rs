//! TypeScript → JavaScript 转译（oxc 管线 + v3 sourcemap）。
//!
//! 管线：oxc_parser 解析（TS 经典脚本语义）→ oxc_semantic 出作用域 →
//! oxc_transformer 剥 TS 语法（类型擦除 + enum/namespace 降级；TransformOptions
//! 默认 target 为最新，ES2015..ES2026 降级变换全部 no-op）→ oxc_codegen 出
//! JS 并同时产出 v3 sourcemap。语义为 **transpile-only**：不做类型检查
//! （编辑器侧 ts.worker 负责诊断红线），语义层诊断（重复声明等）不阻塞。
//!
//! sourcemap 消费：`map_position` 把 V8 运行期位置（转译后 JS）回映到原始
//! TS 行列，供异常消息与 console 位置标注。行列表查表每次现生成（脚本规模
//! 小、调用频次低——每错误/console 行一次，不值得为借用期自引用结构缓存）。

use std::path::Path;

use oxc_allocator::Allocator;
use oxc_ast::ast::{ArrowFunctionExpression, AwaitExpression, ForOfStatement, Function, IdentifierReference};
use oxc_ast_visit::walk::{
    walk_arrow_function_expression, walk_await_expression, walk_for_of_statement,
    walk_function, walk_identifier_reference,
};
use oxc_ast_visit::Visit;
use oxc_codegen::{Codegen, CodegenOptions, CodegenReturn};
use oxc_diagnostics::Diagnostics;
use oxc_parser::Parser;
use oxc_semantic::SemanticBuilder;
use oxc_sourcemap::SourceMap;
use oxc_span::SourceType;
use oxc_syntax::scope::ScopeFlags;
use oxc_transformer::{TransformOptions, Transformer};

/// sourcemap 的 sources 名与 Transformer 的 source_path（统一 "script.ts"，
/// 出现在诊断与映射来源标注里）
const SCRIPT_PATH: &str = "script.ts";

/// 转译产物：JS 代码 + 运行期位置回映用的 sourcemap（owned，不依赖
/// 转译期的 arena 生命周期）
pub struct TsTranspiled {
    pub code: String,
    pub map: SourceMap<'static>,
}

/// 把「转译后 JS 的位置」回映为「原始 TS 的 1-based 行列」。
/// 入参约定：`line` 1-based、`column` 0-based（Message::get_start_column 语义）；
/// StackFrame 的列是 1-based，调用方先减一。sourcemap v3 全 0-based。
pub fn map_position(map: &SourceMap<'static>, line: usize, column: usize) -> Option<(usize, usize)> {
    let zero_line = line.checked_sub(1)? as u32;
    let table = map.generate_lookup_table();
    let token = map.lookup_token(&table, zero_line, column as u32)?;
    Some((token.get_src_line() as usize + 1, token.get_src_col() as usize + 1))
}

/// TS → JS。解析或变换诊断硬失败（oxc 诊断自带原始行列，无需映射）。
pub fn transpile(source: &str) -> Result<TsTranspiled, String> {
    let allocator = Allocator::default();
    // 经典脚本语义（非模块）：import/export 在解析层即报错——与 JS 模式
    // 「不支持模块」的既有边界一致
    let source_type = SourceType::default().with_typescript(true).with_module(false);
    let parsed = Parser::new(&allocator, source, source_type).parse();
    if parsed.diagnostics.errors().next().is_some() {
        return Err(render(parsed.diagnostics, source));
    }
    let mut program = parsed.program;

    // 语义分析只为出作用域（transformer 需要）；诊断忽略——transpile-only
    let semantic = SemanticBuilder::new().with_enum_eval(true).build(&program);
    let scoping = semantic.semantic.into_scoping();

    let transformed = Transformer::new(&allocator, Path::new(SCRIPT_PATH), &TransformOptions::default())
        .build_with_scoping(scoping, &mut program);
    if transformed.diagnostics.errors().next().is_some() {
        return Err(render(transformed.diagnostics, source));
    }

    let CodegenReturn { code, map, .. } = Codegen::new()
        .with_options(CodegenOptions {
            source_map_path: Some(Path::new(SCRIPT_PATH).to_path_buf()),
            ..CodegenOptions::default()
        })
        .build(&program);
    match map {
        Some(map) => Ok(TsTranspiled { code, map: map.into_owned() }),
        // 理论不可达：source_map_path 已给
        None => Err("转译内部错误：codegen 未产出 sourcemap".into()),
    }
}

/// oxc 诊断渲染为带原始源码定位的文本（官方 example 同款；
/// render_with_source_code 按值消费 OxcDiagnostic）
fn render(diagnostics: Diagnostics, source: &str) -> String {
    let mut out = String::new();
    for error in diagnostics.errors() {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&error.clone().render_with_source_code(source.to_string()));
    }
    out
}

// ── 禁用项预拦截（源码交给 V8 之前） ────────────────────────────────
//
// 运行时是纯 V8 同步沙箱（无事件循环、无网络、无 Node 运行时）：这些 API
// 在 V8 裸环境里要么根本不存在（用了撞英文 ReferenceError）、要么存在但
// 永不触发（误导用户）。统一在编译前检测，换成带位置与原因的中文报错。
//
// 语法级走 AST 节点标志（async 函数/箭头、await、for-await），标识符级走
// 引用名单命中。以**模块语义**解析（顶层 await 在模块里是合法语法，AST 才
// 能成形）；解析失败静默跳过——如脚本把 await 当标识符用（模块不允许、经典
// 脚本合法），交回正常执行路径。已知边界：标识符检测不做遮蔽分析
// （`const setTimeout = f` 的局部同名会被误报，属可接受的保守拦截）。

/// 定时器/帧调度类：无事件循环，注册了也永不触发
const FORBIDDEN_TIMER: &[&str] = &[
    "setTimeout",
    "setInterval",
    "setImmediate",
    "requestAnimationFrame",
    "requestIdleCallback",
    "queueMicrotask",
];
/// 网络类：纯 V8 沙箱无网络能力
const FORBIDDEN_NETWORK: &[&str] = &["fetch", "XMLHttpRequest", "WebSocket"];
/// Node.js 运行时专属
const FORBIDDEN_NODE: &[&str] = &["process", "Buffer", "require"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForbiddenKind {
    /// async 函数 / await / for await（无事件循环，全禁）
    AsyncSyntax,
    /// setTimeout 等：注册了也永不触发
    TimerApi,
    /// fetch/XHR/WebSocket：无网络能力
    NetworkApi,
    /// process/Buffer/require：非 Node 运行时
    NodeApi,
}

#[derive(Debug, Clone)]
pub struct ForbiddenUse {
    pub kind: ForbiddenKind,
    /// 命中名（语法级为 "await"/"async 函数" 等描述）
    pub name: String,
    pub line: usize,
    pub col: usize,
}

impl ForbiddenUse {
    /// 面向用户的完整错误消息（中文，带位置与原因）
    pub fn message(&self) -> String {
        match self.kind {
            ForbiddenKind::AsyncSyntax => format!(
                "{}不可用（第 {} 行第 {} 列）：JS 运行时无事件循环，仅支持同步脚本\
                 （Promise 链会在脚本返回后自动清空）；异步能力是后续阶段议题",
                self.name, self.line, self.col
            ),
            ForbiddenKind::TimerApi => format!(
                "「{}」不可用（第 {} 行第 {} 列）：运行时无事件循环，\
                 定时器/帧调度类 API（setTimeout/setInterval/setImmediate/\
                 requestAnimationFrame 等）注册了也永不触发",
                self.name, self.line, self.col
            ),
            ForbiddenKind::NetworkApi => format!(
                "「{}」不可用（第 {} 行第 {} 列）：纯 V8 沙箱无网络能力\
                 （fetch/XMLHttpRequest/WebSocket）；需要时等待后续阶段的宿主桥",
                self.name, self.line, self.col
            ),
            ForbiddenKind::NodeApi => format!(
                "「{}」不可用（第 {} 行第 {} 列）：非 Node.js 运行时，\
                 process/Buffer/require（fs/path 等内置模块）均不存在",
                self.name, self.line, self.col
            ),
        }
    }
}

/// 检测禁用的语法与 API 引用（首个命中即返回）
pub fn detect_forbidden(source: &str, typescript: bool) -> Option<ForbiddenUse> {
    let allocator = Allocator::default();
    let mut source_type = SourceType::default().with_module(true);
    if typescript {
        source_type = source_type.with_typescript(true);
    }
    let parsed = Parser::new(&allocator, source, source_type).parse();
    if parsed.diagnostics.errors().next().is_some() {
        return None;
    }
    let mut visitor = ForbiddenVisitor { source, hit: None };
    visitor.visit_program(&parsed.program);
    visitor.hit
}

/// 字节偏移 → 1-based（行, 列）：行按 `\n` 计、列按字符计（与编辑器一致）
fn offset_to_line_col(source: &str, offset: u32) -> (usize, usize) {
    let offset = (offset as usize).min(source.len());
    let before = source.get(..offset).unwrap_or_default();
    let line = before.matches('\n').count() + 1;
    let line_start = before.rfind('\n').map_or(0, |i| i + 1);
    let col = source.get(line_start..offset).unwrap_or_default().chars().count() + 1;
    (line, col)
}

/// AST 访问器：语法级看节点标志，标识符级看引用名单
struct ForbiddenVisitor<'a> {
    source: &'a str,
    hit: Option<ForbiddenUse>,
}

impl<'a> ForbiddenVisitor<'a> {
    fn record(&mut self, kind: ForbiddenKind, name: &str, offset: u32) {
        if self.hit.is_some() {
            return;
        }
        let (line, col) = offset_to_line_col(self.source, offset);
        self.hit = Some(ForbiddenUse {
            kind,
            name: name.to_string(),
            line,
            col,
        });
    }
}

impl<'a> Visit<'a> for ForbiddenVisitor<'a> {
    fn visit_function(&mut self, it: &Function<'a>, flags: ScopeFlags) {
        if it.r#async {
            self.record(ForbiddenKind::AsyncSyntax, "async 函数", it.span.start);
        }
        walk_function(self, it, flags);
    }

    fn visit_arrow_function_expression(&mut self, it: &ArrowFunctionExpression<'a>) {
        if it.r#async {
            self.record(ForbiddenKind::AsyncSyntax, "async 箭头函数", it.span.start);
        }
        walk_arrow_function_expression(self, it);
    }

    fn visit_await_expression(&mut self, it: &AwaitExpression<'a>) {
        self.record(ForbiddenKind::AsyncSyntax, "await", it.span.start);
        walk_await_expression(self, it);
    }

    fn visit_for_of_statement(&mut self, it: &ForOfStatement<'a>) {
        if it.r#await {
            self.record(ForbiddenKind::AsyncSyntax, "for await", it.span.start);
        }
        walk_for_of_statement(self, it);
    }

    fn visit_identifier_reference(&mut self, it: &IdentifierReference<'a>) {
        if self.hit.is_none() {
            let name = it.name.as_str();
            let kind = if FORBIDDEN_TIMER.contains(&name) {
                Some(ForbiddenKind::TimerApi)
            } else if FORBIDDEN_NETWORK.contains(&name) {
                Some(ForbiddenKind::NetworkApi)
            } else if FORBIDDEN_NODE.contains(&name) {
                Some(ForbiddenKind::NodeApi)
            } else {
                None
            };
            if let Some(kind) = kind {
                self.record(kind, name, it.span.start);
            }
        }
        walk_identifier_reference(self, it);
    }
}
