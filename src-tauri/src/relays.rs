//! 中继（veloterm_relay）配置库：侧栏「中继」页 CRUD，主机表单与 ZK 页
//! 从这里下拉选择，免去每处重复填地址/端口/传输/凭证。
//! 记录定义在 db.rs（RelayRecord），本模块只做命令封装与引用解析。

use crate::db::{self, DbPool, RelayRecord};
use crate::terminal::ssh::common::SshRelayEndpoint;

fn pool() -> Result<DbPool, String> {
    db::global_pool().ok_or_else(|| "数据库未初始化".to_string())
}

#[tauri::command]
pub fn relay_list() -> Result<Vec<RelayRecord>, String> {
    db::list_relays(&pool()?)
}

/// 新建（id=0 分配）或更新（id>0 按主键覆写）。
#[tauri::command]
pub fn relay_save(record: RelayRecord) -> Result<RelayRecord, String> {
    if record.name.trim().is_empty() {
        return Err("中继名称不能为空".into());
    }
    if record.host.trim().is_empty() {
        return Err("中继地址不能为空".into());
    }
    if record.secret.trim().len() != 64 {
        return Err("Secret 需为 64 位十六进制（veloterm_relay token generate 生成）".into());
    }
    let record = RelayRecord {
        name: record.name.trim().to_string(),
        host: record.host.trim().to_string(),
        secret: record.secret.trim().to_string(),
        ..record
    };
    db::save_relay(&pool()?, record)
}

#[tauri::command]
pub fn relay_delete(id: usize) -> Result<(), String> {
    db::delete_relay(&pool()?, id)
}

/// 测试连接：真实 VLR3 握手探测（可达 + key_id/secret 鉴权 + 传输全验证，
/// 不带目标随即断开），返回耗时毫秒数。表单未保存的草稿值也可测。
#[tauri::command]
pub async fn relay_test(record: RelayRecord) -> Result<u64, String> {
    let endpoint = SshRelayEndpoint {
        host: record.host.trim().to_string(),
        port: record.port,
        transport: record.transport,
        key_id: record.key_id,
        secret: record.secret.trim().to_string(),
        cert_fingerprint: record.cert_fingerprint,
    };
    let started = std::time::Instant::now();
    crate::terminal::ssh::relay_transport::probe_relay(&endpoint).await?;
    Ok(started.elapsed().as_millis() as u64)
}

/// 供各连接组链处引用解析：按 id 取中继端点。
pub fn endpoint_by_id(relay_id: Option<usize>) -> Option<SshRelayEndpoint> {
    let rid = relay_id?;
    let pool = db::global_pool()?;
    db::list_relays(&pool)
        .ok()?
        .into_iter()
        .find(|r| r.id == rid)
        .map(|r| SshRelayEndpoint {
            host: r.host,
            port: r.port,
            transport: r.transport,
            key_id: r.key_id,
            secret: r.secret,
            cert_fingerprint: r.cert_fingerprint,
        })
}
