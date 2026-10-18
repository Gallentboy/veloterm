//! gRPC TLS 配置库（grpc_tls_profiles 表）的 Tauri 命令层。
//!
//! 校验域名（server_name）是唯一 key：调用侧（gRPC 调试页）只能下拉选择已保存
//! 配置、按 id 引用，CA/客户端证书/私钥不回显前端，由 grpc_call 命令层解析。

use crate::db::{self, DbPool, GrpcTlsProfileRecord};

fn pool() -> Result<DbPool, String> {
    db::global_pool().ok_or_else(|| "failed to get global db pool".to_string())
}

#[tauri::command]
pub fn grpc_tls_list() -> Result<Vec<GrpcTlsProfileRecord>, String> {
    db::list_grpc_tls_profiles(&pool()?)
}

#[tauri::command]
pub fn grpc_tls_save(mut record: GrpcTlsProfileRecord) -> Result<GrpcTlsProfileRecord, String> {
    record.server_name = record.server_name.trim().to_string();
    if record.server_name.is_empty() {
        return Err("校验域名不能为空".into());
    }
    db::save_grpc_tls_profile(&pool()?, record)
}

#[tauri::command]
pub fn grpc_tls_delete(id: usize) -> Result<(), String> {
    db::delete_grpc_tls_profile(&pool()?, id)
}
