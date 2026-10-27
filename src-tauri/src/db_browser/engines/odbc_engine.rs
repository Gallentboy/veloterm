//! ODBC 引擎：通用 ODBC 驱动管理器兜底（第 7 引擎），运行时 dlopen 形态。
//!
//! 设计对齐工程的「预览引擎第三方库」铁律（pdfium / libjxl 同源）：
//! **刻意不做链接期依赖**（odbc-sys / odbc-api 用 `#[link]` 属性链
//! libodbc/odbc32，构建机无库即编不过、启动硬依赖库文件）——驱动管理器
//! 在首次使用时 dlopen/LoadLibrary，按名解析约 21 个 ODBC C 入口。
//! 没装驱动管理器的机器应用照常运行，本引擎报「不可用」并附安装指引
//! （Windows 系统自带 odbc32.dll 零安装；macOS `brew install unixodbc`；
//! Linux 各发行版 unixODBC 包）。
//!
//! 移植参考 rustgrid（yisier/rustgrid，crates/rustgrid-odbc，Apache-2.0）
//! 的绑定层，按 veloterm 需求增绑 6 个入口（SQLDataSourcesW / SQLColumnsW /
//! SQLPrimaryKeysW / SQLStatisticsW / SQLSetConnectAttr / SQLEndTran）、
//! 裁掉参数化执行三件（SQLPrepare/SQLBindParameter/SQLExecute——工作台
//! 全字面量 SQL，等可编辑网格落地再补）。
//!
//! 目录全部走 ODBC **标准 Catalog API**（SQLTables/SQLColumns/
//! SQLPrimaryKeys/SQLStatistics）——任何合格驱动都被要求实现，方言收敛
//! 到零（不给每个数据库写适配器）。已知边界：无 DDL 生成与对象定义
//! （表详情只给 列/主键/索引 结构视图）、无执行计划（explain 报不支持）、
//! 取消为行间协作（in-flight 的单次 SQLFetch 阻塞不可中断，读取循环内
//! 逐行查会话取消标志——与官方 oracledb 路径同水位）。
//!
//! 全部 W 变体（UTF-16 进出）：ODBC 宽字符 API 是跨驱动最稳的形态，
//! SQL_C_WCHAR 读单元格天然覆盖 ASCII/GBK/UTF-16 数据（驱动侧负责编码
//! 转换），无需像 Oracle 那样自己处理字符集。

use std::ffi::c_void;
use std::ptr::{null, null_mut};
use std::sync::OnceLock;

use serde::Deserialize;

// ── ABI 类型与常量 ──────────────────────────────────────────────────────────

type Handle = *mut c_void;
type HEnv = Handle;
type Hdbc = Handle;
type Hstmt = Handle;
type SqlReturn = i16;

const SQL_SUCCESS: SqlReturn = 0;
const SQL_SUCCESS_WITH_INFO: SqlReturn = 1;
const SQL_NO_DATA: SqlReturn = 100;

const SQL_HANDLE_ENV: i16 = 1;
const SQL_HANDLE_DBC: i16 = 2;
const SQL_HANDLE_STMT: i16 = 3;
const SQL_ATTR_ODBC_VERSION: i32 = 200;
const SQL_OV_ODBC3: usize = 3;
/// 连接属性：自动提交开关（值 = SQL_AUTOCOMMIT_OFF/ON）
const SQL_ATTR_AUTOCOMMIT: i32 = 102;
const SQL_AUTOCOMMIT_OFF: usize = 0;
const SQL_AUTOCOMMIT_ON: usize = 1;
const SQL_DRIVER_NOPROMPT: u16 = 0;
/// 目录枚举方向（SQLDriversW / SQLDataSourcesW 的首条/续取）
const SQL_FETCH_FIRST: i16 = 2;
const SQL_FETCH_NEXT: i16 = 1;
/// SQLEndTran 的完成类型
const SQL_COMMIT: i16 = 0;
const SQL_ROLLBACK: i16 = 1;
/// SQLStatisticsW 的唯一性过滤：全部索引（含非唯一）
const SQL_INDEX_ALL: i16 = 1;
const SQL_DBMS_NAME: u16 = 17;
const SQL_NTS: i16 = -3;
const SQL_NULL_DATA: isize = -1;
const SQL_NO_TOTAL: isize = -4;
/// 单元格读取的目标 C 类型（UTF-16 文本）
const SQL_C_WCHAR: i16 = -8;

fn is_success(ret: SqlReturn) -> bool {
    ret == SQL_SUCCESS || ret == SQL_SUCCESS_WITH_INFO
}

// ── 原始入口签名（ODBC C ABI，`extern "system"` 对齐 Windows 调用约定）─────

type SqlAllocHandleFn =
    unsafe extern "system" fn(handle_type: i16, input: Handle, output: *mut Handle) -> SqlReturn;
type SqlFreeHandleFn = unsafe extern "system" fn(handle_type: i16, handle: Handle) -> SqlReturn;
type SqlSetEnvAttrFn = unsafe extern "system" fn(
    env: HEnv,
    attribute: i32,
    value: *mut c_void,
    string_length: i32,
) -> SqlReturn;
type SqlSetConnectAttrFn = unsafe extern "system" fn(
    dbc: Hdbc,
    attribute: i32,
    value: *mut c_void,
    string_length: i32,
) -> SqlReturn;
#[allow(clippy::type_complexity)]
type SqlEndTranFn =
    unsafe extern "system" fn(handle_type: i16, handle: Handle, completion: i16) -> SqlReturn;
#[allow(clippy::type_complexity)]
type SqlDriverConnectWFn = unsafe extern "system" fn(
    dbc: Hdbc,
    window: Handle,
    in_string: *const u16,
    in_length: i16,
    out_string: *mut u16,
    out_capacity: i16,
    out_length: *mut i16,
    completion: u16,
) -> SqlReturn;
type SqlDisconnectFn = unsafe extern "system" fn(dbc: Hdbc) -> SqlReturn;
type SqlExecDirectWFn =
    unsafe extern "system" fn(stmt: Hstmt, sql: *const u16, sql_length: i32) -> SqlReturn;
type SqlNumResultColsFn = unsafe extern "system" fn(stmt: Hstmt, count: *mut i16) -> SqlReturn;
#[allow(clippy::type_complexity)]
type SqlDescribeColWFn = unsafe extern "system" fn(
    stmt: Hstmt,
    column: u16,
    name: *mut u16,
    name_capacity: i16,
    name_length: *mut i16,
    data_type: *mut i16,
    column_size: *mut usize,
    decimal_digits: *mut i16,
    nullable: *mut i16,
) -> SqlReturn;
type SqlFetchFn = unsafe extern "system" fn(stmt: Hstmt) -> SqlReturn;
#[allow(clippy::type_complexity)]
type SqlGetDataWFn = unsafe extern "system" fn(
    stmt: Hstmt,
    column: u16,
    target_type: i16,
    target: *mut c_void,
    target_capacity: isize,
    indicator: *mut isize,
) -> SqlReturn;
type SqlRowCountFn = unsafe extern "system" fn(stmt: Hstmt, count: *mut isize) -> SqlReturn;
#[allow(clippy::type_complexity)]
type SqlGetDiagRecWFn = unsafe extern "system" fn(
    handle_type: i16,
    handle: Handle,
    record: i16,
    sql_state: *mut u16,
    native_error: *mut i32,
    message: *mut u16,
    message_capacity: i16,
    message_length: *mut i16,
) -> SqlReturn;
#[allow(clippy::type_complexity)]
type SqlDriversWFn = unsafe extern "system" fn(
    env: HEnv,
    direction: i16,
    description: *mut u16,
    description_capacity: i16,
    description_length: *mut i16,
    attributes: *mut u16,
    attributes_capacity: i16,
    attributes_length: *mut i16,
) -> SqlReturn;
#[allow(clippy::type_complexity)]
type SqlDataSourcesWFn = unsafe extern "system" fn(
    env: HEnv,
    direction: i16,
    dsn: *mut u16,
    dsn_capacity: i16,
    dsn_length: *mut i16,
    description: *mut u16,
    description_capacity: i16,
    description_length: *mut i16,
) -> SqlReturn;
#[allow(clippy::type_complexity)]
type SqlGetInfoWFn = unsafe extern "system" fn(
    dbc: Hdbc,
    info_type: u16,
    info: *mut c_void,
    info_capacity: i16,
    info_length: *mut i16,
) -> SqlReturn;
#[allow(clippy::type_complexity)]
type SqlTablesWFn = unsafe extern "system" fn(
    stmt: Hstmt,
    catalog: *const u16,
    catalog_length: i16,
    schema: *const u16,
    schema_length: i16,
    table: *const u16,
    table_length: i16,
    table_type: *const u16,
    table_type_length: i16,
) -> SqlReturn;
#[allow(clippy::type_complexity)]
type SqlColumnsWFn = unsafe extern "system" fn(
    stmt: Hstmt,
    catalog: *const u16,
    catalog_length: i16,
    schema: *const u16,
    schema_length: i16,
    table: *const u16,
    table_length: i16,
    column: *const u16,
    column_length: i16,
) -> SqlReturn;
#[allow(clippy::type_complexity)]
type SqlPrimaryKeysWFn = unsafe extern "system" fn(
    stmt: Hstmt,
    catalog: *const u16,
    catalog_length: i16,
    schema: *const u16,
    schema_length: i16,
    table: *const u16,
    table_length: i16,
) -> SqlReturn;
#[allow(clippy::type_complexity)]
type SqlStatisticsWFn = unsafe extern "system" fn(
    stmt: Hstmt,
    catalog: *const u16,
    catalog_length: i16,
    schema: *const u16,
    schema_length: i16,
    table: *const u16,
    table_length: i16,
    unique: i16,
    reserved: i16,
) -> SqlReturn;

/// 已加载的 ODBC 驱动管理器及其解析好的入口。
pub(crate) struct OdbcApi {
    _library: libloading::Library,
    environment: HEnv,
    alloc_handle: SqlAllocHandleFn,
    free_handle: SqlFreeHandleFn,
    set_connect_attr: SqlSetConnectAttrFn,
    end_tran: SqlEndTranFn,
    driver_connect: SqlDriverConnectWFn,
    disconnect: SqlDisconnectFn,
    exec_direct: SqlExecDirectWFn,
    num_result_cols: SqlNumResultColsFn,
    describe_col: SqlDescribeColWFn,
    fetch: SqlFetchFn,
    get_data: SqlGetDataWFn,
    row_count: SqlRowCountFn,
    get_diag_rec: SqlGetDiagRecWFn,
    drivers: SqlDriversWFn,
    data_sources: SqlDataSourcesWFn,
    get_info: SqlGetInfoWFn,
    tables: SqlTablesWFn,
    columns: SqlColumnsWFn,
    primary_keys: SqlPrimaryKeysWFn,
    statistics: SqlStatisticsWFn,
}

// SAFETY: environment 由 ODBC 保证线程安全（ODBC 3.x 规范要求环境句柄可被
// 多线程共享），函数指针只是地址；连接/语句句柄从不在本类型里——它们由
// OdbcDbc（Send 包装）持有，且只在 with_odbc 的 take/归还串行化下使用。
unsafe impl Send for OdbcApi {}
unsafe impl Sync for OdbcApi {}

static API: OnceLock<Result<OdbcApi, String>> = OnceLock::new();

/// 进程级驱动管理器单例，首次使用加载。没装时返回带平台安装指引的中文错误
///（连接与表单下拉命令共用，错误只解析一次）。
pub(crate) fn odbc_api() -> Result<&'static OdbcApi, String> {
    match API.get_or_init(load) {
        Ok(api) => Ok(api),
        Err(error) => Err(error.clone()),
    }
}

fn missing_manager_error() -> String {
    if cfg!(target_os = "windows") {
        "系统未找到 ODBC 驱动管理器（odbc32.dll）——Windows 自带该组件，异常缺失请检查系统完整性".to_string()
    } else if cfg!(target_os = "macos") {
        "未找到 ODBC 驱动管理器：macOS 不再自带，请执行 `brew install unixodbc` 后重试（并安装目标数据库的 ODBC 驱动）".to_string()
    } else {
        "未找到 ODBC 驱动管理器：请安装 unixODBC（Debian/Ubuntu 的 unixodbc 包、RHEL 的 unixODBC 包）及目标数据库的 ODBC 驱动后重试".to_string()
    }
}

fn candidate_libraries() -> &'static [&'static str] {
    #[cfg(target_os = "windows")]
    {
        &["odbc32.dll"]
    }
    #[cfg(target_os = "macos")]
    {
        // macOS 不自带驱动管理器，通常来自 Homebrew——其 lib 目录不在默认
        // dyld 搜索路径，故先试裸名再试绝对路径（libodbc = unixODBC；
        // libiodbc = iODBC，其 Homebrew formula 是 keg-only、只有 opt 路径可达）
        &[
            "libodbc.2.dylib",
            "libodbc.dylib",
            "/opt/homebrew/lib/libodbc.2.dylib",
            "/opt/homebrew/lib/libodbc.dylib",
            "/usr/local/lib/libodbc.2.dylib",
            "/usr/local/lib/libodbc.dylib",
            "libiodbc.2.dylib",
            "libiodbc.dylib",
            "/opt/homebrew/opt/libiodbc/lib/libiodbc.2.dylib",
            "/usr/local/opt/libiodbc/lib/libiodbc.2.dylib",
            "/usr/local/lib/libiodbc.2.dylib",
        ]
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        // 包管理器装的 unixODBC 在加载器搜索路径上，裸名即可解析；
        // 绝对路径覆盖手工安装/被迁移的场景
        &[
            "libodbc.so.2",
            "libodbc.so",
            "/usr/lib/x86_64-linux-gnu/libodbc.so.2",
            "/usr/lib/aarch64-linux-gnu/libodbc.so.2",
            "/usr/lib64/libodbc.so.2",
            "/usr/local/lib/libodbc.so.2",
        ]
    }
    #[cfg(not(any(target_os = "windows", unix)))]
    {
        &[]
    }
}

fn load() -> Result<OdbcApi, String> {
    let mut library = None;
    for name in candidate_libraries() {
        // SAFETY: 加载系统库并按文档名解析入口，是本模块的既定用途。
        if let Ok(loaded) = unsafe { libloading::Library::new(*name) } {
            library = Some(loaded);
            break;
        }
    }
    let library = library.ok_or_else(missing_manager_error)?;

    // SAFETY: 每个符号按 ODBC 文档名解析并转换为匹配的签名。
    unsafe {
        let alloc_handle: SqlAllocHandleFn = resolve(&library, b"SQLAllocHandle\0")?;
        let free_handle: SqlFreeHandleFn = resolve(&library, b"SQLFreeHandle\0")?;
        let set_env_attr: SqlSetEnvAttrFn = resolve(&library, b"SQLSetEnvAttr\0")?;
        let set_connect_attr: SqlSetConnectAttrFn = resolve(&library, b"SQLSetConnectAttr\0")?;
        let end_tran: SqlEndTranFn = resolve(&library, b"SQLEndTran\0")?;
        let driver_connect: SqlDriverConnectWFn = resolve(&library, b"SQLDriverConnectW\0")?;
        let disconnect: SqlDisconnectFn = resolve(&library, b"SQLDisconnect\0")?;
        let exec_direct: SqlExecDirectWFn = resolve(&library, b"SQLExecDirectW\0")?;
        let num_result_cols: SqlNumResultColsFn = resolve(&library, b"SQLNumResultCols\0")?;
        let describe_col: SqlDescribeColWFn = resolve(&library, b"SQLDescribeColW\0")?;
        let fetch: SqlFetchFn = resolve(&library, b"SQLFetch\0")?;
        // unixODBC/iODBC 不导出 SQLGetDataW（其宽字符模型经 SQLGetData +
        // SQL_C_WCHAR 目标类型在驱动管理器层转换；Windows odbc32 才有
        // 专门的 W 入口）——入口名无关紧要，TargetType 决定缓冲语义，
        // 故缺失时回落 ANSI 入口（签名相同）。brew unixodbc 2.3.14 实测
        // 导出其余 20 个 W 符号、唯独没有 SQLGetDataW。
        let get_data: SqlGetDataWFn = match resolve(&library, b"SQLGetDataW\0") {
            Ok(f) => f,
            Err(_) => resolve(&library, b"SQLGetData\0")?,
        };
        let row_count: SqlRowCountFn = resolve(&library, b"SQLRowCount\0")?;
        let get_diag_rec: SqlGetDiagRecWFn = resolve(&library, b"SQLGetDiagRecW\0")?;
        let drivers: SqlDriversWFn = resolve(&library, b"SQLDriversW\0")?;
        let data_sources: SqlDataSourcesWFn = resolve(&library, b"SQLDataSourcesW\0")?;
        let get_info: SqlGetInfoWFn = resolve(&library, b"SQLGetInfoW\0")?;
        let tables: SqlTablesWFn = resolve(&library, b"SQLTablesW\0")?;
        let columns: SqlColumnsWFn = resolve(&library, b"SQLColumnsW\0")?;
        let primary_keys: SqlPrimaryKeysWFn = resolve(&library, b"SQLPrimaryKeysW\0")?;
        let statistics: SqlStatisticsWFn = resolve(&library, b"SQLStatisticsW\0")?;

        let mut environment: HEnv = null_mut();
        if !is_success(alloc_handle(SQL_HANDLE_ENV, null_mut(), &mut environment)) {
            return Err("ODBC 环境句柄分配失败".to_string());
        }
        let version = SQL_OV_ODBC3 as *mut c_void;
        if !is_success(set_env_attr(environment, SQL_ATTR_ODBC_VERSION, version, 0)) {
            let _ = free_handle(SQL_HANDLE_ENV, environment);
            return Err("声明 ODBC 3 版本失败（驱动管理器异常）".to_string());
        }

        Ok(OdbcApi {
            _library: library,
            environment,
            alloc_handle,
            free_handle,
            set_connect_attr,
            end_tran,
            driver_connect,
            disconnect,
            exec_direct,
            num_result_cols,
            describe_col,
            fetch,
            get_data,
            row_count,
            get_diag_rec,
            drivers,
            data_sources,
            get_info,
            tables,
            columns,
            primary_keys,
            statistics,
        })
    }
}

unsafe fn resolve<T: Copy>(library: &libloading::Library, name: &[u8]) -> Result<T, String> {
    // SAFETY: 调用方保证 name 是 ODBC 文档符号名。
    let symbol = unsafe { library.get::<T>(name) }.map_err(|error| {
        format!(
            "ODBC 入口缺失 {}：{error}（驱动管理器版本异常）",
            symbol_name(name)
        )
    })?;
    Ok(*symbol)
}

fn symbol_name(name: &[u8]) -> String {
    let end = name.iter().position(|byte| *byte == 0).unwrap_or(name.len());
    String::from_utf8_lossy(name.get(..end).unwrap_or(name)).into_owned()
}

// ── UTF-16 编解码 ────────────────────────────────────────────────────────────

/// 编码为 NUL 结尾的 UTF-16。
fn to_wide(value: &str) -> Vec<u16> {
    let mut units: Vec<u16> = value.encode_utf16().collect();
    units.push(0);
    units
}

/// 非空过滤值编码为 NUL 结尾 UTF-16；None 表示「不过滤」（传 NULL 指针）。
fn optional_wide(value: &str) -> Option<Vec<u16>> {
    (!value.is_empty()).then(|| to_wide(value))
}

/// 解码 NUL 结尾（或定宽）UTF-16 缓冲。
fn from_wide(buffer: &[u16]) -> String {
    let end = buffer.iter().position(|unit| *unit == 0).unwrap_or(buffer.len());
    String::from_utf16_lossy(buffer.get(..end).unwrap_or(buffer))
}

// ── OdbcApi 操作面（全部同步，只在 with_odbc 的 spawn_blocking 里调用）─────

/// 一次语句执行的结果（列名 + 字符串化行 / 受影响行数）。
pub(crate) struct OdbcQueryOutcome {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
    /// DML 的受影响行数（无结果集语句）；驱动不支持时 None
    pub affected: Option<u64>,
    /// 读取循环中命中取消标志（行间协作取消）
    pub cancelled: bool,
    /// 达到 max_rows 客户端截断
    pub truncated: bool,
}

impl OdbcApi {
    /// 句柄最近的诊断记录（SQLGetDiagRecW 聚合，分号连接），无记录时给兜底文案。
    fn error_message(&self, handle_type: i16, handle: Handle) -> String {
        let mut message = String::new();
        let mut record = 1;
        loop {
            let mut state = [0u16; 6];
            let mut native_error: i32 = 0;
            let mut text = vec![0u16; 1024];
            // SAFETY: handle 是该 handle_type 的存活句柄。
            let ret = unsafe {
                (self.get_diag_rec)(
                    handle_type,
                    handle,
                    record,
                    state.as_mut_ptr(),
                    &mut native_error,
                    text.as_mut_ptr(),
                    text.len() as i16,
                    null_mut(),
                )
            };
            if !is_success(ret) {
                break;
            }
            if !message.is_empty() {
                message.push_str("; ");
            }
            let state_text = from_wide(&state);
            let body = from_wide(&text);
            if !state_text.is_empty() {
                message.push_str(&format!("[{state_text}] "));
            }
            message.push_str(&body);
            record += 1;
        }
        if message.is_empty() {
            "ODBC 调用失败（无诊断信息）".to_string()
        } else {
            message
        }
    }

    /// 系统已安装的驱动描述清单（表单的驱动下拉数据源）。
    pub(crate) fn driver_descriptions(&self) -> Result<Vec<String>, String> {
        let mut drivers = Vec::new();
        let mut description = vec![0u16; 1024];
        let mut attributes = vec![0u16; 1024];
        let mut direction = SQL_FETCH_FIRST;
        loop {
            let mut description_length: i16 = 0;
            let mut attributes_length: i16 = 0;
            // SAFETY: environment 存活，缓冲区长度匹配。
            let ret = unsafe {
                (self.drivers)(
                    self.environment,
                    direction,
                    description.as_mut_ptr(),
                    description.len() as i16,
                    &mut description_length,
                    attributes.as_mut_ptr(),
                    attributes.len() as i16,
                    &mut attributes_length,
                )
            };
            if ret == SQL_NO_DATA {
                break;
            }
            if !is_success(ret) {
                return Err(self.error_message(SQL_HANDLE_ENV, self.environment));
            }
            let name = from_wide(&description);
            if !name.is_empty() {
                drivers.push(name);
            }
            direction = SQL_FETCH_NEXT;
        }
        drivers.sort();
        drivers.dedup();
        Ok(drivers)
    }

    /// 系统已配置的数据源（DSN）清单（表单的 DSN 下拉数据源）。
    /// 只回裸 DSN 名——下拉的 value 要无损回填连接表单，驱动描述尾巴
    /// 会污染往返。
    pub(crate) fn data_sources(&self) -> Result<Vec<String>, String> {
        let mut sources = Vec::new();
        let mut dsn = vec![0u16; 1024];
        let mut description = vec![0u16; 1024];
        let mut direction = SQL_FETCH_FIRST;
        loop {
            let mut dsn_length: i16 = 0;
            let mut description_length: i16 = 0;
            // SAFETY: environment 存活，缓冲区长度匹配。
            let ret = unsafe {
                (self.data_sources)(
                    self.environment,
                    direction,
                    dsn.as_mut_ptr(),
                    dsn.len() as i16,
                    &mut dsn_length,
                    description.as_mut_ptr(),
                    description.len() as i16,
                    &mut description_length,
                )
            };
            if ret == SQL_NO_DATA {
                break;
            }
            if !is_success(ret) {
                return Err(self.error_message(SQL_HANDLE_ENV, self.environment));
            }
            let name = from_wide(&dsn);
            if !name.is_empty() {
                sources.push(name);
            }
            direction = SQL_FETCH_NEXT;
        }
        Ok(sources)
    }

    /// 用连接串开连接，返回连接句柄。失败给诊断文案并释放半成品句柄。
    pub(crate) fn connect(&self, connection_string: &str) -> Result<Hdbc, String> {
        let mut dbc: Hdbc = null_mut();
        // SAFETY: 从存活的 environment 分配连接句柄。
        if !is_success(unsafe { (self.alloc_handle)(SQL_HANDLE_DBC, self.environment, &mut dbc) }) {
            return Err(self.error_message(SQL_HANDLE_ENV, self.environment));
        }

        let input = to_wide(connection_string);
        let mut output = vec![0u16; 1024];
        let mut output_length: i16 = 0;
        // SAFETY: dbc 存活，缓冲区长度匹配；SQL_DRIVER_NOPROMPT 禁止弹系统对话框。
        let ret = unsafe {
            (self.driver_connect)(
                dbc,
                null_mut(),
                input.as_ptr(),
                SQL_NTS,
                output.as_mut_ptr(),
                output.len() as i16,
                &mut output_length,
                SQL_DRIVER_NOPROMPT,
            )
        };
        if !is_success(ret) {
            let message = self.error_message(SQL_HANDLE_DBC, dbc);
            // SAFETY: dbc 是本函数分配、尚未外借的句柄。
            unsafe { (self.disconnect)(dbc) };
            unsafe { (self.free_handle)(SQL_HANDLE_DBC, dbc) };
            return Err(format!("ODBC 连接失败：{message}"));
        }
        Ok(dbc)
    }

    /// 断开并释放连接句柄（close 路径）。
    pub(crate) fn disconnect_and_free(&self, dbc: Hdbc) {
        // SAFETY: dbc 是存活句柄，调用后不再使用。
        unsafe {
            let _ = (self.disconnect)(dbc);
            let _ = (self.free_handle)(SQL_HANDLE_DBC, dbc);
        }
    }

    /// 驱动上报的 DBMS 名（版本串展示用；取不到为 None）。
    pub(crate) fn dbms_name(&self, dbc: Hdbc) -> Option<String> {
        let mut buffer = vec![0u16; 256];
        let mut length: i16 = 0;
        // SAFETY: dbc 存活，缓冲区长度匹配。
        let ret = unsafe {
            (self.get_info)(
                dbc,
                SQL_DBMS_NAME,
                buffer.as_mut_ptr().cast(),
                buffer.len() as i16,
                &mut length,
            )
        };
        if is_success(ret) {
            let name = from_wide(&buffer);
            return (!name.is_empty()).then_some(name);
        }
        None
    }

    /// 切换连接级自动提交（事务开关；连接默认 ON，与 ODBC 规范一致）。
    pub(crate) fn set_autocommit(&self, dbc: Hdbc, on: bool) -> Result<(), String> {
        let value = if on { SQL_AUTOCOMMIT_ON } else { SQL_AUTOCOMMIT_OFF };
        // SAFETY: dbc 存活；整数属性按指针宽度传值（ODBC 规范）。
        let ret = unsafe { (self.set_connect_attr)(dbc, SQL_ATTR_AUTOCOMMIT, value as *mut c_void, 0) };
        if is_success(ret) {
            Ok(())
        } else {
            Err(format!("ODBC 切换自动提交失败：{}", self.error_message(SQL_HANDLE_DBC, dbc)))
        }
    }

    /// 提交/回滚当前事务（SQLEndTran，协议级而非 SQL 文本）。
    pub(crate) fn end_tran(&self, dbc: Hdbc, commit: bool) -> Result<(), String> {
        let completion = if commit { SQL_COMMIT } else { SQL_ROLLBACK };
        // SAFETY: dbc 存活。
        let ret = unsafe { (self.end_tran)(SQL_HANDLE_DBC, dbc, completion) };
        if is_success(ret) {
            Ok(())
        } else {
            let action = if commit { "提交" } else { "回滚" };
            Err(format!("ODBC 事务{action}失败：{}", self.error_message(SQL_HANDLE_DBC, dbc)))
        }
    }

    /// 执行语句并读结果（行间协作取消：读取循环逐行查 `cancel` 标志，
    /// 命中即停并标记 cancelled——in-flight 的单次 SQLFetch 阻塞不可中断，
    /// 服务端语句随语句句柄释放而终止）。
    pub(crate) fn exec_query(
        &self,
        dbc: Hdbc,
        sql: &str,
        max_rows: usize,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> Result<OdbcQueryOutcome, String> {
        let stmt = self.alloc_statement(dbc)?;
        let sql = to_wide(sql);
        // SAFETY: stmt 存活，SQL 缓冲 NUL 结尾。
        let ret = unsafe { (self.exec_direct)(stmt, sql.as_ptr(), SQL_NTS as i32) };
        if !is_success(ret) {
            let message = self.error_message(SQL_HANDLE_STMT, stmt);
            self.free_statement(stmt);
            return Err(message);
        }
        let outcome = self.read_outcome(stmt, max_rows, cancel);
        self.free_statement(stmt);
        outcome
    }

    /// 分配语句句柄。
    fn alloc_statement(&self, dbc: Hdbc) -> Result<Hstmt, String> {
        let mut stmt: Hstmt = null_mut();
        // SAFETY: dbc 存活。
        if !is_success(unsafe { (self.alloc_handle)(SQL_HANDLE_STMT, dbc, &mut stmt) }) {
            return Err(self.error_message(SQL_HANDLE_DBC, dbc));
        }
        Ok(stmt)
    }

    fn free_statement(&self, stmt: Hstmt) {
        // SAFETY: stmt 是存活语句句柄，释放后不再使用。
        let _ = unsafe { (self.free_handle)(SQL_HANDLE_STMT, stmt) };
    }

    /// 读取语句的结果（列名 + 行 / 受影响行数），带行间取消与截断。
    fn read_outcome(
        &self,
        stmt: Hstmt,
        max_rows: usize,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> Result<OdbcQueryOutcome, String> {
        use std::sync::atomic::Ordering;

        let mut count: i16 = 0;
        // SAFETY: stmt 存活。
        if !is_success(unsafe { (self.num_result_cols)(stmt, &mut count) }) {
            return Err(self.error_message(SQL_HANDLE_STMT, stmt));
        }
        if count <= 0 {
            // 无结果集（DML/DDL）：affected 走 SQLRowCount
            let mut n: isize = 0;
            // SAFETY: stmt 存活。
            let affected = if is_success(unsafe { (self.row_count)(stmt, &mut n) }) && n >= 0 {
                Some(n as u64)
            } else {
                None
            };
            return Ok(OdbcQueryOutcome {
                columns: Vec::new(),
                rows: Vec::new(),
                affected,
                cancelled: false,
                truncated: false,
            });
        }

        let mut columns = Vec::with_capacity(count as usize);
        for index in 1..=count as u16 {
            let mut name = vec![0u16; 256];
            let mut name_length: i16 = 0;
            let mut data_type: i16 = 0;
            let mut column_size: usize = 0;
            let mut decimal_digits: i16 = 0;
            let mut nullable: i16 = 0;
            // SAFETY: stmt 存活，出参指针有效。
            let ret = unsafe {
                (self.describe_col)(
                    stmt,
                    index,
                    name.as_mut_ptr(),
                    name.len() as i16,
                    &mut name_length,
                    &mut data_type,
                    &mut column_size,
                    &mut decimal_digits,
                    &mut nullable,
                )
            };
            let _ = (column_size, nullable);
            columns.push(if is_success(ret) {
                from_wide(&name)
            } else {
                String::new()
            });
        }

        let mut rows: Vec<Vec<String>> = Vec::new();
        let mut truncated = false;
        let mut cancelled = false;
        loop {
            if cancel.load(Ordering::Relaxed) {
                cancelled = true;
                break;
            }
            if rows.len() >= max_rows {
                truncated = true;
                break;
            }
            // SAFETY: stmt 存活。
            let fetch = unsafe { (self.fetch)(stmt) };
            if fetch == SQL_NO_DATA {
                break;
            }
            if !is_success(fetch) {
                return Err(self.error_message(SQL_HANDLE_STMT, stmt));
            }
            let mut values = Vec::with_capacity(count as usize);
            for index in 1..=count as u16 {
                values.push(self.read_cell(stmt, index)?);
            }
            rows.push(values);
        }
        Ok(OdbcQueryOutcome {
            columns,
            rows,
            affected: None,
            cancelled,
            truncated,
        })
    }

    /// 读一格（SQLGetDataW 分块：超过 1024 UTF-16 单元的值按 SQL_SUCCESS_WITH_INFO
    /// 续块拼接；NULL 按 SQL_NULL_DATA 判定，渲染 "NULL" 与其它引擎一致）。
    fn read_cell(&self, stmt: Hstmt, index: u16) -> Result<String, String> {
        let mut units: Vec<u16> = Vec::new();
        let mut chunk = vec![0u16; 1024];
        loop {
            let mut indicator: isize = 0;
            // SAFETY: stmt 存活，chunk 按 UTF-16 字节长度传容量。
            let ret = unsafe {
                (self.get_data)(
                    stmt,
                    index,
                    SQL_C_WCHAR,
                    chunk.as_mut_ptr().cast(),
                    (chunk.len() * 2) as isize,
                    &mut indicator,
                )
            };
            if ret == SQL_NO_DATA {
                break;
            }
            if !is_success(ret) {
                return Err(self.error_message(SQL_HANDLE_STMT, stmt));
            }
            if indicator == SQL_NULL_DATA {
                return Ok("NULL".to_string());
            }
            let taken = if indicator == SQL_NO_TOTAL {
                chunk.iter().position(|unit| *unit == 0).unwrap_or(chunk.len())
            } else {
                ((indicator as usize) / 2).min(chunk.len().saturating_sub(1))
            };
            units.extend_from_slice(chunk.get(..taken).unwrap_or(chunk.as_slice()));
            if ret == SQL_SUCCESS {
                break;
            }
        }
        Ok(String::from_utf16_lossy(&units))
    }

    /// SQLTables 目录查询（全空过滤 = 枚举；table_type 如 "TABLE" / "VIEW"）。
    /// 返回标准五列行集（TABLE_CAT/TABLE_SCHEM/TABLE_NAME/TABLE_TYPE/REMARKS）。
    pub(crate) fn tables(
        &self,
        dbc: Hdbc,
        catalog: &str,
        schema: &str,
        table: &str,
        table_type: &str,
    ) -> Result<(Vec<String>, Vec<Vec<String>>), String> {
        let stmt = self.alloc_statement(dbc)?;
        let catalog = optional_wide(catalog);
        let schema = optional_wide(schema);
        let table = optional_wide(table);
        let table_type = optional_wide(table_type);
        let length = |value: &Option<Vec<u16>>| if value.is_some() { SQL_NTS } else { 0 };
        // SAFETY: stmt 存活；每个过滤值要么是 NUL 结尾缓冲、要么是 NULL 指针
        //（ODBC 语义 = 该字段不过滤）。
        let ret = unsafe {
            (self.tables)(
                stmt,
                catalog.as_ref().map_or(null(), |value| value.as_ptr()),
                length(&catalog),
                schema.as_ref().map_or(null(), |value| value.as_ptr()),
                length(&schema),
                table.as_ref().map_or(null(), |value| value.as_ptr()),
                length(&table),
                table_type.as_ref().map_or(null(), |value| value.as_ptr()),
                length(&table_type),
            )
        };
        if !is_success(ret) {
            let message = self.error_message(SQL_HANDLE_STMT, stmt);
            self.free_statement(stmt);
            return Err(message);
        }
        let result = self.read_all(stmt);
        self.free_statement(stmt);
        result
    }

    /// SQLColumns 目录查询（列清单；全空过滤 = 全部列）。
    pub(crate) fn columns_catalog(
        &self,
        dbc: Hdbc,
        catalog: &str,
        schema: &str,
        table: &str,
    ) -> Result<(Vec<String>, Vec<Vec<String>>), String> {
        let stmt = self.alloc_statement(dbc)?;
        let catalog = optional_wide(catalog);
        let schema = optional_wide(schema);
        let table = optional_wide(table);
        let length = |value: &Option<Vec<u16>>| if value.is_some() { SQL_NTS } else { 0 };
        // SAFETY: 同 tables。
        let ret = unsafe {
            (self.columns)(
                stmt,
                catalog.as_ref().map_or(null(), |value| value.as_ptr()),
                length(&catalog),
                schema.as_ref().map_or(null(), |value| value.as_ptr()),
                length(&schema),
                table.as_ref().map_or(null(), |value| value.as_ptr()),
                length(&table),
                null(),
                0,
            )
        };
        if !is_success(ret) {
            let message = self.error_message(SQL_HANDLE_STMT, stmt);
            self.free_statement(stmt);
            return Err(message);
        }
        let result = self.read_all(stmt);
        self.free_statement(stmt);
        result
    }

    /// SQLPrimaryKeys 目录查询（表的主键列，按 KEY_SEQ 排序返回）。
    pub(crate) fn primary_keys(
        &self,
        dbc: Hdbc,
        catalog: &str,
        schema: &str,
        table: &str,
    ) -> Result<Vec<Vec<String>>, String> {
        let stmt = self.alloc_statement(dbc)?;
        let catalog = optional_wide(catalog);
        let schema = optional_wide(schema);
        let table = optional_wide(table);
        let length = |value: &Option<Vec<u16>>| if value.is_some() { SQL_NTS } else { 0 };
        // SAFETY: 同 tables。
        let ret = unsafe {
            (self.primary_keys)(
                stmt,
                catalog.as_ref().map_or(null(), |value| value.as_ptr()),
                length(&catalog),
                schema.as_ref().map_or(null(), |value| value.as_ptr()),
                length(&schema),
                table.as_ref().map_or(null(), |value| value.as_ptr()),
                length(&table),
            )
        };
        if !is_success(ret) {
            let message = self.error_message(SQL_HANDLE_STMT, stmt);
            self.free_statement(stmt);
            return Err(message);
        }
        let result = self.read_all(stmt);
        self.free_statement(stmt);
        result.map(|(_, rows)| rows)
    }

    /// SQLStatistics 目录查询（表的全部索引与索引列）。
    pub(crate) fn statistics(
        &self,
        dbc: Hdbc,
        catalog: &str,
        schema: &str,
        table: &str,
    ) -> Result<Vec<Vec<String>>, String> {
        let stmt = self.alloc_statement(dbc)?;
        let catalog = optional_wide(catalog);
        let schema = optional_wide(schema);
        let table = optional_wide(table);
        let length = |value: &Option<Vec<u16>>| if value.is_some() { SQL_NTS } else { 0 };
        // SAFETY: 同 tables。
        let ret = unsafe {
            (self.statistics)(
                stmt,
                catalog.as_ref().map_or(null(), |value| value.as_ptr()),
                length(&catalog),
                schema.as_ref().map_or(null(), |value| value.as_ptr()),
                length(&schema),
                table.as_ref().map_or(null(), |value| value.as_ptr()),
                length(&table),
                SQL_INDEX_ALL,
                0,
            )
        };
        if !is_success(ret) {
            let message = self.error_message(SQL_HANDLE_STMT, stmt);
            self.free_statement(stmt);
            return Err(message);
        }
        let result = self.read_all(stmt);
        self.free_statement(stmt);
        result.map(|(_, rows)| rows)
    }

    /// 读全量行集（目录查询专用：小结果集、不取消、不截断）。
    fn read_all(&self, stmt: Hstmt) -> Result<(Vec<String>, Vec<Vec<String>>), String> {
        let never = std::sync::atomic::AtomicBool::new(false);
        let outcome = self.read_outcome(stmt, usize::MAX, &never)?;
        Ok((outcome.columns, outcome.rows))
    }
}

// ── 连接串构造（三态优先级：raw > dsn > driver）────────────────────────────

/// ODBC 引擎的 extra_options 结构（db_connections.extra_options JSON，
/// RedisExtraOptions 同模式：camelCase、非法 JSON 静默回默认）。
#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OdbcExtraOptions {
    /// 连接方式：dsn（默认）| driver | raw
    #[serde(default)]
    pub connection_type: String,
    #[serde(default)]
    pub dsn: String,
    #[serde(default)]
    pub driver: String,
    #[serde(default)]
    pub connection_string: String,
}

/// 解析 extra_options（非法/空 JSON 回默认 = DSN 模式，老记录兼容语义）。
pub(crate) fn odbc_extra_options(raw: Option<&str>) -> OdbcExtraOptions {
    serde_json::from_str(raw.unwrap_or("")).ok().unwrap_or_default()
}

/// ODBC 连接串值转义：含 `[]{}(),;?*=!@` 或首尾空格时包 `{}`，包内 `}` 翻倍
///（ODBC 规范的连接串转义规则——不转义的 `;` 会被当成分隔符截断值）。
fn odbc_escape_value(value: &str) -> String {
    if value.is_empty() {
        return "{}".to_string();
    }
    let needs = value.chars().any(|c| {
        matches!(c, '[' | ']' | '{' | '}' | '(' | ')' | ',' | ';' | '?' | '*' | '=' | '!' | '@')
    }) || value.starts_with(' ')
        || value.ends_with(' ');
    if needs {
        format!("{{{}}}", value.replace('}', "}}"))
    } else {
        value.to_string()
    }
}

/// 按三态优先级构造连接串：raw 透传（账密非空且串里没写则追加）>
/// DSN + 账密 > 驱动直连（Driver + Server/Port + 账密）。
/// 密码为空不写 PWD（DSN 里常已带凭据，空 PWD 反而覆盖掉正确值）。
pub(crate) fn odbc_connection_string(
    extra: &OdbcExtraOptions,
    host: &str,
    port: u16,
    username: &str,
    password: &str,
) -> Result<String, String> {
    let conn_type = extra.connection_type.trim();
    if conn_type == "raw" || (conn_type.is_empty() && !extra.connection_string.is_empty() && extra.dsn.is_empty() && extra.driver.is_empty())
    {
        let raw = extra.connection_string.trim();
        if raw.is_empty() {
            return Err("ODBC 连接方式为「自定义连接串」但没有填写连接串".to_string());
        }
        let mut out = raw.to_string();
        // 串里已显式写了凭据就不追加（大小写不敏感探测，防重复键）
        let lower = out.to_ascii_lowercase();
        if !username.is_empty() && !lower.contains("uid=") && !lower.contains("user id=") {
            out.push_str(&format!(
                ";UID={};PWD={}",
                odbc_escape_value(username),
                odbc_escape_value(password)
            ));
        }
        return Ok(out);
    }

    let mut parts: Vec<String> = Vec::new();
    if conn_type == "driver" {
        let driver = extra.driver.trim();
        if driver.is_empty() {
            return Err("ODBC 连接方式为「驱动直连」但没有选择驱动".to_string());
        }
        if host.trim().is_empty() {
            return Err("ODBC 驱动直连需要填写主机地址".to_string());
        }
        parts.push(format!("Driver={}", odbc_escape_value(driver)));
        parts.push(format!("Server={}", odbc_escape_value(host.trim())));
        parts.push(format!("Port={port}"));
    } else {
        let dsn = extra.dsn.trim();
        if dsn.is_empty() {
            return Err("ODBC 连接方式为「DSN」但没有选择数据源".to_string());
        }
        parts.push(format!("DSN={}", odbc_escape_value(dsn)));
    }
    if !username.is_empty() {
        parts.push(format!("UID={}", odbc_escape_value(username)));
        if !password.is_empty() {
            parts.push(format!("PWD={}", odbc_escape_value(password)));
        }
    }
    Ok(parts.join(";"))
}

/// 行取值：目录查询的行按列取字符串（越界/NULL 判定统一）。
#[inline]
pub(crate) fn odbc_row_string(row: &[String], i: usize) -> String {
    row.get(i).cloned().unwrap_or_else(|| "NULL".to_string())
}

/// 单元格是否为 SQL NULL 形态（"NULL" 字面量；目录逻辑判断用）。
#[inline]
pub(crate) fn odbc_is_null(row: &[String], i: usize) -> bool {
    row.get(i).map(|v| v == "NULL" || v.is_empty()).unwrap_or(true)
}

// ── 引擎句柄与目录辅助 ──────────────────────────────────────────────────────

/// ODBC 连接句柄的 Send 包装：裸指针无线程语义，但 veloterm 是单会话单
/// 连接模型，句柄只在 with_odbc 的 take→spawn_blocking→归还串行化下触碰
///（同一时刻仅一个线程持有），Send 声明由此成立（rustgrid 同型）。
/// Drop 兜底断开：建连超时被丢弃的孤儿闭包、JoinError 路径都靠它释放句柄。
pub struct OdbcDbc(pub(crate) Hdbc);

unsafe impl Send for OdbcDbc {}

impl Drop for OdbcDbc {
    fn drop(&mut self) {
        if !self.0.is_null() {
            if let Ok(api) = odbc_api() {
                api.disconnect_and_free(self.0);
            }
            self.0 = std::ptr::null_mut();
        }
    }
}

/// ODBC 引擎：连接句柄 + 会话取消标志（bind_cancel_flag 注入，读取循环
/// 逐行轮询——SQLite progress handler 的同型协作取消）。
pub struct OdbcEngine {
    pub(crate) dbc: OdbcDbc,
    pub(crate) cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl OdbcEngine {
    /// 建连（with_odbc 之外只在 connect 阶段用一次）。返回引擎 + 版本串
    /// （DBMS 名，驱动上报）。
    pub(crate) fn connect(connection_string: &str) -> Result<(Self, String), String> {
        let api = odbc_api()?;
        let dbc = api.connect(connection_string)?;
        let version = api.dbms_name(dbc).unwrap_or_else(|| "ODBC".to_string());
        Ok((
            Self {
                dbc: OdbcDbc(dbc),
                cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            },
            version,
        ))
    }

    pub(crate) fn bind_cancel(&mut self, flag: std::sync::Arc<std::sync::atomic::AtomicBool>) {
        self.cancel = flag;
    }
}

/// 目录（catalog/database）清单：SQLTables 全量聚合 TABLE_CAT 去重。
/// 驱动不填 TABLE_CAT 时（如 MySQL ODBC 驱动）回落 DBMS 名单根——此时
/// 该名作为 catalog 过滤会得空树，属已知边界（主流 ODBC 驱动都正确填充；
/// 原生 MySQL 引擎另有专用臂，用户没有理由走 ODBC 连 MySQL）。
pub(crate) fn odbc_catalog_names(engine: &mut OdbcEngine) -> Result<Vec<String>, String> {
    let api = odbc_api()?;
    let (_, rows) = api.tables(engine.dbc.0, "", "", "", "")?;
    let mut names: Vec<String> = rows
        .iter()
        .filter_map(|r| r.first().cloned())
        .filter(|v| !v.is_empty() && v != "NULL")
        .collect();
    names.sort();
    names.dedup();
    if names.is_empty() {
        names.push(api.dbms_name(engine.dbc.0).unwrap_or_else(|| "ODBC".to_string()));
    }
    Ok(names)
}

/// 拆限定名："SCHEMA.TABLE" → (Some("SCHEMA"), "TABLE")；裸名 → (None, name)。
/// 目录对象清单的显示名可能带 schema 前缀，列/主键/索引查询要拆开过滤。
pub(crate) fn odbc_split_qualified(name: &str) -> (Option<String>, String) {
    match name.split_once('.') {
        Some((s, t)) if !s.is_empty() && !t.is_empty() => {
            (Some(s.to_string()), t.to_string())
        }
        _ => (None, name.to_string()),
    }
}

/// SQLTables 的 TABLE_TYPE 值是否为视图（部分驱动返回 "SYSTEM VIEW" 等
/// 变体，统一按 VIEW 归类）。
pub(crate) fn odbc_is_view_type(table_type: &str) -> bool {
    table_type.eq_ignore_ascii_case("VIEW") || table_type.eq_ignore_ascii_case("SYSTEM VIEW")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 连接串三态构造与转义：DSN/驱动直连/自定义串的优先级、特殊字符
    /// 密码的 {} 包裹与 } 翻倍、空密码不写 PWD、raw 串已含凭据不追加
    #[test]
    fn odbc_connection_string_modes() {
        let mk = |t: &str, dsn: &str, driver: &str, raw: &str| OdbcExtraOptions {
            connection_type: t.to_string(),
            dsn: dsn.to_string(),
            driver: driver.to_string(),
            connection_string: raw.to_string(),
        };

        // DSN 模式（默认）
        let s = odbc_connection_string(&mk("", "mydb", "", ""), "", 0, "u", "p").unwrap();
        assert_eq!(s, "DSN=mydb;UID=u;PWD=p");
        // 空密码不写 PWD
        let s = odbc_connection_string(&mk("dsn", "mydb", "", ""), "", 0, "u", "").unwrap();
        assert_eq!(s, "DSN=mydb;UID=u");
        // DSN 缺失报错
        assert!(odbc_connection_string(&mk("dsn", " ", "", ""), "", 0, "u", "p").is_err());

        // 驱动直连
        let s = odbc_connection_string(&mk("driver", "", "PostgreSQL Unicode", ""), "db.host", 5432, "u", "p")
            .unwrap();
        assert_eq!(s, "Driver=PostgreSQL Unicode;Server=db.host;Port=5432;UID=u;PWD=p");
        // 驱动缺失/主机缺失报错
        assert!(odbc_connection_string(&mk("driver", "", " ", ""), "h", 1, "u", "p").is_err());
        assert!(odbc_connection_string(&mk("driver", "", "drv", ""), " ", 1, "u", "p").is_err());

        // 自定义串透传 + 凭据追加（串里没写才追加）
        let s = odbc_connection_string(&mk("raw", "", "", "Server=x;Database=y"), "", 0, "u", "p")
            .unwrap();
        assert_eq!(s, "Server=x;Database=y;UID=u;PWD=p");
        let s = odbc_connection_string(&mk("raw", "", "", "DSN=zz;UID=a"), "", 0, "u", "p").unwrap();
        assert_eq!(s, "DSN=zz;UID=a");
        // raw 空串报错
        assert!(odbc_connection_string(&mk("raw", "", "", "  "), "", 0, "u", "p").is_err());
    }

    /// ODBC 连接串转义规则：分隔符/括号类字符包 {}、包内 } 翻倍、
    /// 首尾空格包裹、普通值原样
    #[test]
    fn odbc_escape_value_rules() {
        assert_eq!(odbc_escape_value("plain"), "plain");
        assert_eq!(odbc_escape_value("a;b"), "{a;b}");
        assert_eq!(odbc_escape_value("p@ss}word"), "{p@ss}}word}");
        assert_eq!(odbc_escape_value(" leading"), "{ leading}");
        assert_eq!(odbc_escape_value(""), "{}");
        assert_eq!(odbc_escape_value("中文密码"), "中文密码");
    }

    /// UTF-16 往返与 NUL 语义
    #[test]
    fn wide_round_trip() {
        let text = "中文 abc;";
        let wide = to_wide(text);
        assert_eq!(wide.last(), Some(&0));
        assert_eq!(from_wide(&wide), text);
        assert_eq!(from_wide(&[0x0041, 0]), "A");
        // 无 NUL 的定宽缓冲全量解码
        assert_eq!(from_wide(&[0x0042, 0x0043]), "BC");
    }

    /// extra_options 解析：合法 camelCase 取值、非法/空回 DSN 默认
    #[test]
    fn odbc_extra_options_parse() {
        let extra = odbc_extra_options(Some(
            r#"{"connectionType":"driver","driver":"DM8 ODBC DRIVER"}"#,
        ));
        assert_eq!(extra.connection_type, "driver");
        assert_eq!(extra.driver, "DM8 ODBC DRIVER");

        let extra = odbc_extra_options(Some("{oops"));
        assert_eq!(extra.connection_type, "");
        assert_eq!(extra.dsn, "");

        let extra = odbc_extra_options(None);
        assert_eq!(extra.connection_type, "");
    }

    /// 行取值与 NULL 判定的边界
    #[test]
    fn odbc_row_helpers() {
        let row = vec!["CAT".to_string(), "NULL".to_string()];
        assert_eq!(odbc_row_string(&row, 0), "CAT");
        assert_eq!(odbc_row_string(&row, 1), "NULL");
        assert_eq!(odbc_row_string(&row, 9), "NULL");
        assert!(!odbc_is_null(&row, 0));
        assert!(odbc_is_null(&row, 1));
        assert!(odbc_is_null(&row, 9));
    }

    /// 驱动管理器缺失的文案按平台给指引（macOS 本机无管理器正好走这条路径）
    #[test]
    fn missing_manager_message_has_platform_hint() {
        let msg = missing_manager_error();
        if cfg!(target_os = "macos") {
            assert!(msg.contains("unixodbc"), "macOS 文案应含安装指引：{msg}");
        } else if cfg!(target_os = "windows") {
            assert!(msg.contains("odbc32"), "Windows 文案应指明组件：{msg}");
        } else {
            assert!(msg.contains("unixODBC"), "Linux 文案应含包名：{msg}");
        }
    }

    /// 优雅不可用路径（真 dlopen）：没装驱动管理器的环境拿到带指引的错误，
    /// 装了的环境正常加载——两态自洽（OnceLock 缓存，进程内只解析一次）。
    /// macOS 本机（无 unixODBC/iODBC）命中 Err 分支；Windows（odbc32 系统
    /// 自带）命中 Ok 分支。
    #[test]
    fn odbc_api_graceful_unavailable_or_loaded() {
        match odbc_api() {
            Ok(api) => {
                // 加载成功则枚举接口可用（无驱动时返回空清单而非报错）
                let _ = api.driver_descriptions();
            }
            Err(e) => {
                assert!(
                    e.contains("unixodbc") || e.contains("unixODBC") || e.contains("odbc32"),
                    "缺失错误应含平台安装指引：{e}"
                );
            }
        }
    }
}
