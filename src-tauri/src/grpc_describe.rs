//! gRPC describe 渲染器（纯函数，fdp 层）：把反射拿到的 FileDescriptorProto
//! 集合里的指定符号渲染成 proto 定义文本（grpcurl describe 风格）。
//! 支持四类符号：服务全名、服务.方法、消息全名（含嵌套）、枚举全名。
//! 渲染覆盖 proto3 常见面：scalar/引用类型、repeated/optional、oneof、map、
//! 嵌套消息/枚举；外部引用类型给全名不再递归展开（grpcurl 同款边界）。

use prost_types::{
    DescriptorProto, EnumDescriptorProto, FieldDescriptorProto, FileDescriptorProto,
    ServiceDescriptorProto,
};

/// 在反射文件集中定位服务（package.Service == symbol）。
pub fn find_service<'a>(
    fdps: &'a [FileDescriptorProto],
    symbol: &str,
) -> Option<&'a ServiceDescriptorProto> {
    for fdp in fdps {
        let pkg = fdp.package.as_deref().unwrap_or("");
        for svc in &fdp.service {
            if qualified(pkg, svc.name.as_deref().unwrap_or("")) == symbol {
                return Some(svc);
            }
        }
    }
    None
}

/// 渲染符号定义文本；找不到返回 Err（带可用服务名辅助排错）。
pub fn render_symbol_definition(
    fdps: &[FileDescriptorProto],
    symbol: &str,
) -> Result<String, String> {
    // 1. 服务
    if let Some(svc) = find_service(fdps, symbol) {
        let mut out = String::new();
        out.push_str(&format!("{symbol} is a service:\n\n"));
        render_service(svc, &mut out);
        // 连带渲染方法的入出参消息定义（grpcurl describe 行为）
        let mut rendered: Vec<String> = Vec::new();
        for m in &svc.method {
            for ty in [
                m.input_type.as_deref().unwrap_or(""),
                m.output_type.as_deref().unwrap_or(""),
            ] {
                let name = ty.trim_start_matches('.');
                if name.is_empty() || rendered.iter().any(|r| r == name) {
                    continue;
                }
                if let Some((msg, _)) = find_message(fdps, name) {
                    out.push('\n');
                    render_message(msg, &mut out);
                    rendered.push(name.to_string());
                }
            }
        }
        return Ok(out);
    }
    // 2. 方法（symbol 按最后一段切出宿主服务）
    if let Some((svc_name, method_name)) = symbol.rsplit_once('.')
        && let Some(svc) = find_service(fdps, svc_name)
    {
        if let Some(m) = svc
            .method
            .iter()
            .find(|m| m.name.as_deref() == Some(method_name))
        {
            let mut out = String::new();
            out.push_str(&format!("{symbol} is a method:\n\n"));
            out.push_str(&rpc_signature(m));
            out.push('\n');
            let mut rendered: Vec<String> = Vec::new();
            for ty in [
                m.input_type.as_deref().unwrap_or(""),
                m.output_type.as_deref().unwrap_or(""),
            ] {
                let name = ty.trim_start_matches('.');
                if name.is_empty() || rendered.iter().any(|r| r == name) {
                    continue;
                }
                if let Some((msg, _)) = find_message(fdps, name) {
                    out.push('\n');
                    render_message(msg, &mut out);
                    rendered.push(name.to_string());
                }
            }
            return Ok(out);
        }
        return Err(format!(
            "服务 {svc_name} 没有方法 {method_name}；可用方法：{}",
            svc.method
                .iter()
                .filter_map(|m| m.name.clone())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    // 3. 消息（含嵌套全名）
    if let Some((msg, _)) = find_message(fdps, symbol) {
        let mut out = String::new();
        out.push_str(&format!("{symbol} is a message:\n\n"));
        render_message(msg, &mut out);
        return Ok(out);
    }
    // 4. 枚举
    if let Some(en) = find_enum(fdps, symbol) {
        let mut out = String::new();
        out.push_str(&format!("{symbol} is an enum:\n\n"));
        render_enum(en, &mut out);
        return Ok(out);
    }
    let services: Vec<String> = fdps
        .iter()
        .flat_map(|fdp| {
            let pkg = fdp.package.as_deref().unwrap_or("");
            fdp.service
                .iter()
                .map(move |s| qualified(pkg, s.name.as_deref().unwrap_or("")))
        })
        .collect();
    Err(format!(
        "符号 {symbol} 未在反射结果中找到；可 describe 的服务：{}",
        if services.is_empty() {
            "（无）".to_string()
        } else {
            services.join(", ")
        }
    ))
}

fn qualified(pkg: &str, name: &str) -> String {
    if pkg.is_empty() {
        name.to_string()
    } else {
        format!("{pkg}.{name}")
    }
}

/// 在反射文件集中递归找消息（含嵌套），返回以「宿主消息路径」定位的引用。
fn find_message<'a>(
    fdps: &'a [FileDescriptorProto],
    symbol: &str,
) -> Option<(&'a DescriptorProto, Vec<&'a str>)> {
    /// 在一条消息内递归：path 是外层嵌套名（含自身短名）。
    fn walk<'a>(
        msg: &'a DescriptorProto,
        parent: &[&'a str],
        symbol: &str,
    ) -> Option<(&'a DescriptorProto, Vec<&'a str>)> {
        let name = msg.name.as_deref().unwrap_or("");
        let mut path = parent.to_vec();
        path.push(name);
        if path.join(".") == symbol {
            return Some((msg, path));
        }
        for nested in &msg.nested_type {
            if walk(nested, &path, symbol).is_some() {
                return walk(nested, &path, symbol);
            }
        }
        None
    }
    for fdp in fdps {
        let pkg = fdp.package.as_deref().unwrap_or("");
        // package 分段作为嵌套路径前缀，嵌套消息的全名才能与 symbol 对齐
        let scope: Vec<&str> = if pkg.is_empty() {
            Vec::new()
        } else {
            pkg.split('.').collect()
        };
        for msg in &fdp.message_type {
            if walk(msg, &scope, symbol).is_some() {
                return walk(msg, &scope, symbol);
            }
        }
    }
    None
}

fn find_enum<'a>(fdps: &'a [FileDescriptorProto], symbol: &str) -> Option<&'a EnumDescriptorProto> {
    fn walk_msg<'a>(msg: &'a DescriptorProto, parent: &str, symbol: &str) -> Option<&'a EnumDescriptorProto> {
        let name = msg.name.as_deref().unwrap_or("");
        let scope = qualified(parent, name);
        for en in &msg.enum_type {
            if en.name.as_deref().is_some_and(|n| qualified(&scope, n) == symbol) {
                return Some(en);
            }
        }
        for nested in &msg.nested_type {
            if let Some(found) = walk_msg(nested, &scope, symbol) {
                return Some(found);
            }
        }
        None
    }
    for fdp in fdps {
        let pkg = fdp.package.as_deref().unwrap_or("");
        for en in &fdp.enum_type {
            if en.name.as_deref().is_some_and(|n| qualified(pkg, n) == symbol) {
                return Some(en);
            }
        }
        for msg in &fdp.message_type {
            if let Some(found) = walk_msg(msg, pkg, symbol) {
                return Some(found);
            }
        }
    }
    None
}

fn render_service(svc: &ServiceDescriptorProto, out: &mut String) {
    out.push_str(&format!(
        "service {} {{\n",
        svc.name.as_deref().unwrap_or("")
    ));
    for m in &svc.method {
        out.push_str("  ");
        out.push_str(&rpc_signature(m));
        out.push('\n');
    }
    out.push_str("}\n");
}

fn rpc_signature(m: &prost_types::MethodDescriptorProto) -> String {
    let input = m
        .input_type
        .as_deref()
        .unwrap_or("")
        .trim_start_matches('.');
    let output = m
        .output_type
        .as_deref()
        .unwrap_or("")
        .trim_start_matches('.');
    let client = if m.client_streaming.unwrap_or(false) {
        "stream "
    } else {
        ""
    };
    let server = if m.server_streaming.unwrap_or(false) {
        "stream "
    } else {
        ""
    };
    format!(
        "rpc {}({client}{input}) returns ({server}{output});",
        m.name.as_deref().unwrap_or("")
    )
}

/// 渲染消息定义：字段、oneof、map；嵌套消息/枚举原样递归（保持作用域）。
fn render_message(msg: &DescriptorProto, out: &mut String) {
    out.push_str(&format!(
        "message {} {{\n",
        msg.name.as_deref().unwrap_or("")
    ));
    // map 字段（type_name 指向 MapEntry 消息）需要看 entry 内部，提前索引本消息嵌套
    let map_entries: Vec<&DescriptorProto> = msg
        .nested_type
        .iter()
        .filter(|n| n.options.as_ref().is_some_and(|o| o.map_entry.unwrap_or(false)))
        .collect();
    // oneof 声明成员（跳过 proto3 optional 生成的 synthetic oneof：恰好一个成员
    // 且 oneof 名 == "_" + 成员名）
    let synthetic: Vec<bool> = msg
        .oneof_decl
        .iter()
        .enumerate()
        .map(|(idx, decl)| {
            let members: Vec<&FieldDescriptorProto> = msg
                .field
                .iter()
                .filter(|f| f.oneof_index == Some(idx as i32))
                .collect();
            members.len() == 1
                && decl.name.as_deref().is_some_and(|n| n.starts_with('_'))
                && members
                    .first()
                    .and_then(|f| f.name.as_deref())
                    .zip(decl.name.as_deref())
                    .is_some_and(|(fname, oname)| format!("_{fname}") == oname)
        })
        .collect();

    for f in &msg.field {
        let name = f.name.as_deref().unwrap_or("");
        let number = f.number.unwrap_or_default();
        // proto3 optional：label Optional + 有 synthetic oneof → optional 前缀
        let is_synthetic_optional = f
            .oneof_index
            .and_then(|i| synthetic.get(i as usize).copied())
            .unwrap_or(false);
        if let Some(entry) = map_field_entry(f, &map_entries) {
            let key = entry
                .field
                .iter()
                .find(|x| x.number == Some(1))
                .map(field_type_name)
                .unwrap_or_default();
            let value = entry
                .field
                .iter()
                .find(|x| x.number == Some(2))
                .map(field_type_name)
                .unwrap_or_default();
            out.push_str(&format!("  map<{key}, {value}> {name} = {number};\n"));
            continue;
        }
        let prefix = if f.label() == prost_types::field_descriptor_proto::Label::Repeated {
            "repeated "
        } else if is_synthetic_optional {
            "optional "
        } else {
            ""
        };
        out.push_str(&format!(
            "  {}{} {} = {number};\n",
            prefix,
            field_type_name(f),
            name
        ));
    }
    // 真 oneof 分组渲染（跳过 synthetic）
    for (idx, decl) in msg.oneof_decl.iter().enumerate() {
        if synthetic.get(idx).copied().unwrap_or(false) {
            continue;
        }
        out.push_str(&format!(
            "  oneof {} {{\n",
            decl.name.as_deref().unwrap_or("")
        ));
        for f in &msg.field {
            if f.oneof_index == Some(idx as i32) {
                out.push_str(&format!(
                    "    {} {} = {};\n",
                    field_type_name(f),
                    f.name.as_deref().unwrap_or(""),
                    f.number.unwrap_or_default()
                ));
            }
        }
        out.push_str("  }\n");
    }
    for en in &msg.enum_type {
        render_enum_indented(en, out, 1);
    }
    for nested in &msg.nested_type {
        if nested
            .options
            .as_ref()
            .is_some_and(|o| o.map_entry.unwrap_or(false))
        {
            continue;
        }
        out.push_str("  message ");
        // 嵌套消息体内字段相对自身作用域，简单递归（缩进不完美但语义正确）
        let mut body = String::new();
        render_message(nested, &mut body);
        out.push_str(&body);
    }
    out.push_str("}\n");
}

fn render_enum(en: &EnumDescriptorProto, out: &mut String) {
    render_enum_indented(en, out, 0);
}

fn render_enum_indented(en: &EnumDescriptorProto, out: &mut String, indent: usize) {
    let pad = "  ".repeat(indent);
    out.push_str(&format!(
        "{pad}enum {} {{\n",
        en.name.as_deref().unwrap_or("")
    ));
    for v in &en.value {
        out.push_str(&format!(
            "{pad}  {} = {};\n",
            v.name.as_deref().unwrap_or(""),
            v.number.unwrap_or_default()
        ));
    }
    out.push_str(&format!("{pad}}}\n"));
}

/// 字段类型名：scalar 关键字或引用全名（去前导点）。
fn field_type_name(f: &FieldDescriptorProto) -> String {
    use prost_types::field_descriptor_proto::Type;
    if let Some(ty) = f.r#type {
        let scalar = match Type::try_from(ty) {
            Ok(Type::Double) => Some("double"),
            Ok(Type::Float) => Some("float"),
            Ok(Type::Int64) => Some("int64"),
            Ok(Type::Uint64) => Some("uint64"),
            Ok(Type::Int32) => Some("int32"),
            Ok(Type::Fixed64) => Some("fixed64"),
            Ok(Type::Fixed32) => Some("fixed32"),
            Ok(Type::Bool) => Some("bool"),
            Ok(Type::String) => Some("string"),
            Ok(Type::Bytes) => Some("bytes"),
            Ok(Type::Uint32) => Some("uint32"),
            Ok(Type::Sfixed32) => Some("sfixed32"),
            Ok(Type::Sfixed64) => Some("sfixed64"),
            Ok(Type::Sint32) => Some("sint32"),
            Ok(Type::Sint64) => Some("sint64"),
            _ => None,
        };
        if let Some(s) = scalar {
            return s.to_string();
        }
    }
    f.type_name
        .as_deref()
        .unwrap_or("unknown")
        .trim_start_matches('.')
        .to_string()
}

/// map 字段判定：message/enum 引用 + type_name 指向的嵌套是 MapEntry。
fn map_field_entry<'a>(
    f: &FieldDescriptorProto,
    entries: &[&'a DescriptorProto],
) -> Option<&'a DescriptorProto> {
    let type_name = f.type_name.as_deref()?.trim_start_matches('.');
    let short = type_name.rsplit('.').next()?;
    entries.iter().copied().find(|e| e.name.as_deref() == Some(short))
}
