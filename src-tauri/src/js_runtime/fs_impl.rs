//! JS 脚本运行时（V8）——同步 fs 沙盒文件操作。
//!
//! 安全沙盒规则：
//! 1. 严格限制在两个目录范围内：
//!    - 工作区目录：`ProjectDirs::data_dir().join("scripts_workspace")`
//!    - 临时目录：`std::env::temp_dir().join("veloterm_scripts")`
//! 2. 任何其它路径、`..` 目录穿越、逃逸符号链接均直接抛出安全拒绝异常。
//! 3. 全量纯同步（Synchronous）：无 Promise，无事件循环，阻塞完成即返回。

use std::fs;
use std::path::{Component, Path, PathBuf};
use directories::ProjectDirs;

/// 获取跨平台应用数据目录下的脚本沙盒工作空间目录
pub fn get_workspace_dir() -> PathBuf {
    if let Some(proj_dirs) = ProjectDirs::from("com", "gallentboy", "veloterm") {
        let mut data_dir = proj_dirs.data_dir().to_path_buf();
        data_dir.push("scripts_workspace");
        if !data_dir.exists() {
            let _ = fs::create_dir_all(&data_dir);
        }
        data_dir
    } else {
        let fallback = std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(".scripts_workspace");
        if !fallback.exists() {
            let _ = fs::create_dir_all(&fallback);
        }
        fallback
    }
}

/// 获取跨平台脚本沙盒专用临时目录
pub fn get_temp_dir() -> PathBuf {
    let mut temp = std::env::temp_dir();
    temp.push("veloterm_scripts");
    if !temp.exists() {
        let _ = fs::create_dir_all(&temp);
    }
    temp
}

/// 逻辑规整化路径组件（消除 CurDir 与 ParentDir）
fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            c => normalized.push(c.as_os_str()),
        }
    }
    normalized
}

/// 校验路径是否在合法沙盒内（工作空间目录或临时目录），若合法则返回物理归一化后的 PathBuf
pub fn resolve_sandboxed_path(raw_path: &str) -> Result<PathBuf, String> {
    let raw_path = raw_path.trim();
    if raw_path.is_empty() {
        return Err("路径不能为空".to_string());
    }

    let ws = get_workspace_dir();
    let temp = get_temp_dir();

    let ws_canonical = ws.canonicalize().unwrap_or(ws.clone());
    let temp_canonical = temp.canonicalize().unwrap_or(temp.clone());

    let target = Path::new(raw_path);
    let full_path = if target.is_absolute() {
        target.to_path_buf()
    } else {
        ws.join(target)
    };

    let normalized = normalize_path(&full_path);

    // 检查是否在 workspace 或 temp 目录下
    let in_workspace = normalized.starts_with(&ws) || normalized.starts_with(&ws_canonical);
    let in_temp = normalized.starts_with(&temp) || normalized.starts_with(&temp_canonical);

    if !in_workspace && !in_temp {
        return Err(format!(
            "沙盒越界: 路径「{}」超出允许的工作空间目录与临时目录",
            raw_path
        ));
    }

    // 防符号链接逃逸检查：向回溯找已存在的最长祖先并验证真实 canonicalize 路径
    let mut ancestor = normalized.clone();
    while !ancestor.exists() {
        if let Some(parent) = ancestor.parent() {
            ancestor = parent.to_path_buf();
        } else {
            break;
        }
    }
    if ancestor.exists()
        && let Ok(canon_ancestor) = ancestor.canonicalize()
        && !canon_ancestor.starts_with(&ws_canonical)
        && !canon_ancestor.starts_with(&temp_canonical)
    {
        return Err(format!(
            "符号链接逃逸: 路径「{}」实际指向沙盒外「{}」",
            raw_path,
            canon_ancestor.display()
        ));
    }

    Ok(normalized)
}

fn throw_js_error(scope: &mut v8::PinScope, msg: &str) {
    let s = v8::String::new(scope, msg).unwrap_or_else(|| v8::String::empty(scope));
    let err = v8::Exception::error(scope, s);
    scope.throw_exception(err);
}

// ── V8 函数回调 ────────────────────────────────────────────────────────

/// fs.readTextFile(path: string): string
pub fn fs_read_text_file_cb(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue<v8::Value>,
) {
    let Some(raw_path) = get_path_arg(scope, &args) else {
        return;
    };
    let path = match resolve_sandboxed_path(&raw_path) {
        Ok(p) => p,
        Err(e) => {
            throw_js_error(scope, &e);
            return;
        }
    };
    match fs::read_to_string(&path) {
        Ok(content) => {
            if let Some(s) = v8::String::new(scope, &content) {
                rv.set(s.into());
            }
        }
        Err(e) => {
            throw_js_error(scope, &format!("读取文本文件「{}」失败: {e}", path.display()));
        }
    }
}

/// fs.readFile(path: string): Uint8Array
pub fn fs_read_file_cb(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue<v8::Value>,
) {
    let Some(raw_path) = get_path_arg(scope, &args) else {
        return;
    };
    let path = match resolve_sandboxed_path(&raw_path) {
        Ok(p) => p,
        Err(e) => {
            throw_js_error(scope, &e);
            return;
        }
    };
    match fs::read(&path) {
        Ok(bytes) => {
            let len = bytes.len();
            let ab = v8::ArrayBuffer::new(scope, len);
            if len > 0 {
                let store = ab.get_backing_store();
                if let Some(ptr) = store.data() {
                    unsafe {
                        std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr.as_ptr() as *mut u8, len);
                    }
                }
            }
            if let Some(u8arr) = v8::Uint8Array::new(scope, ab, 0, len) {
                rv.set(u8arr.into());
            }
        }
        Err(e) => {
            throw_js_error(scope, &format!("读取二进制文件「{}」失败: {e}", path.display()));
        }
    }
}

/// fs.writeTextFile(path: string, content: string): void
pub fn fs_write_text_file_cb(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    _rv: v8::ReturnValue<v8::Value>,
) {
    let Some(raw_path) = get_path_arg(scope, &args) else {
        return;
    };
    let content = if args.length() > 1 {
        args.get(1).to_rust_string_lossy(scope)
    } else {
        String::new()
    };
    let path = match resolve_sandboxed_path(&raw_path) {
        Ok(p) => p,
        Err(e) => {
            throw_js_error(scope, &e);
            return;
        }
    };
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Err(e) = fs::write(&path, content) {
        throw_js_error(scope, &format!("写入文本文件「{}」失败: {e}", path.display()));
    }
}

/// fs.writeFile(path: string, data: string | Uint8Array | ArrayBuffer): void
pub fn fs_write_file_cb(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    _rv: v8::ReturnValue<v8::Value>,
) {
    let Some(raw_path) = get_path_arg(scope, &args) else {
        return;
    };
    let bytes = if args.length() > 1 {
        extract_bytes(scope, args.get(1))
    } else {
        Vec::new()
    };
    let path = match resolve_sandboxed_path(&raw_path) {
        Ok(p) => p,
        Err(e) => {
            throw_js_error(scope, &e);
            return;
        }
    };
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Err(e) = fs::write(&path, bytes) {
        throw_js_error(scope, &format!("写入文件「{}」失败: {e}", path.display()));
    }
}

/// fs.appendFile(path: string, data: string | Uint8Array | ArrayBuffer): void
pub fn fs_append_file_cb(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    _rv: v8::ReturnValue<v8::Value>,
) {
    use std::io::Write;
    let Some(raw_path) = get_path_arg(scope, &args) else {
        return;
    };
    let bytes = if args.length() > 1 {
        extract_bytes(scope, args.get(1))
    } else {
        Vec::new()
    };
    let path = match resolve_sandboxed_path(&raw_path) {
        Ok(p) => p,
        Err(e) => {
            throw_js_error(scope, &e);
            return;
        }
    };
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    match fs::OpenOptions::new().create(true).append(true).open(&path) {
        Ok(mut f) => {
            if let Err(e) = f.write_all(&bytes) {
                throw_js_error(scope, &format!("追加文件「{}」失败: {e}", path.display()));
            }
        }
        Err(e) => {
            throw_js_error(scope, &format!("打开追加文件「{}」失败: {e}", path.display()));
        }
    }
}

/// fs.exists(path: string): boolean
pub fn fs_exists_cb(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue<v8::Value>,
) {
    let Some(raw_path) = get_path_arg(scope, &args) else {
        return;
    };
    let exists = match resolve_sandboxed_path(&raw_path) {
        Ok(p) => p.exists(),
        Err(_) => false,
    };
    rv.set(v8::Boolean::new(scope, exists).into());
}

/// fs.mkdir(path: string, recursive?: boolean): void
pub fn fs_mkdir_cb(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    _rv: v8::ReturnValue<v8::Value>,
) {
    let Some(raw_path) = get_path_arg(scope, &args) else {
        return;
    };
    let recursive = if args.length() > 1 {
        args.get(1).is_true()
    } else {
        true
    };
    let path = match resolve_sandboxed_path(&raw_path) {
        Ok(p) => p,
        Err(e) => {
            throw_js_error(scope, &e);
            return;
        }
    };
    let res = if recursive {
        fs::create_dir_all(&path)
    } else {
        fs::create_dir(&path)
    };
    if let Err(e) = res {
        throw_js_error(scope, &format!("创建目录「{}」失败: {e}", path.display()));
    }
}

/// fs.readDir(path: string): Array<{ name: string, isDirectory: boolean, isFile: boolean, size: number }>
pub fn fs_read_dir_cb(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue<v8::Value>,
) {
    let raw_path = if args.length() > 0 && !args.get(0).is_undefined() {
        args.get(0).to_rust_string_lossy(scope)
    } else {
        ".".to_string()
    };
    let path = match resolve_sandboxed_path(&raw_path) {
        Ok(p) => p,
        Err(e) => {
            throw_js_error(scope, &e);
            return;
        }
    };
    let entries = match fs::read_dir(&path) {
        Ok(rd) => rd,
        Err(e) => {
            throw_js_error(scope, &format!("读取目录「{}」失败: {e}", path.display()));
            return;
        }
    };

    let arr = v8::Array::new(scope, 0);
    for (idx, entry) in entries.flatten().enumerate() {
        let obj = v8::Object::new(scope);
        let name = entry.file_name().to_string_lossy().to_string();
        let file_type = entry.file_type().ok();
        let is_dir = file_type.as_ref().is_some_and(|t| t.is_dir());
        let is_file = file_type.as_ref().is_some_and(|t| t.is_file());
        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);

        if let (Some(k), Some(v)) = (v8::String::new(scope, "name"), v8::String::new(scope, &name)) {
            obj.set(scope, k.into(), v.into());
        }
        if let Some(k) = v8::String::new(scope, "isDirectory") {
            obj.set(scope, k.into(), v8::Boolean::new(scope, is_dir).into());
        }
        if let Some(k) = v8::String::new(scope, "isFile") {
            obj.set(scope, k.into(), v8::Boolean::new(scope, is_file).into());
        }
        if let Some(k) = v8::String::new(scope, "size") {
            obj.set(scope, k.into(), v8::Number::new(scope, size as f64).into());
        }

        arr.set_index(scope, idx as u32, obj.into());
    }

    rv.set(arr.into());
}

/// fs.remove(path: string, recursive?: boolean): void
pub fn fs_remove_cb(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    _rv: v8::ReturnValue<v8::Value>,
) {
    let Some(raw_path) = get_path_arg(scope, &args) else {
        return;
    };
    let recursive = if args.length() > 1 {
        args.get(1).is_true()
    } else {
        false
    };
    let path = match resolve_sandboxed_path(&raw_path) {
        Ok(p) => p,
        Err(e) => {
            throw_js_error(scope, &e);
            return;
        }
    };
    let res = if path.is_dir() {
        if recursive {
            fs::remove_dir_all(&path)
        } else {
            fs::remove_dir(&path)
        }
    } else {
        fs::remove_file(&path)
    };
    if let Err(e) = res {
        throw_js_error(scope, &format!("删除文件/目录「{}」失败: {e}", path.display()));
    }
}

/// fs.stat(path: string): { isDirectory: boolean, isFile: boolean, size: number, modifiedMs: number }
pub fn fs_stat_cb(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue<v8::Value>,
) {
    let Some(raw_path) = get_path_arg(scope, &args) else {
        return;
    };
    let path = match resolve_sandboxed_path(&raw_path) {
        Ok(p) => p,
        Err(e) => {
            throw_js_error(scope, &e);
            return;
        }
    };
    let meta = match fs::metadata(&path) {
        Ok(m) => m,
        Err(e) => {
            throw_js_error(scope, &format!("获取元数据「{}」失败: {e}", path.display()));
            return;
        }
    };

    let obj = v8::Object::new(scope);
    let is_dir = meta.is_dir();
    let is_file = meta.is_file();
    let size = meta.len();
    let modified_ms = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as f64)
        .unwrap_or(0.0);

    if let Some(k) = v8::String::new(scope, "isDirectory") {
        obj.set(scope, k.into(), v8::Boolean::new(scope, is_dir).into());
    }
    if let Some(k) = v8::String::new(scope, "isFile") {
        obj.set(scope, k.into(), v8::Boolean::new(scope, is_file).into());
    }
    if let Some(k) = v8::String::new(scope, "size") {
        obj.set(scope, k.into(), v8::Number::new(scope, size as f64).into());
    }
    if let Some(k) = v8::String::new(scope, "modifiedMs") {
        obj.set(scope, k.into(), v8::Number::new(scope, modified_ms).into());
    }

    rv.set(obj.into());
}

/// fs.copyFile(from: string, to: string): void
pub fn fs_copy_file_cb(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    _rv: v8::ReturnValue<v8::Value>,
) {
    if args.length() < 2 {
        throw_js_error(scope, "copyFile 需要源路径与目标路径 2 个参数");
        return;
    }
    let from_raw = args.get(0).to_rust_string_lossy(scope);
    let to_raw = args.get(1).to_rust_string_lossy(scope);
    let from_path = match resolve_sandboxed_path(&from_raw) {
        Ok(p) => p,
        Err(e) => {
            throw_js_error(scope, &e);
            return;
        }
    };
    let to_path = match resolve_sandboxed_path(&to_raw) {
        Ok(p) => p,
        Err(e) => {
            throw_js_error(scope, &e);
            return;
        }
    };
    if let Some(parent) = to_path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Err(e) = fs::copy(&from_path, &to_path) {
        throw_js_error(scope, &format!("复制文件从「{}」至「{}」失败: {e}", from_path.display(), to_path.display()));
    }
}

/// fs.rename(from: string, to: string): void
pub fn fs_rename_cb(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    _rv: v8::ReturnValue<v8::Value>,
) {
    if args.length() < 2 {
        throw_js_error(scope, "rename 需要源路径与目标路径 2 个参数");
        return;
    }
    let from_raw = args.get(0).to_rust_string_lossy(scope);
    let to_raw = args.get(1).to_rust_string_lossy(scope);
    let from_path = match resolve_sandboxed_path(&from_raw) {
        Ok(p) => p,
        Err(e) => {
            throw_js_error(scope, &e);
            return;
        }
    };
    let to_path = match resolve_sandboxed_path(&to_raw) {
        Ok(p) => p,
        Err(e) => {
            throw_js_error(scope, &e);
            return;
        }
    };
    if let Some(parent) = to_path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Err(e) = fs::rename(&from_path, &to_path) {
        throw_js_error(scope, &format!("重命名「{}」至「{}」失败: {e}", from_path.display(), to_path.display()));
    }
}

/// fs.getWorkspaceDir(): string
pub fn fs_get_workspace_dir_cb(
    scope: &mut v8::PinScope,
    _args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue<v8::Value>,
) {
    let ws = get_workspace_dir().to_string_lossy().to_string();
    if let Some(s) = v8::String::new(scope, &ws) {
        rv.set(s.into());
    }
}

/// fs.getTempDir(): string
pub fn fs_get_temp_dir_cb(
    scope: &mut v8::PinScope,
    _args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue<v8::Value>,
) {
    let temp = get_temp_dir().to_string_lossy().to_string();
    if let Some(s) = v8::String::new(scope, &temp) {
        rv.set(s.into());
    }
}

// ── 辅助函数 ──────────────────────────────────────────────────────────

fn get_path_arg(scope: &mut v8::PinScope, args: &v8::FunctionCallbackArguments) -> Option<String> {
    if args.length() == 0 || args.get(0).is_undefined() || args.get(0).is_null() {
        throw_js_error(scope, "缺少文件路径参数");
        return None;
    }
    Some(args.get(0).to_rust_string_lossy(scope))
}

fn extract_bytes(scope: &mut v8::PinScope, val: v8::Local<v8::Value>) -> Vec<u8> {
    if val.is_string() {
        val.to_rust_string_lossy(scope).into_bytes()
    } else if val.is_array_buffer() {
        if let Ok(ab) = v8::Local::<v8::ArrayBuffer>::try_from(val)
            && let Some(ptr) = ab.get_backing_store().data()
        {
            let len = ab.byte_length();
            let slice = unsafe { std::slice::from_raw_parts(ptr.as_ptr() as *const u8, len) };
            slice.to_vec()
        } else {
            Vec::new()
        }
    } else if val.is_uint8_array() {
        if let Ok(u8arr) = v8::Local::<v8::Uint8Array>::try_from(val) {
            let len = u8arr.byte_length();
            let mut buf = vec![0u8; len];
            u8arr.copy_contents(&mut buf);
            buf
        } else {
            Vec::new()
        }
    } else {
        val.to_rust_string_lossy(scope).into_bytes()
    }
}

/// 将 fs 模块的所有方法注册到给定的 ObjectTemplate 上
pub fn register_fs_methods(
    scope: &mut v8::PinScope,
    fs_tmpl: &v8::Local<v8::ObjectTemplate>,
) {
    macro_rules! bind {
        ($name:expr, $cb:expr) => {
            if let Some(k) = v8::String::new(scope, $name) {
                let f = v8::FunctionTemplate::new(scope, $cb);
                fs_tmpl.set(k.into(), f.into());
            }
        };
    }

    bind!("readTextFile", fs_read_text_file_cb);
    bind!("readFileSync", fs_read_text_file_cb);
    bind!("readFile", fs_read_file_cb);
    bind!("readBytes", fs_read_file_cb);
    bind!("writeTextFile", fs_write_text_file_cb);
    bind!("writeFileSync", fs_write_text_file_cb);
    bind!("writeFile", fs_write_file_cb);
    bind!("appendFile", fs_append_file_cb);
    bind!("appendFileSync", fs_append_file_cb);
    bind!("exists", fs_exists_cb);
    bind!("existsSync", fs_exists_cb);
    bind!("mkdir", fs_mkdir_cb);
    bind!("mkdirSync", fs_mkdir_cb);
    bind!("readDir", fs_read_dir_cb);
    bind!("readdirSync", fs_read_dir_cb);
    bind!("remove", fs_remove_cb);
    bind!("removeSync", fs_remove_cb);
    bind!("unlinkSync", fs_remove_cb);
    bind!("rmdirSync", fs_remove_cb);
    bind!("stat", fs_stat_cb);
    bind!("statSync", fs_stat_cb);
    bind!("copyFile", fs_copy_file_cb);
    bind!("copyFileSync", fs_copy_file_cb);
    bind!("rename", fs_rename_cb);
    bind!("renameSync", fs_rename_cb);
    bind!("getWorkspaceDir", fs_get_workspace_dir_cb);
    bind!("getTempDir", fs_get_temp_dir_cb);
}
