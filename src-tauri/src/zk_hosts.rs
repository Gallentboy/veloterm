//! ZK 主机持久化配置库：侧栏「ZK」页 CRUD，主机凭证（用户名/密码）与
//! default 凭证引用存 SQLCipher 加密库。记录定义在 db.rs
//! （ZkHostRecord / ZkCredentialRecord），本模块只做命令封装。

use crate::db::{self, DbPool, ZkHostRecord};

fn pool() -> Result<DbPool, String> {
    db::global_pool().ok_or_else(|| "数据库未初始化".to_string())
}

#[tauri::command]
pub fn zk_host_list() -> Result<Vec<ZkHostRecord>, String> {
    db::list_zk_hosts(&pool()?)
}

/// 新建（id=0 分配）或更新（id>0 覆写 meta 并对凭证做集合同步）。
#[tauri::command]
pub fn zk_host_save(record: ZkHostRecord) -> Result<ZkHostRecord, String> {
    let record = ZkHostRecord {
        name: record.name.trim().to_string(),
        endpoints: record.endpoints.trim().to_string(),
        ..record
    };
    db::save_zk_host(&pool()?, record)
}

#[tauri::command]
pub fn zk_host_delete(id: usize) -> Result<(), String> {
    db::delete_zk_host(&pool()?, id)
}
