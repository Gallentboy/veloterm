//! jq 过滤执行器：调用内置（Windows coreutils/tools）或系统（macOS/Linux）的 jq CLI
//! 对输入 JSON 进行查询，返回过滤后的 JSON 字符串。

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

/// 定位 jq 二进制或其宿主
enum JqRunner {
    /// 独立 jq 可执行文件（如 tools/jq.exe 或系统 /usr/bin/jq），参数直接为 `[filter]`
    Standalone(PathBuf),
    /// Windows coreutils 多呼叫二进制，参数为 `["jq", filter]`
    #[allow(dead_code)]
    Coreutils(PathBuf),
}

fn resolve_jq_runner() -> Option<JqRunner> {
    // 1. Windows: 优先找安装目录下的 tools/jq.exe，其次 coreutils.exe
    #[cfg(windows)]
    {
        if let Ok(exe) = std::env::current_exe()
            && let Some(dir) = exe.parent()
        {
            let tools_jq = dir.join("tools").join("jq.exe");
            if tools_jq.is_file() {
                return Some(JqRunner::Standalone(tools_jq));
            }
            let coreutils = dir.join("coreutils.exe");
            if coreutils.is_file() {
                return Some(JqRunner::Coreutils(coreutils));
            }
            // 开发模式 target/debug 路径兜底
            let dev_coreutils = dir.join("..").join("coreutils.exe");
            if dev_coreutils.is_file() {
                return Some(JqRunner::Coreutils(dev_coreutils));
            }
        }
    }

    // 2. macOS / Linux 常用路径探测
    #[cfg(not(windows))]
    {
        for p in [
            "/opt/homebrew/bin/jq",
            "/usr/local/bin/jq",
            "/usr/bin/jq",
            "/bin/jq",
        ] {
            let path = Path::new(p);
            if path.is_file() {
                return Some(JqRunner::Standalone(path.to_path_buf()));
            }
        }
    }

    // 3. PATH 环境变量搜索
    if let Ok(path_var) = std::env::var("PATH") {
        let delimiter = if cfg!(windows) { ';' } else { ':' };
        let exe_name = if cfg!(windows) { "jq.exe" } else { "jq" };
        for dir in path_var.split(delimiter) {
            let p = Path::new(dir).join(exe_name);
            if p.is_file() {
                return Some(JqRunner::Standalone(p));
            }
        }
    }

    None
}

/// 执行 jq 过滤表达式，输出结果文本。
///
/// 设置 3 秒执行超时与 5MB 输出截断保护，防止死循环或大内存爆炸。
#[tauri::command]
pub async fn execute_jq_filter(json_input: String, filter: String) -> Result<String, String> {
    let clean_filter = filter.trim();
    let filter_arg = if clean_filter.is_empty() { "." } else { clean_filter };

    let runner = resolve_jq_runner().ok_or_else(|| {
        "未检测到 jq 工具（Windows 随包内置，macOS 可通过 brew install jq 安装）".to_string()
    })?;

    let mut cmd = match runner {
        JqRunner::Standalone(bin) => {
            let mut c = Command::new(bin);
            c.arg(filter_arg);
            c
        }
        JqRunner::Coreutils(bin) => {
            let mut c = Command::new(bin);
            c.arg("jq").arg(filter_arg);
            c
        }
    };

    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    #[cfg(windows)]
    {
        // Windows 下隐藏子进程控制台黑框
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("启动 jq 进程失败: {e}"))?;

    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "无法接管 jq 输入流".to_string())?;

    // 异步喂入数据并在超时内收集输出
    let write_task = async move {
        stdin.write_all(json_input.as_bytes()).await?;
        stdin.shutdown().await?;
        Ok::<(), std::io::Error>(())
    };

    let execution = async {
        let (write_res, wait_res) = tokio::join!(write_task, child.wait_with_output());
        write_res.map_err(|e| format!("写入数据失败: {e}"))?;
        let output = wait_res.map_err(|e| format!("执行异常: {e}"))?;
        Ok::<std::process::Output, String>(output)
    };

    let output = match tokio::time::timeout(Duration::from_secs(3), execution).await {
        Ok(res) => res?,
        Err(_) => {
            return Err("jq 过滤执行超时（3 秒上限）".to_string());
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            format!("jq 执行失败，退出码: {:?}", output.status.code())
        } else {
            stderr
        });
    }

    // 限制最大输出 5MB
    const MAX_OUTPUT_BYTES: usize = 5 * 1024 * 1024;
    let stdout = if output.stdout.len() > MAX_OUTPUT_BYTES {
        let truncated = String::from_utf8_lossy(
            output.stdout.get(..MAX_OUTPUT_BYTES).unwrap_or_default(),
        );
        format!("{truncated}\n... [输出超过 5MB 已截断]")
    } else {
        String::from_utf8_lossy(&output.stdout).to_string()
    };

    Ok(stdout)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_jq_filter_fallback_or_run() {
        let runner = resolve_jq_runner();
        if runner.is_none() {
            eprintln!("当前环境未检测到 jq，跳过此测试");
            return;
        }
        let json = r#"{"name": "veloterm", "features": ["sftp", "terminal", "preview"]}"#;
        let res = execute_jq_filter(json.to_string(), ".name".to_string()).await;
        assert!(res.is_ok(), "jq 执行失败: {:?}", res.err());
        let val = res.unwrap();
        assert!(val.contains("veloterm"), "输出应含 veloterm: {val}");
    }
}
