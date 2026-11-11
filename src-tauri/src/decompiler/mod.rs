//! Java 反编译与结构透视模块
//!
//! - `archive`: Jar / War / Zip 流式结构透视与 pom/manifest 元数据提取
//! - `daemon`: Vineflower 常驻守护子进程生命周期管理与 IPC 通信

pub mod archive;
pub mod commands;
pub mod daemon;

use std::sync::LazyLock;
pub use archive::{parse_jar_structure, read_jar_entry_bytes, JarMetadata, JarStructureOverview, JarEntryNode};
pub use commands::{decompiler_open_jar, decompiler_read_entry, decompiler_get_status, decompiler_stop_daemon};
pub use daemon::{DecompilerManager, DecompilerStatus};

/// 全局常驻守护管理器单例
pub static GLOBAL_DECOMPILER_MGR: LazyLock<DecompilerManager> = LazyLock::new(DecompilerManager::new);
