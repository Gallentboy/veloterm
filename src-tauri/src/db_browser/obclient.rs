//! OceanBase Oracle 模式租户的连接安全包装：libobclient（OceanBase
//! Connector/C）阻塞 C API 的 Rust 侧薄封装，sys 层见 vendor/obclnt-sys。
//!
//! 定位与先例：与 engines.rs 的 with_oracle（vendor rust-oracle 同步 API）
//! 同型——C 调用全部阻塞，engines 层经 with_obclient 把每次操作移入
//! spawn_blocking（连接取出/归还，会话层 tokio::Mutex 串行独占）。
//!
//! 取消语义边界：取消粒度 = 语句间（会话层 AtomicBool 在语句边界生效，
//! 与 Oracle 同步臂一致——exec_streaming 只拿到 Notify、无法在阻塞线程内
//! 轮询同步标志）；行流与 C 调用中不可打断（fetch 阻塞在线缆读上无协作
//! 取消点）。语句卡服务端长执行时会话层超时置废连接：引擎置 Closed，
//! 后台阻塞任务收尾时 Drop 兜底 mysql_close（连接不泄漏）。
//!
//! TLS P0 仅 disable：libobclient 侧不配 TLS（明文连接，ssl_mode=disable
//! 语义），ssl_mode 非 disable 时连接层（engines.rs）直接报错提示——不做
//! 静默明文回退；OB MySQL 模式走 mysql_async 臂不受此限。
//!
//! 安全性依据（unsafe impl Send）：句柄仅在会话层的 tokio::Mutex 内使用
//!（同一时刻只有一个线程持有 &mut），跨线程移动安全、无并发共享——与
//! oracle::Connection 的 Send 保证同构。libobclient 句柄非线程绑定
//!（无 thread-local 状态依赖），Send 声明成立。

// 结果集句柄（*mut MysqlRes）是不透明 C 句柄：只在本模块产生（exec /
// take_use_result）并原样传回 C 函数，Rust 侧从不解引用——clippy 的
// raw 指针参数告警对这种 opaque handle 形态是误报，模块级豁免（sys 层
// 同型豁免先例）
#![allow(clippy::not_unsafe_ptr_arg_deref)]

use std::ffi::{c_char, c_uint, c_ulong, CStr, CString};
use std::ptr;
use std::sync::OnceLock;

use obclnt_sys as ffi;

/// 进程级库初始化（mysql_server_init 一次性 + 结果缓存）：libobclient 的
/// 全局状态（错误消息缓冲、字符集表、插件注册）需在首个句柄前就绪。
/// 永不调 mysql_server_end——进程生命周期内库状态常驻（与 C 宿主惯例如
/// mysql_init 隐式初始化共存，收尾反而在多会话场景下危险）。
static LIB_INIT: OnceLock<Result<(), String>> = OnceLock::new();

fn ensure_lib_init() -> Result<(), String> {
    LIB_INIT
        .get_or_init(|| {
            // argc/argv/groups 传 0/NULL/NULL 走默认初始化（sys 层签名注）
            let rc = unsafe { ffi::mysql_server_init(0, ptr::null_mut(), ptr::null_mut()) };
            if rc != 0 {
                Err(format!("libobclient 初始化失败（错误码 {rc}）"))
            } else {
                Ok(())
            }
        })
        .clone()
}

/// OB Oracle 模式连接句柄。Drop 兜底 mysql_close（含优雅断连包）——
/// spawn_blocking 任务被丢弃/panic 展开时连接也能释放。
#[derive(Debug)]
pub struct ObConn(*mut ffi::Mysql);

// Send 依据见模块头「安全性依据」：会话层互斥串行独占，无并发共享
unsafe impl Send for ObConn {}

/// 低层执行原语的返回形态：无结果集（DML/DDL）给影响行数，有结果集给
/// use 形态句柄（逐行从线缆拉，须配对 free_result / drain_result）。
pub enum ObExecResult {
    Affected(u64),
    UseResult(*mut ffi::MysqlRes),
}

/// CString 构造 + 内嵌 NUL 校验（C 字符串以 NUL 结尾，内嵌 NUL 会静默
/// 截断参数——用户名/密码被截成前半段是难排查的认证错，直接报错）
fn cstr_checked(label: &str, s: &str) -> Result<CString, String> {
    CString::new(s).map_err(|_| format!("OceanBase {label}含内嵌 NUL 字节，无法用于 C 接口"))
}

impl ObConn {
    /// 建连（阻塞 C 调用，调用方须在 spawn_blocking 内）：
    /// 1. 进程级库初始化；2. mysql_init + 选项（utf8mb4 字符集、连接超时）；
    /// 3. mysql_real_connect（client_flag 声明懂 Oracle 模式 + 多语句/多结果
    ///    集/插件认证）；4. 租户模式检测 `SHOW VARIABLES LIKE
    ///    'ob_compatibility_mode'`（取 Value 列判 ORACLE——SHOW 语句两种
    ///    模式租户都接受，探测本身不产生方言报错；不通过时只报「检测
    ///    失败」不透传服务端错误文本）；5. database 参数 = 初始 schema
    ///    语义（不传给 real_connect——其内部走 USE，对 Oracle 模式租户
    ///    无效），非空则连接成功后 ALTER SESSION SET CURRENT_SCHEMA。
    ///
    /// 返回 (连接, 服务端版本串)。
    pub fn connect(
        host: &str,
        port: u16,
        user: &str,
        password: &str,
        database: Option<&str>,
        connect_timeout_secs: u32,
    ) -> Result<(ObConn, String), String> {
        ensure_lib_init()?;
        let c_host = cstr_checked("主机地址", host)?;
        let c_user = cstr_checked("用户名", user)?;
        let c_pass = cstr_checked("密码", password)?;
        let c_db = match database {
            Some(db) => Some(cstr_checked("初始 schema", db)?),
            None => None,
        };

        let mysql = unsafe { ffi::mysql_init(ptr::null_mut()) };
        if mysql.is_null() {
            return Err("OceanBase 连接失败：分配连接句柄失败（内存不足）（错误码 0）".to_string());
        }
        let mut conn = ObConn(mysql);
        // mysql_options 失败只可能因参数非法（编译期常量已锁定），忽略返回值
        unsafe {
            let charset: *const c_char = c"utf8mb4".as_ptr();
            ffi::mysql_options(conn.0, ffi::mysql_option::MYSQL_SET_CHARSET_NAME, charset.cast());
            let timeout: c_uint = connect_timeout_secs;
            ffi::mysql_options(
                conn.0,
                ffi::mysql_option::MYSQL_OPT_CONNECT_TIMEOUT,
                ptr::from_ref(&timeout).cast(),
            );
            let client_flag = ffi::client_flag::CLIENT_PROTOCOL_41
                | ffi::client_flag::CLIENT_MULTI_STATEMENTS
                | ffi::client_flag::CLIENT_MULTI_RESULTS
                | ffi::client_flag::CLIENT_PLUGIN_AUTH
                | ffi::client_flag::CLIENT_SUPPORT_ORACLE_MODE;
            let connected = ffi::mysql_real_connect(
                conn.0,
                c_host.as_ptr(),
                c_user.as_ptr(),
                c_pass.as_ptr(),
                // db 传 NULL：初始 schema 走连接后的 ALTER SESSION（见函数头注）
                c_db.as_ref().map_or(ptr::null(), |d| d.as_ptr()),
                c_uint::from(port),
                ptr::null(),
                client_flag,
            );
            if connected.is_null() {
                return Err(format!("OceanBase 连接失败：{}", conn.last_error()));
            }
        }

        // 租户模式检测：SHOW VARIABLES LIKE 'ob_compatibility_mode' 的
        // Value 列（MYSQL / ORACLE——SHOW 语句两种模式租户都接受，探测
        // 本身不产生方言报错）。查询失败或值非 ORACLE 一律只报「检测
        // 失败」：不透传服务端错误文本（trace id 等原始报错对用户没有
        // 增量信息，模式对不上时引导检查设置即可）
        let mut mode_ok = false;
        if conn
            .exec("SHOW VARIABLES LIKE 'ob_compatibility_mode'")
            .is_ok()
            && let Ok(res) = conn.take_use_result()
        {
            if let Some(row) = conn.fetch_row_cells(res) {
                // 列序：Variable_name, Value —— 取第二列判 ORACLE
                mode_ok = row
                    .get(1)
                    .and_then(|v| v.as_ref())
                    .is_some_and(|v| v.eq_ignore_ascii_case(b"ORACLE"));
            }
            conn.drain_result(res);
        }
        if !mode_ok {
            return Err(
                "OceanBase 连接成功但租户模式检测失败（非 Oracle 模式租户），请检查租户模式与路由设置".to_string(),
            );
        }

        let version = unsafe {
            let info = ffi::mysql_get_server_info(conn.0);
            if info.is_null() {
                String::new()
            } else {
                CStr::from_ptr(info).to_string_lossy().into_owned()
            }
        };

        // 初始 schema（可选）：database 字段在 OB Oracle 语义 = 连接后的
        // CURRENT_SCHEMA（只改名字解析不改权限）
        if let Some(db) = database.filter(|s| !s.trim().is_empty()) {
            conn.select_schema(db.trim())
                .map_err(|e| format!("OceanBase 设置初始 schema 失败：{e}"))?;
        }

        Ok((conn, version))
    }

    /// 优雅关闭（对活连接发送断连包并释放句柄）——Drop 兜底同效，显式
    /// 消费形态对齐 engines 层的 close 语义。
    pub fn close(self) {
        drop(self);
    }

    /// 最近一次错误的错误码（0 = 无错）。
    fn server_errno(&self) -> c_uint {
        unsafe { ffi::mysql_errno(self.0) }
    }

    /// 最近一次错误的文本 + 错误码（mysql_error 指向句柄内部缓冲，立即
    /// 拷贝出来避免被后续调用覆盖）。
    fn last_error(&self) -> String {
        let msg = unsafe {
            let p = ffi::mysql_error(self.0);
            if p.is_null() {
                String::new()
            } else {
                CStr::from_ptr(p).to_string_lossy().into_owned()
            }
        };
        format!("{msg}（错误码 {}）", self.server_errno())
    }

    /// 执行语句（阻塞）：field_count==0 → 无结果集（DML/DDL 走影响行数），
    /// 否则取 use 形态结果集句柄（结果集存活期间不得在同连接上发新语句）。
    pub fn exec(&mut self, sql: &str) -> Result<ObExecResult, String> {
        let c_sql =
            cstr_checked("SQL 文本", sql).map_err(|e| format!("OceanBase：{e}"))?;
        let rc = unsafe {
            ffi::mysql_real_query(
                self.0,
                c_sql.as_ptr(),
                c_sql.as_bytes().len() as c_ulong,
            )
        };
        if rc != 0 {
            return Err(format!("OceanBase：{}", self.last_error()));
        }
        if self.field_count() == 0 {
            let affected = unsafe { ffi::mysql_affected_rows(self.0) };
            Ok(ObExecResult::Affected(affected as u64))
        } else {
            Ok(ObExecResult::UseResult(self.take_use_result()?))
        }
    }

    /// 当前结果集列数（0 = 无结果集）。
    pub fn field_count(&mut self) -> c_uint {
        unsafe { ffi::mysql_field_count(self.0) }
    }

    /// 取 use 形态结果集句柄（real_query 后或 next_result 推进后；
    /// field_count>0 时调用），失败返回错误。
    pub fn take_use_result(&mut self) -> Result<*mut ffi::MysqlRes, String> {
        let res = unsafe { ffi::mysql_use_result(self.0) };
        if res.is_null() {
            return Err(format!("OceanBase：{}", self.last_error()));
        }
        Ok(res)
    }

    /// 结果集列名（fetch_fields + num_fields；指向结果集内部，须在
    /// free_result 前调用）。
    pub fn fetch_columns(&mut self, res: *mut ffi::MysqlRes) -> Vec<String> {
        let mut names = Vec::new();
        unsafe {
            let n = ffi::mysql_num_fields(res);
            let fields = ffi::mysql_fetch_fields(res);
            if fields.is_null() {
                return names;
            }
            for i in 0..n {
                let field = &*fields.add(i as usize);
                if field.name.is_null() {
                    names.push(String::new());
                } else {
                    names.push(CStr::from_ptr(field.name).to_string_lossy().into_owned());
                }
            }
        }
        names
    }

    /// 取下一行：None = 行流结束（fetch_row 返回 NULL——行级错误同形态，
    /// 工作台按结束容忍处理，同 Oracle 臂的行错误不中断语义）；行内 NULL
    /// 列 = None，非空列 = 原始字节（长度取 fetch_lengths，二进制安全）。
    pub fn fetch_row_cells(&mut self, res: *mut ffi::MysqlRes) -> Option<Vec<Option<Vec<u8>>>> {
        unsafe {
            let row = ffi::mysql_fetch_row(res);
            if row.is_null() {
                return None;
            }
            let n = ffi::mysql_num_fields(res);
            let lengths = ffi::mysql_fetch_lengths(res);
            let mut cells = Vec::with_capacity(n as usize);
            for i in 0..n as usize {
                let cell = *row.add(i);
                if cell.is_null() {
                    cells.push(None);
                } else {
                    let len: c_ulong = if lengths.is_null() {
                        0
                    } else {
                        *lengths.add(i)
                    };
                    let bytes = std::slice::from_raw_parts(cell.cast::<u8>(), len as usize);
                    cells.push(Some(bytes.to_vec()));
                }
            }
            Some(cells)
        }
    }

    /// 释放结果集（store/use 形态都必须调；遗漏会卡住后续读取）。
    pub fn free_result(&mut self, res: *mut ffi::MysqlRes) {
        if !res.is_null() {
            unsafe { ffi::mysql_free_result(res) };
        }
    }

    /// 整集排空并释放（多结果集的后续集：工作台单结果集模型只展示第一组，
    /// 后续集仍须消费到尾——否则连接状态卡在「命令不同步」）。
    pub fn drain_result(&mut self, res: *mut ffi::MysqlRes) {
        while self.fetch_row_cells(res).is_some() {}
        self.free_result(res);
    }

    /// 多结果集：还有下一个结果（存储过程多集 / MULTI_STATEMENTS 多语句）。
    pub fn more_results(&mut self) -> bool {
        unsafe { ffi::mysql_more_results(self.0) != 0 }
    }

    /// 推进到下一个结果集：Ok(true) = 有结果集待取（field_count>0，调用方
    /// 走 take_use_result）；Ok(false) = 没有更多（或推进到的是 OK 包——
    /// 单结果集模型下等价收尾）；Err = 服务端错误。
    pub fn next_result(&mut self) -> Result<bool, String> {
        let rc = unsafe { ffi::mysql_next_result(self.0) };
        if rc == -1 {
            return Ok(false);
        }
        if rc > 0 {
            return Err(format!("OceanBase：{}", self.last_error()));
        }
        Ok(self.field_count() > 0)
    }

    /// 会话内切换当前 schema：`ALTER SESSION SET CURRENT_SCHEMA = "X"`——
    /// 标识符双引号包裹、内部双引号翻倍（Oracle 引用标识符转义规则，
    /// 拼接注入安全）；只改名字解析不改权限。
    pub fn select_schema(&mut self, schema: &str) -> Result<(), String> {
        let quoted = format!("\"{}\"", schema.replace('"', "\"\""));
        let sql = format!("ALTER SESSION SET CURRENT_SCHEMA = {quoted}");
        self.exec(&sql)
            .map_err(|e| format!("OceanBase 切换 schema 失败：{e}"))?;
        Ok(())
    }

    /// 便捷查询：执行并收集全部行（小结果集场景——用户/schema 清单、目录
    /// 树计数；大结果集走 engines 的流式臂）。单元格保持原始字节
    /// （None = NULL 列）。
    pub fn query_rows(&mut self, sql: &str) -> Result<Vec<Vec<Option<Vec<u8>>>>, String> {
        match self.exec(sql)? {
            ObExecResult::Affected(_) => Ok(Vec::new()),
            ObExecResult::UseResult(res) => {
                let mut rows = Vec::new();
                while let Some(cells) = self.fetch_row_cells(res) {
                    rows.push(cells);
                }
                self.free_result(res);
                Ok(rows)
            }
        }
    }
}

impl Drop for ObConn {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // mysql_close 对活连接先发断连包再释放（优雅关闭），幂等安全
            unsafe { ffi::mysql_close(self.0) };
            self.0 = ptr::null_mut();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::c_int;

    /// libobclient 版本锁定（vendor/obconnector-c 单一来源
    /// rpm/libobclient-VER.txt，升级换树时同步改）
    #[test]
    fn ob_libobclient_version_is_2_2_14() {
        let mut major: c_int = 0;
        let mut minor: c_int = 0;
        let mut patch: c_int = 0;
        let mut buf = [0u8; 64];
        unsafe {
            ffi::ob_get_libobclient_version(
                &mut major,
                &mut minor,
                &mut patch,
                buf.as_mut_ptr().cast::<c_char>(),
                buf.len() as c_int,
            );
        }
        assert_eq!((major, minor, patch), (2, 2, 14));
        // 版本文本可读（NUL 结尾 CStr），形如 "2.2.14"
        let text = CStr::from_bytes_until_nul(&buf)
            .expect("version text should be NUL-terminated")
            .to_string_lossy()
            .into_owned();
        assert!(text.starts_with("2.2.14"), "unexpected version text: {text}");
    }

    /// 假连（127.0.0.1:1 拒连，超时 2s 兜底）：Err 且错误信息含「连接失败」
    ///（真实服务端行为需连库环境，这里锁错误映射形态）
    #[test]
    fn ob_connect_refused_returns_error() {
        let err = ObConn::connect("127.0.0.1", 1, "u", "p", None, 2)
            .expect_err("connect to 127.0.0.1:1 should fail");
        assert!(err.contains("连接失败"), "unexpected error: {err}");
    }

    /// CString 内嵌 NUL 校验：宿主 / 用户名 / 密码含 NUL 直接报错（不发起
    /// 网络连接——参数构造在 mysql_init 之前）
    #[test]
    fn ob_connect_rejects_embedded_nul() {
        for (host, user, pass) in [
            ("127.0.0.1\0x", "u", "p"),
            ("127.0.0.1", "u\0ser", "p"),
            ("127.0.0.1", "u", "p\0ass"),
        ] {
            let err = ObConn::connect(host, 1, user, pass, None, 2)
                .expect_err("embedded NUL should be rejected");
            assert!(err.contains("NUL"), "unexpected error: {err}");
        }
        // 初始 schema 的 NUL 校验路径（不会走到网络层）
        let err = ObConn::connect("127.0.0.1", 1, "u", "p", Some("sch\0ma"), 2)
            .expect_err("embedded NUL in schema should be rejected");
        assert!(err.contains("NUL"), "unexpected error: {err}");
    }

    /// client_flag 组合值锁定：建连声明位的按位或（sys 层常量随上游头文件
    /// 复核，移位即分叉——这里锁数值防静默漂移）
    #[test]
    fn ob_client_flag_combo_value() {
        let combo = ffi::client_flag::CLIENT_PROTOCOL_41
            | ffi::client_flag::CLIENT_MULTI_STATEMENTS
            | ffi::client_flag::CLIENT_MULTI_RESULTS
            | ffi::client_flag::CLIENT_PLUGIN_AUTH
            | ffi::client_flag::CLIENT_SUPPORT_ORACLE_MODE;
        // 512 | 1<<16 | 1<<17 | 1<<19 | 1<<27
        assert_eq!(combo, 512 | (1 << 16) | (1 << 17) | (1 << 19) | (1 << 27));
    }
}
