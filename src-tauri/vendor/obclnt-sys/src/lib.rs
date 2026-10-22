//! libobclient（OceanBase Connector/C）的原始 C API 绑定（sys 层，无安全包装）。
//!
//! 库本体见 `vendor/obconnector-c`（MariaDB Connector/C 3.1.x fork，版本
//! 2.2.14）；链接经本 crate 的 build.rs 静态接入（mac 的 .a 自包含、
//! Windows 的 obclnt.lib 需消费方自带 vcpkg ssl/crypto），产物方式见
//! `vendor/obclient-lib/README.md`。安全层（spawn_blocking 包阻塞调用、
//! 行流适配、错误转 String）属 app 侧 db_browser 引擎臂，不在本 crate。
//!
//! 声明口径（对照 `vendor/obconnector-c/include/mysql.h` 逐条抄写，升级
//! 换树时按头文件复核）：
//! - **调用约定**：上游 STDCALL 在 x64/ARM64 的 MSVC 下是单调用约定的
//!   别名（无-op），macOS 下展开为空——统一 `extern "C"` 声明；
//! - **类型映射**：`my_bool` = char、`my_ulonglong` = unsigned long long；
//!   `unsigned long` 用 c_ulong（Windows 4 字节 / macOS 8 字节，与各平台
//!   C 的 long 一致，repr(C) 布局自动对齐）；C enum 统一 c_uint（两个
//!   编译器都不含超 int 值的枚举成员）；
//! - **覆盖面**：仅声明 db_browser 引擎臂所需子集（生命周期/连接/查询/
//!   结果集/错误/信息 + OB 扩展版本），不是 mysql.h 的全量镜像——后续
//!   需要（prepared statement、非阻塞 *_start/*_cont 族、TLS 选项等）
//!   按头文件增量补；
//! - `mysql_library_init/end` 是 `mysql_server_init/end` 的宏别名
//!   （mysql.h:1038），按本体符号声明。
//!
//! OB 私有常量的背景：`CLIENT_SUPPORT_ORACLE_MODE`（1<<27，与 MySQL 8.0
//! 官方语义 CLIENT_QUERY_ATTRIBUTES 同位分叉）在握手时声明客户端懂
//! Oracle 模式；`SERVER_ORACLE_MODE`（server status 第 2 位）是服务端在
//! OK 包里宣告 Oracle 模式租户的标志——连接层据此切 `mysql->oracle_mode`。
//! Rust 侧不镜像 MYSQL 结构体（字段多且随上游版本浮动），Oracle 模式
//! 探测由安全层用 `SELECT 1 FROM DUAL` 语义探针完成（obclient 同型）。

// sys 层约定豁免：全部为不安全的原始 C 绑定（安全性由安全层统一论述）；
// mysql_real_connect 的 8 参签名照抄 C 头文件
#![allow(clippy::missing_safety_doc)]
#![allow(clippy::too_many_arguments)]

use std::ffi::{c_char, c_int, c_uint, c_ulong, c_ulonglong, c_uchar, c_void};

// ── 类型别名与不透明指针 ─────────────────────────────────────────────────

/// C 侧 my_bool（mysql.h:42 typedef char）
pub type MyBool = c_char;
/// C 侧 my_ulonglong（mysql.h:48）
pub type MyUlonglong = c_ulonglong;
/// C 侧 MYSQL_ROW = char **（mysql.h:162）——fetch_row 返回的字符串数组，
/// 空值单元为 NULL 指针（非空串），长度经 mysql_fetch_lengths 取
pub type MysqlRow = *mut *mut c_char;

/// 连接句柄（C 侧 st_mysql 完整定义在 mysql.h，但字段随上游版本浮动，
/// Rust 侧不镜像——经函数 API 访问）
#[repr(C)]
pub struct Mysql {
    _private: [u8; 0],
}

/// 结果集句柄（st_mysql_res，同上不镜像）
#[repr(C)]
pub struct MysqlRes {
    _private: [u8; 0],
}

/// MYSQL_FIELD 的 repr(C) 镜像（mysql.h:122-160 逐字段照抄，含 OB 扩展的
/// owner/elem 族——Oracle 模式对象类型列元数据）。
///
/// 安全层消费 `name`（列名）与 `type`（类型标）即可；其余字段保留完整
/// 布局是为让指针算术/对齐与 C 侧严格一致——**不得删减字段**，上游加字
/// 段时同步镜像。
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct MysqlField {
    pub name: *mut c_char,
    pub org_name: *mut c_char,
    pub table: *mut c_char,
    pub org_table: *mut c_char,
    pub db: *mut c_char,
    pub catalog: *mut c_char,
    pub def: *mut c_char,
    pub length: c_ulong,
    pub max_length: c_ulong,
    pub name_length: c_uint,
    pub org_name_length: c_uint,
    pub table_length: c_uint,
    pub org_table_length: c_uint,
    pub db_length: c_uint,
    pub catalog_length: c_uint,
    pub def_length: c_uint,
    pub flags: c_uint,
    pub decimals: c_uint,
    pub precision: c_uint,
    /// C enum ObClientRoutineParamInOut（mysql.h:116，值 1/2/3）
    pub ob_routine_param_inout: c_uint,
    pub is_implicit_rowid: MyBool,
    pub charsetnr: c_uint,
    pub owner_name: *mut c_uchar,
    pub type_name: *mut c_uchar,
    pub version: c_ulong,
    pub owner_name_length: c_uint,
    pub type_name_length: c_uint,
    /// C enum enum_field_types，见 MYSQL_TYPE_* 常量
    pub type_: c_uint,
    pub elem_owner_name: *mut c_uchar,
    pub elem_type_name: *mut c_uchar,
    pub elem_version: c_ulong,
    pub elem_owner_name_length: c_uint,
    pub elem_type_name_length: c_uint,
    pub elem_type: c_uint,
    pub extension: *mut c_void,
}

// ── 常量（值逐条对照 mariadb_com.h / mysql.h 的宏与枚举序） ─────────────

/// mysql_real_connect 的 clientflag 位（mariadb_com.h:154-188）
pub mod client_flag {
    use super::c_ulong;

    pub const CLIENT_COMPRESS: c_ulong = 32;
    pub const CLIENT_LOCAL_FILES: c_ulong = 128;
    pub const CLIENT_PROTOCOL_41: c_ulong = 512;
    pub const CLIENT_SSL: c_ulong = 2048;
    pub const CLIENT_MULTI_STATEMENTS: c_ulong = 1 << 16;
    pub const CLIENT_MULTI_RESULTS: c_ulong = 1 << 17;
    pub const CLIENT_PLUGIN_AUTH: c_ulong = 1 << 19;
    /// OB 私有位：声明客户端支持 Oracle 模式（握手能力协商用；MySQL 8.0
    /// 官方语义在同一位是 CLIENT_QUERY_ATTRIBUTES——两线缆已分叉）
    pub const CLIENT_SUPPORT_ORACLE_MODE: c_ulong = 1 << 27;
    /// OB 私有位：隐藏 rowid 列回传
    pub const CLIENT_RETURN_HIDDEN_ROWID: c_ulong = 1 << 28;
    /// OB 私有位：LOB 走 locator 形态
    pub const CLIENT_USE_LOB_LOCATOR: c_ulong = 1 << 29;
}

/// OK 包 server status 的 OB 私有位（mariadb_com.h:249）——服务端宣告
/// Oracle 模式租户。C 侧读 mysql->server_status 判定；Rust 侧不镜像
/// MYSQL 结构，安全层用 dual 探针替代，此常量留作协议层参考
pub const SERVER_ORACLE_MODE: c_uint = 4;

/// mysql_options 的 option 参数（mysql.h:218 枚举序，0 起数）
pub mod mysql_option {
    use super::c_uint;

    pub const MYSQL_OPT_CONNECT_TIMEOUT: c_uint = 0;
    pub const MYSQL_SET_CHARSET_NAME: c_uint = 7;
    pub const MYSQL_OPT_READ_TIMEOUT: c_uint = 11;
    pub const MYSQL_OPT_WRITE_TIMEOUT: c_uint = 12;
}

/// enum_field_types（mariadb_com.h:452-487）——列类型标；160+ 为 OB 的
/// Oracle 对象类型扩展（MysqlField.type / elem_type 的取值域）
pub mod field_type {
    use super::c_uint;

    pub const MYSQL_TYPE_DECIMAL: c_uint = 0;
    pub const MYSQL_TYPE_TINY: c_uint = 1;
    pub const MYSQL_TYPE_SHORT: c_uint = 2;
    pub const MYSQL_TYPE_LONG: c_uint = 3;
    pub const MYSQL_TYPE_FLOAT: c_uint = 4;
    pub const MYSQL_TYPE_DOUBLE: c_uint = 5;
    pub const MYSQL_TYPE_NULL: c_uint = 6;
    pub const MYSQL_TYPE_TIMESTAMP: c_uint = 7;
    pub const MYSQL_TYPE_LONGLONG: c_uint = 8;
    pub const MYSQL_TYPE_INT24: c_uint = 9;
    pub const MYSQL_TYPE_DATE: c_uint = 10;
    pub const MYSQL_TYPE_TIME: c_uint = 11;
    pub const MYSQL_TYPE_DATETIME: c_uint = 12;
    pub const MYSQL_TYPE_YEAR: c_uint = 13;
    pub const MYSQL_TYPE_NEWDATE: c_uint = 14;
    pub const MYSQL_TYPE_VARCHAR: c_uint = 15;
    pub const MYSQL_TYPE_BIT: c_uint = 16;
    pub const MYSQL_TYPE_TIMESTAMP2: c_uint = 17;
    pub const MYSQL_TYPE_DATETIME2: c_uint = 18;
    pub const MYSQL_TYPE_TIME2: c_uint = 19;
    // OB Oracle 扩展类型（mariadb_com.h:470-487）
    pub const MYSQL_TYPE_OBJECT: c_uint = 160;
    pub const MYSQL_TYPE_ARRAY: c_uint = 161;
    pub const MYSQL_TYPE_STRUCT: c_uint = 162;
    pub const MYSQL_TYPE_CURSOR: c_uint = 163;
    pub const MYSQL_TYPE_PLARRAY: c_uint = 164;
    pub const MYSQL_TYPE_OB_TIMESTAMP_WITH_TIME_ZONE: c_uint = 200;
    pub const MYSQL_TYPE_OB_TIMESTAMP_WITH_LOCAL_TIME_ZONE: c_uint = 201;
    pub const MYSQL_TYPE_OB_TIMESTAMP_NANO: c_uint = 202;
    pub const MYSQL_TYPE_OB_RAW: c_uint = 203;
    pub const MYSQL_TYPE_OB_INTERVAL_YM: c_uint = 204;
    pub const MYSQL_TYPE_OB_INTERVAL_DS: c_uint = 205;
    pub const MYSQL_TYPE_OB_NUMBER_FLOAT: c_uint = 206;
    pub const MYSQL_TYPE_OB_NVARCHAR2: c_uint = 207;
    pub const MYSQL_TYPE_OB_NCHAR: c_uint = 208;
    pub const MYSQL_TYPE_OB_UROWID: c_uint = 209;
    pub const MYSQL_TYPE_ORA_BLOB: c_uint = 210;
    pub const MYSQL_TYPE_ORA_CLOB: c_uint = 211;
}

// ── C API（签名对照 mysql.h 逐条抄写） ──────────────────────────────────

extern "C" {
    /// 库级一次性初始化（= mysql_library_init；argc/argv/groups 传
    /// 0/NULL/NULL 走默认即可，非 NULL 时指针需比调用存活得久）
    pub fn mysql_server_init(argc: c_int, argv: *mut *mut c_char, groups: *mut *mut c_char) -> c_int;
    /// 库级收尾（= mysql_library_end）
    pub fn mysql_server_end();

    /// 分配/初始化连接句柄；传 NULL 新分配（返回值需 mysql_close 释放），
    /// 失败返回 NULL（内存不足）
    pub fn mysql_init(mysql: *mut Mysql) -> *mut Mysql;
    /// 连接前设选项（option 见 [mysql_option] 模块；arg 的类型随选项而异：
    /// 超时为 *const c_uint、字符集名为 *const c_char）
    pub fn mysql_options(mysql: *mut Mysql, option: c_uint, arg: *const c_void) -> c_int;
    /// 建连（阻塞；用户名/口令直连 OceanBase 时可带 `user@tenant#cluster`
    /// 形态，由服务端/obproxy 解析；clientflag 见 [client_flag] 模块）
    pub fn mysql_real_connect(
        mysql: *mut Mysql,
        host: *const c_char,
        user: *const c_char,
        passwd: *const c_char,
        db: *const c_char,
        port: c_uint,
        unix_socket: *const c_char,
        clientflag: c_ulong,
    ) -> *mut Mysql;
    /// 关闭并释放连接句柄（幂等于 NULL 不安全——只关有效句柄）
    pub fn mysql_close(sock: *mut Mysql);
    /// 会话内切库（MySQL 协议的 USE；Oracle 模式租户无效——安全层用
    /// ALTER SESSION SET CURRENT_SCHEMA 替代）
    pub fn mysql_select_db(mysql: *mut Mysql, db: *const c_char) -> c_int;
    /// 连通性探测（断线且未开 MYSQL_OPT_RECONNECT 时报错）
    pub fn mysql_ping(mysql: *mut Mysql) -> c_int;

    /// 最近一次错误的错误码（CR_* / ER_*；0 = 无错）
    pub fn mysql_errno(mysql: *mut Mysql) -> c_uint;
    /// 最近一次错误的文本（指向句柄内部缓冲，下次调用前失效）
    pub fn mysql_error(mysql: *mut Mysql) -> *const c_char;

    /// 执行语句（q 不要求 NUL 结尾，长度显式给——可含内嵌 NUL 的二进制
    /// 载荷；阻塞，取消语义由安全层的行间检查承担）
    pub fn mysql_real_query(mysql: *mut Mysql, q: *const c_char, length: c_ulong) -> c_int;
    /// 上条语句的结果集列数（0 = 无结果集，DML/DDL 走 affected_rows）
    pub fn mysql_field_count(mysql: *mut Mysql) -> c_uint;
    /// 上条 DML 的影响行数
    pub fn mysql_affected_rows(mysql: *mut Mysql) -> MyUlonglong;
    /// 多结果集：还有下一个结果（存储过程多集/多语句）
    pub fn mysql_more_results(mysql: *mut Mysql) -> MyBool;
    /// 多结果集：推进到下一个（-1 = 无更多；>0 = 错误码；0 = 有结果集或 OK）
    pub fn mysql_next_result(mysql: *mut Mysql) -> c_int;

    /// 取结果集（store 形态：全量缓冲到客户端内存——大结果集用 use 形态）
    pub fn mysql_store_result(mysql: *mut Mysql) -> *mut MysqlRes;
    /// 取结果集（use 形态：逐行从线缆拉，配合 fetch_row 流式消费；结果集
    /// 存活期间不得在同连接上发新语句）
    pub fn mysql_use_result(mysql: *mut Mysql) -> *mut MysqlRes;
    /// 释放结果集（store/use 形态都必须调；遗漏会卡住后续读取）
    pub fn mysql_free_result(result: *mut MysqlRes);
    /// 取下一行（NULL = 结束或出错——用 mysql_errno 区分；行内 NULL 列
    /// 是 NULL 指针单元，长度经 [mysql_fetch_lengths]）
    pub fn mysql_fetch_row(result: *mut MysqlRes) -> MysqlRow;
    /// 当前行各列的字节长（与 fetch_row 配对调用）
    pub fn mysql_fetch_lengths(result: *mut MysqlRes) -> *mut c_ulong;
    /// 全部列元数据（数组长度 = mysql_num_fields；指向结果集内部，随
    /// free_result 失效）
    pub fn mysql_fetch_fields(result: *mut MysqlRes) -> *mut MysqlField;
    /// 结果集列数
    pub fn mysql_num_fields(result: *mut MysqlRes) -> c_uint;

    /// 客户端库版本串（无需句柄；OB 构建下如 "10.5.5"）
    pub fn mysql_get_client_info() -> *const c_char;
    /// 客户端库版本数值（10.5.5 → 100505）
    pub fn mysql_get_client_version() -> c_ulong;
    /// 服务端版本串（需已建连）
    pub fn mysql_get_server_info(mysql: *mut Mysql) -> *mut c_char;
    /// 服务端版本数值
    pub fn mysql_get_server_version(mysql: *mut Mysql) -> c_ulong;
    /// 会话字符集（建连后设 utf8mb4 / OB Oracle 模式的 AL32UTF8 由服务端
    /// 侧字符集体系承接，客户端按 utf8mb4 传输层语义走）
    pub fn mysql_set_character_set(mysql: *mut Mysql, csname: *const c_char) -> c_int;

    // ── OB 扩展 ──────────────────────────────────────────────────────────
    /// libobclient 自身版本（major/minor/patch 出参 + version 文本缓冲），
    /// 与 rpm/libobclient-VER.txt 同源（当前 2.2.14）
    pub fn ob_get_libobclient_version(
        major: *mut c_int,
        minor: *mut c_int,
        patch: *mut c_int,
        version: *mut c_char,
        len: c_int,
    );
}
