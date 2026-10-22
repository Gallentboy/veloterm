//! bash/shell 脚本的 tree-sitter 单文档分析（Monaco `shell` 补全的后端半边）。
//!
//! 移植自 bash-language-server 5.7.0（MIT，github.com/bash-lsp/bash-language-server，
//! commit 3218a31）的 `server.ts::onCompletion` → `analyser.ts` →
//! `util/declarations.ts::getLocalDeclarations` 子集：前端补全 provider 每次请求
//! 把全文与光标位置传进来（`analyze_bash_document`），这里返回该位置可见的
//! 变量/函数符号、注释态与所在命令的命令名。上游的跨文件 sourcing 合并、
//! workspace 后台索引、`man`/`help` 文档、bash-completion 选项补全是 Node
//! （fs/子进程）专属能力，不移植——veloterm 编辑的都是 SFTP 远端文件，
//! 本地 fs 语义不成立。
//!
//! 语义对齐上游 getLocalDeclarations（单文件）：
//! - 作用域自底向上：光标节点的各祖先的直接子节点里收集 declaration_command
//!   （local/declare/typeset/export/readonly）、for_statement 循环变量、
//!   variable_assignment、function_definition、`: "${VAR:=default}"` 惯用法；
//! - 全局变量回填：全树收集不在 declaration_command 内的 variable_assignment
//!   （函数体内裸赋值在 bash 中作用域是全局的，上游同收）；
//! - 每个名字只保留「声明行 ≤ 光标行的最近一次声明」，光标之后才声明的
//!   名字不出现（对齐上游 getAllDeclarations 的 position 过滤）。
//!
//! 三处有意偏离上游（bashLanguage.ts 文件头有对应记录）：
//! ① declaration_command 的全部变量都收——上游只取第一个 variable_assignment，
//!   且漏掉 `local A` / `export V` 这类无等号的裸声明；
//! ② `read` 命令的接收变量名也收（上游仅定义跳转收，补全路径不收）；
//! ③ 同名符号跨作用域合并按「≤ 光标行的最近声明」全局取，不做上游的
//!   按名先到先得（上游会因作用域内仅有的声明在光标之后而丢掉更早的
//!   全局声明，bash 运行期语义里后者是可见的）。

use std::collections::HashMap;

use serde::Serialize;
use tree_sitter::{Node, Parser, Point};

/// 光标处可见的一个声明符号。line 为 0-based（对齐 tree-sitter row / LSP line）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BashSymbol {
    pub name: String,
    pub line: u32,
    /// 声明是否位于函数体内（前端 detail 展示用，不参与过滤）
    pub local: bool,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BashAnalysisResult {
    /// 光标是否在 comment 节点内（tree-sitter 判定：heredoc 正文里的 `#`
    /// 不是 comment 节点，比行内正则更准）
    pub in_comment: bool,
    pub variables: Vec<BashSymbol>,
    pub functions: Vec<BashSymbol>,
    /// 光标所在 command 的命令名（未来 `-` 选项补全的上下文；当前前端未消费）
    pub command_name: Option<String>,
}

/// Monaco shell 补全的文档符号分析（内容型分析，与主机无关）。
/// line/column 为 0-based，column 是 UTF-16 code unit（Monaco position 语义）。
#[tauri::command]
pub fn analyze_bash_document(content: String, line: u32, column: u32) -> BashAnalysisResult {
    analyze(&content, line, column)
}

pub fn analyze(source: &str, line: u32, column: u32) -> BashAnalysisResult {
    let mut parser = Parser::new();
    // 静态语言 + 编译期版本配对，错误只可能来自版本漂移；单测锁定，
    // 运行期按「无分析结果」降级（补全退纯静态词表），不 panic
    if parser.set_language(&tree_sitter_bash::LANGUAGE.into()).is_err() {
        return BashAnalysisResult::default();
    }
    let Some(tree) = parser.parse(source, None) else {
        return BashAnalysisResult::default();
    };
    let root = tree.root_node();
    let point = utf16_point_to_byte_point(source, line, column);
    let at = root.descendant_for_point_range(point, point);

    let mut vars: HashMap<String, BashSymbol> = HashMap::new();
    let mut funcs: HashMap<String, BashSymbol> = HashMap::new();
    let cursor_line = line;

    // 作用域自底向上：光标节点的各祖先的直接子节点（上游 getLocalDeclarations 的 walk）
    let mut scope = at;
    while let Some(node) = scope {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            collect_scope_child(child, source, &mut vars, &mut funcs, cursor_line);
        }
        scope = node.parent();
    }

    // 全局变量回填 + read 接收变量（偏离②）：全树遍历
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        match node.kind() {
            "variable_assignment" => {
                if node.parent().is_none_or(|p| p.kind() != "declaration_command") {
                    insert_assignment_var(node, source, &mut vars, cursor_line);
                }
            }
            "command" => collect_read_vars(node, source, &mut vars, cursor_line),
            _ => {}
        }
        let mut cursor = node.walk();
        let children: Vec<Node> = node.children(&mut cursor).collect();
        stack.extend(children);
    }

    BashAnalysisResult {
        in_comment: at.is_some_and(|n| n.kind() == "comment"),
        variables: into_sorted(vars),
        functions: into_sorted(funcs),
        command_name: at.and_then(|n| command_name_at(n, source)),
    }
}

/// 作用域层的一个直接子节点（上游 walk 的逐分支对应物）。
fn collect_scope_child(
    child: Node,
    source: &str,
    vars: &mut HashMap<String, BashSymbol>,
    funcs: &mut HashMap<String, BashSymbol>,
    cursor_line: u32,
) {
    match child.kind() {
        // local/declare/typeset/export/readonly ...：偏离①——全部变量都收（上游
        // 只取第一个 variable_assignment），且裸声明（`local A` / `export V`）也收。
        // 只看直接子节点：`local a=$b` 里 $b 的 variable_name 在 expansion 更深处，不会混入
        "declaration_command" => {
            let mut cursor = child.walk();
            for var_child in child.children(&mut cursor) {
                match var_child.kind() {
                    "variable_assignment" => {
                        insert_assignment_var(var_child, source, vars, cursor_line);
                    }
                    "variable_name" => {
                        if let Some(name) = text_of(var_child, source) {
                            insert_symbol(
                                vars,
                                name,
                                var_child.start_position().row as u32,
                                has_function_ancestor(var_child),
                                cursor_line,
                            );
                        }
                    }
                    _ => {}
                }
            }
        }
        // for_statement 的循环变量（child(1)，上游 child(1) 同位）
        "for_statement" => {
            if let Some(var_node) = child.child(1).filter(|n| n.kind() == "variable_name")
                && let Some(name) = text_of(var_node, source)
            {
                insert_symbol(
                    vars,
                    name,
                    var_node.start_position().row as u32,
                    has_function_ancestor(var_node),
                    cursor_line,
                );
            }
        }
        // 上游 isDefinition：variable_assignment / function_definition
        "variable_assignment" => insert_assignment_var(child, source, vars, cursor_line),
        "function_definition" => {
            if let Some(name_node) = child.named_child(0)
                && let Some(name) = text_of(name_node, source)
            {
                insert_symbol(
                    funcs,
                    name,
                    child.start_position().row as u32,
                    false,
                    cursor_line,
                );
            }
        }
        // `: "${VAR:=default}"` 惯用法（: 做参数展开并保留副作用）
        "command" => collect_colon_default(child, source, vars, cursor_line),
        _ => {}
    }
}

/// `: "${VAR:=default}"`：string → expansion → variable_name（上游层级同构）
fn collect_colon_default(
    command: Node,
    source: &str,
    vars: &mut HashMap<String, BashSymbol>,
    cursor_line: u32,
) {
    if !text_of(command, source).is_some_and(|t| t.starts_with(": ")) {
        return;
    }
    let mut cursor = command.walk();
    for arg in command.children(&mut cursor) {
        if arg.kind() != "string" {
            continue;
        }
        let mut string_cursor = arg.walk();
        for expansion in arg.children(&mut string_cursor) {
            if expansion.kind() != "expansion" {
                continue;
            }
            let mut expansion_cursor = expansion.walk();
            for var_node in expansion.children(&mut expansion_cursor) {
                if var_node.kind() == "variable_name"
                    && let Some(name) = text_of(var_node, source)
                {
                    insert_symbol(
                        vars,
                        name,
                        var_node.start_position().row as u32,
                        has_function_ancestor(var_node),
                        cursor_line,
                    );
                }
            }
        }
    }
}

/// `read [-r] NAME...` 的接收变量（偏离②）。对齐上游 isVariableInReadCommand：
/// word 节点、不以 `-` 开头、且前一个兄弟不是携带参数的标志
/// （`-d/-i/-n/-N/-p/-t/-u` 之后的一个词是标志参数，不是变量）。
fn collect_read_vars(
    command: Node,
    source: &str,
    vars: &mut HashMap<String, BashSymbol>,
    cursor_line: u32,
) {
    let Some(first) = command.child(0) else {
        return;
    };
    if text_of(first, source) != Some("read") {
        return;
    }
    let mut cursor = command.walk();
    let mut prev: Option<Node> = None;
    for word_node in command.children(&mut cursor) {
        if word_node.kind() == "word"
            && let Some(text) = text_of(word_node, source)
        {
            let prev_takes_arg = prev.is_some_and(|p| {
                text_of(p, source).is_some_and(|t| {
                    t.starts_with('-')
                        && t.ends_with(['d', 'i', 'n', 'N', 'p', 't', 'u'])
                })
            });
            if !text.starts_with('-') && !prev_takes_arg {
                insert_symbol(
                    vars,
                    text,
                    word_node.start_position().row as u32,
                    has_function_ancestor(word_node),
                    cursor_line,
                );
            }
        }
        prev = Some(word_node);
    }
}

/// variable_assignment 的变量名 = 首个命名子节点（variable_name）
fn insert_assignment_var(
    assignment: Node,
    source: &str,
    vars: &mut HashMap<String, BashSymbol>,
    cursor_line: u32,
) {
    let Some(name_node) = assignment.named_child(0) else {
        return;
    };
    let Some(name) = text_of(name_node, source) else {
        return;
    };
    insert_symbol(
        vars,
        name,
        assignment.start_position().row as u32,
        has_function_ancestor(assignment),
        cursor_line,
    );
}

/// 统一的符号入口：光标之后声明的不收；同名保留「≤ 光标行的最近声明」（偏离③）
fn insert_symbol(
    map: &mut HashMap<String, BashSymbol>,
    name: &str,
    line: u32,
    local: bool,
    cursor_line: u32,
) {
    if line > cursor_line {
        return;
    }
    let should_update = match map.get(name) {
        Some(existing) => existing.line < line,
        None => true,
    };
    if should_update {
        let owned = name.to_string();
        map.insert(owned.clone(), BashSymbol { name: owned, line, local });
    }
}

#[inline]
fn has_function_ancestor(node: Node) -> bool {
    let mut parent = node.parent();
    while let Some(p) = parent {
        if p.kind() == "function_definition" {
            return true;
        }
        parent = p.parent();
    }
    false
}

/// 光标所在 command 的命令名（上游 commandNameAtPoint：向上找 command，
/// 其首个命名子节点须是 command_name）
fn command_name_at(node: Node, source: &str) -> Option<String> {
    let mut current = Some(node);
    while let Some(n) = current {
        if n.kind() == "command" {
            return n
                .named_child(0)
                .filter(|c| c.kind() == "command_name")
                .and_then(|c| text_of(c, source))
                .map(|t| t.trim().to_string());
        }
        current = n.parent();
    }
    None
}

#[inline(always)]
fn text_of<'a>(node: Node, source: &'a str) -> Option<&'a str> {
    node.utf8_text(source.as_bytes()).ok()
}

fn into_sorted(map: HashMap<String, BashSymbol>) -> Vec<BashSymbol> {
    let mut symbols: Vec<BashSymbol> = map.into_values().collect();
    symbols.sort_by(|a, b| a.line.cmp(&b.line).then_with(|| a.name.cmp(&b.name)));
    symbols
}

/// Monaco 光标（line + UTF-16 column）→ tree-sitter Point（row + 行内字节列）。
/// tree-sitter 的 column 语义是字节偏移；字符串切片只落在 char_indices/find
/// 得出的字符边界上。
fn utf16_point_to_byte_point(source: &str, line: u32, column_utf16: u32) -> Point {
    let target_line = line as usize;
    let mut cur_line = 0usize;
    let mut line_start = 0usize;
    for (idx, ch) in source.char_indices() {
        if ch == '\n' {
            if cur_line == target_line {
                break;
            }
            cur_line += 1;
            line_start = idx + 1;
        }
    }
    if cur_line < target_line {
        // 行号超出文件（理论不该发生）：夹取到末行末列
        return Point::new(cur_line, source.len() - line_start);
    }
    let line_end = source[line_start..]
        .find('\n')
        .map_or(source.len(), |i| line_start + i);
    let mut byte_col = 0usize;
    let mut u16_count = 0usize;
    for ch in source[line_start..line_end].chars() {
        if u16_count >= column_utf16 as usize {
            break;
        }
        u16_count += ch.len_utf16();
        byte_col += ch.len_utf8();
    }
    Point::new(target_line, byte_col)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn var_names(r: &BashAnalysisResult) -> Vec<String> {
        r.variables.iter().map(|s| s.name.clone()).collect()
    }

    fn func_names(r: &BashAnalysisResult) -> Vec<String> {
        r.functions.iter().map(|s| s.name.clone()).collect()
    }

    fn assert_has(names: &[String], expect: &str) {
        assert!(
            names.iter().any(|n| n == expect),
            "expected {expect:?} in {names:?}"
        );
    }

    fn assert_lacks(names: &[String], expect: &str) {
        assert!(
            !names.iter().any(|n| n == expect),
            "unexpected {expect:?} in {names:?}"
        );
    }

    #[test]
    fn global_variables_visible_and_cursor_filter() {
        // TOP=1 在光标行之前：可见；LATER=2 在光标行之后：不可见
        let src = "TOP=1\necho hi\nLATER=2\n";
        let r = analyze(src, 1, 6);
        let vars = var_names(&r);
        assert_has(&vars, "TOP");
        assert_lacks(&vars, "LATER");
    }

    #[test]
    fn local_variable_scoping() {
        // 光标在 f 内：f 的 local 可见、g 的 local 不可见；末尾补一行顶层命令
        // 作「回到顶层」的探测位（光标压在闭括号行上仍算在函数 span 内）
        let src = "g() {\n  local GV=1\n}\nf() {\n  local FV=2\n  echo\n}\necho\n";
        let r = analyze(src, 5, 4);
        let vars = var_names(&r);
        assert_has(&vars, "FV");
        assert_lacks(&vars, "GV");
        // FV 声明在函数体内：local 标记为 true
        let fv = r.variables.iter().find(|s| s.name == "FV");
        assert!(fv.is_some_and(|s| s.local));

        // 光标回到顶层（末尾独立命令行）：两个 local 都不可见
        let r2 = analyze(src, 8, 2);
        let vars2 = var_names(&r2);
        assert_lacks(&vars2, "FV");
        assert_lacks(&vars2, "GV");
    }

    #[test]
    fn plain_assignment_in_function_is_global() {
        // 函数体内裸赋值在 bash 中是全局变量：顶层光标也可见（上游全局回填同收）
        let src = "f() {\n  INSIDE=1\n}\necho\n";
        let r = analyze(src, 3, 2);
        assert_has(&var_names(&r), "INSIDE");
    }

    #[test]
    fn for_loop_variable() {
        let src = "for i in 1 2 3; do\n  echo $i\ndone\necho\n";
        let r = analyze(src, 3, 0);
        assert_has(&var_names(&r), "i");
    }

    #[test]
    fn read_command_variables() {
        // 偏离②：read 的接收变量收进补全
        let src = "read -r NAME\necho\n";
        let r = analyze(src, 1, 0);
        assert_has(&var_names(&r), "NAME");
        // 标志带参数：-p 之后的提示串不是变量，其后才是
        let src2 = "read -p \"prompt\" ANS\necho\n";
        let r2 = analyze(src2, 1, 0);
        let vars2 = var_names(&r2);
        assert_has(&vars2, "ANS");
        assert_lacks(&vars2, "prompt");
    }

    #[test]
    fn declaration_command_all_vars_and_bare_names() {
        // 偏离①：local 的全部变量 + export 的裸声明
        let src = "f() {\n  local A=1 B=2\n  export VERBOSE\n  echo\n}\n";
        let r = analyze(src, 3, 2);
        let vars = var_names(&r);
        assert_has(&vars, "A");
        assert_has(&vars, "B");
        assert_has(&vars, "VERBOSE");
    }

    #[test]
    fn functions_visible_with_cursor_filter() {
        // 光标在第 2 行（deploy 内的闭括号行）：deploy 可见；rollback 声明在
        // 光标行之后：不可见（同行的声明保留，对齐上游 strictly-after 过滤）
        let src = "deploy() {\n  echo 1\n}\nrollback() {\n  echo 2\n}\necho\n";
        let r = analyze(src, 2, 0);
        let funcs = func_names(&r);
        assert_has(&funcs, "deploy");
        assert_lacks(&funcs, "rollback");
    }

    #[test]
    fn comment_detection_and_heredoc() {
        // 行内注释：光标在 # 之后
        let src = "echo hi # a comment\n";
        let r = analyze(src, 0, 12);
        assert!(r.in_comment);
        // heredoc 正文里的 # 不是注释节点
        let src2 = "cat <<EOF\n# not a comment\nEOF\nx\n";
        let r2 = analyze(src2, 1, 8);
        assert!(!r2.in_comment);
    }

    #[test]
    fn command_name_at_point() {
        let src = "git checkout main\n";
        let r = analyze(src, 0, 8);
        assert_eq!(r.command_name.as_deref(), Some("git"));
    }

    #[test]
    fn colon_default_idiom() {
        let src = ": \"${VAR:=default}\"\necho\n";
        let r = analyze(src, 1, 0);
        assert_has(&var_names(&r), "VAR");
    }

    #[test]
    fn utf16_column_to_byte_point() {
        // "ab中c"：中 占 3 字节 / 1 个 UTF-16 单元；column 3（a b 中 之后）→ 字节列 5
        let src = "ab中c\nX=1\n";
        let p = utf16_point_to_byte_point(src, 0, 3);
        assert_eq!(p.row, 0);
        assert_eq!(p.column, 5);
        // 代理对（😀 占 2 个 UTF-16 单元 / 4 字节）
        let src2 = "a😀b\n";
        let p2 = utf16_point_to_byte_point(src2, 0, 3);
        assert_eq!(p2.column, 5);
    }

    #[test]
    fn empty_and_invalid_input_degrade() {
        let r = analyze("", 0, 0);
        assert!(!r.in_comment);
        assert!(r.variables.is_empty());
        // 语法错误输入：tree-sitter 产出含 ERROR 节点的树，不 panic
        let r2 = analyze("if [[ ; then\n BROKEN=(((\n", 1, 5);
        assert!(!r2.in_comment);
    }
}
