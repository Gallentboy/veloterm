//! Tauri 命令层：暴露给前端调用的反编译与 Jar 透视接口

use super::archive::{parse_jar_structure, read_jar_entry_bytes, JarStructureOverview};
use super::daemon::DecompilerStatus;
use super::GLOBAL_DECOMPILER_MGR;

/// 打开并解析指定 Jar / War / Zip 结构与元数据
#[tauri::command]
pub async fn decompiler_open_jar(file_path: String) -> Result<JarStructureOverview, String> {
    tokio::task::spawn_blocking(move || parse_jar_structure(&file_path))
        .await
        .map_err(|e| format!("执行 Jar 解析任务失败: {}", e))?
}

/// 读取 Jar 内指定条目（若是 .class 则自动反编译为 Java 源码，文本文件直接返回，二进制文件提示）
#[tauri::command]
pub async fn decompiler_read_entry(file_path: String, entry_path: String) -> Result<String, String> {
    eprintln!("[decompiler] 收到条目读取请求: file={}, entry={}", file_path, entry_path);
    tokio::task::spawn_blocking(move || {
        let bytes = read_jar_entry_bytes(&file_path, &entry_path).map_err(|e| {
            eprintln!("[decompiler] 读取条目字节失败: entry={}, err={}", entry_path, e);
            e
        })?;

        if entry_path.ends_with(".class") {
            // 取类名 (如 com/example/Foo)
            let class_name = entry_path.trim_end_matches(".class");
            eprintln!("[decompiler] 准备反编译 Class: {}, 字节数: {}", class_name, bytes.len());
            GLOBAL_DECOMPILER_MGR.decompile_class(class_name, &bytes)
        } else {
            // 普通文本文件（如 pom.properties, MANIFEST.MF, application.yml）
            match String::from_utf8(bytes.clone()) {
                Ok(text) => Ok(text),
                Err(_) => {
                    // 非 UTF-8，判断是否为二进制
                    Ok(format!(
                        "// [二进制资源文件，暂不支持纯文本预览]\n// 路径: {}\n// 大小: {} 字节",
                        entry_path,
                        bytes.len()
                    ))
                }
            }
        }
    })
    .await
    .map_err(|e| format!("读取条目任务失败: {}", e))?
}

/// 获取当前反编译守护环境状态（JRE 路径、版本、Worker 就绪状态）
#[tauri::command]
pub fn decompiler_get_status() -> DecompilerStatus {
    GLOBAL_DECOMPILER_MGR.get_status()
}

/// 弹出原生文件选择对话框选择本地 JAR / WAR 文件（取消返回 None）
#[tauri::command]
pub async fn decompiler_pick_local_jar() -> Result<Option<String>, String> {
    let file = rfd::AsyncFileDialog::new()
        .set_title("选择本地 JAR / WAR 归档文件")
        .add_filter("Java Archive", &["jar", "war"])
        .pick_file()
        .await;

    Ok(file.map(|f| f.path().to_string_lossy().into_owned()))
}

/// 停止常驻守护进程（用于用户离开面板或手动回收内存）
#[tauri::command]
pub fn decompiler_stop_daemon() {
    GLOBAL_DECOMPILER_MGR.stop_daemon();
}
