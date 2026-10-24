//! 列出 veloterm.db 里保存的数据库连接，并为 Oracle / 达梦记录直接生成
//! db_engine_test（全链路真机测试 CLI）的可复制运行命令。
//!
//! 运行方式（在 `src-tauri/` 目录下）：
//! ```sh
//! cargo run --example db_conn_list
//! ```
//! 库路径 / 主密钥可用 VELOTERM_DB_PATH / VELOTERM_DB_KEY 覆盖（同 seed 例）。
//! 命令里的密码已按 POSIX 单引号规则转义；仅读本机自己的配置库。

use std::path::PathBuf;

use app_lib::db;

fn resolve_db_path() -> PathBuf {
    if let Ok(p) = std::env::var("VELOTERM_DB_PATH")
        && !p.trim().is_empty()
    {
        return PathBuf::from(p);
    }
    let home = std::env::var("HOME").unwrap_or_default();
    PathBuf::from(home)
        .join("Library")
        .join("Application Support")
        .join("com.gallentboy.veloterm.app")
        .join(db::DEFAULT_DB_FILE_NAME)
}

/// POSIX 单引号包裹（内部单引号转义为 '\''）
fn shq(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

fn main() -> Result<(), String> {
    let path = resolve_db_path();
    let key = std::env::var("VELOTERM_DB_KEY")
        .ok()
        .filter(|k| !k.trim().is_empty())
        .unwrap_or_else(|| db::DEFAULT_MASTER_KEY.to_string());
    let pool = db::init_db_at_path(&path, &key)?;
    let conns = db::list_db_connections(&pool)?;

    println!("[db_conn_list] {} 条连接记录（{}）", conns.len(), path.display());
    println!();
    for c in &conns {
        let head = format!(
            "# {} · id={} · {} @ {}:{}",
            c.name, c.id, c.engine, c.host, c.port
        );
        match c.engine.as_str() {
            "oracle" => {
                let service = c
                    .service_name
                    .clone()
                    .or_else(|| c.database.clone())
                    .unwrap_or_else(|| "<必填：服务名缺失>".to_string());
                let user = c.username.clone().unwrap_or_default();
                // Oracle 字典里 owner 恒为大写（未加引号创建的用户），
                // 生成命令直接给大写 schema 免得查空
                let schema = user.to_uppercase();
                println!("{head}");
                println!(
                    "cargo run --example db_engine_test -- oracle \\\n    --host {} --port {} --user {} --pass {} --service {} --schema {}",
                    shq(&c.host),
                    c.port,
                    shq(&user),
                    shq(c.password.as_deref().unwrap_or("")),
                    shq(&service),
                    shq(&schema),
                );
            }
            "dameng" => {
                let user = c.username.clone().unwrap_or_default();
                let compat = c.compatible_mode.clone().unwrap_or_else(|| "oracle".into());
                println!("{head}");
                println!(
                    "cargo run --example db_engine_test -- dameng \\\n    --host {} --port {} --user {} --pass {} --schema {} --dm-compat {}",
                    shq(&c.host),
                    c.port,
                    shq(&user),
                    shq(c.password.as_deref().unwrap_or("")),
                    shq(&user),
                    shq(&compat),
                );
            }
            other => {
                println!("{head}（{other}：db_engine_test 暂不覆盖，跳过）");
            }
        }
        println!();
    }
    Ok(())
}
