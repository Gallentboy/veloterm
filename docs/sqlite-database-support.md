# 数据库工作台接入 SQLite：设计与实现说明

> 日期：2026-10-22
> 范围：数据库工作台（侧栏「数据库」）新增第四个引擎 SQLite——连接记录、连接表单、对象树、SQL 执行、只读防护、取消。
> 状态：代码已落地，**验证门禁尚未执行**（见第 10 节），文中所有「待验证」项均为未跑过的结论。

## 1. 概述

数据库工作台新增 SQLite 引擎：复用既有 `db_connections` 表与 `DbEngine` 会话链路，在 `DbEngine` 枚举里加 `Sqlite` 变体，会话、对象树、流式执行、只读拦截、取消全部走与 MySQL / PostgreSQL / Oracle 相同的命令与事件契约。**零新增依赖**——`src-tauri/Cargo.toml` 里 `rusqlite = { version = "0.32", features = ["bundled-sqlcipher"] }` 早已存在（应用自身的 `veloterm.db` 就靠它），本次只是把同一份 `rusqlite` 用到工作台的用户库上。

SQLite 与另三引擎的本质差异有两条，其余都是这两条的推论：

1. **它是本地文件，不是网络服务**——没有 host / port / TLS / 用户名，连接参数只有「文件路径 + 加密方式 + 口令」。
2. **它是进程内库，没有连接级会话状态**——`USE db` 这类切库概念不存在（`ATTACH` 是库级挂载，不是当前库切换），对象树顶层直接用 `PRAGMA database_list` 表达。

## 2. 为什么是「表复用」而不是新建表

`db_connections` 的绝大多数列对 SQLite 同样成立：`name` / `env_level` / `readonly` / `color` / `created_at` / `updated_at` / `ssh_host_id` 与引擎无关，是连接记录的通用元信息。差异只有两列（`db_path`、`encryption_mode`），如果为此新建 `sqlite_connections` 表，代价是：

- `db::list_db_connections` / `get_db_connection` / `save_db_connection` / `delete_db_connection` 四组 CRUD 各要写第二份；
- 「数据库主机」列表页要合并两个数据源排序；
- 会话元信息 `DbSessionMeta` 的 `connection_id` 要能指向两张表，`db_connect` 与 `db_connection_test` 都得分派。

收益只是「不给 MySQL 记录留空列」。因此选择表复用，代价是 `host` / `port` / `ssl_mode` / `username` / `database` / `service_name` 对 SQLite 为空值或无意义，需要两层配合：

- **表单层**：`DbHostsScreen` 在 `engine === 'sqlite'` 时不渲染这些字段，提交时归一化——`host: ''`、`port: 0`、`database: null`、`serviceName: null`、`username: null`、`sslMode: 'disable'`；切到 SQLite 时主动 `setSslMode('disable')`，避免把 `require` 写进库。
- **校验层**：`db.rs` 的 `validate_db_connection` 在 `engine == "sqlite"` 时提前返回，只校验 `encryption_mode` 取值合法，跳过 `host` 非空与 `ssl_mode` 枚举校验（`sqlite_encryption_mode_valid`：`none | sqlcipher4 | sqlcipher3 | sqlcipher2 | sqlcipher-raw`）。

## 3. 数据模型

`db_connections` 在 SQLite 语义下的列含义：

| 列 | SQLite 语义 | 是否参与连接 |
| --- | --- | --- |
| `id` | 主键 | — |
| `name` | 连接名称 | 否（仅展示） |
| `engine` | 固定 `'sqlite'` | 是（引擎分发） |
| `host` | 无意义，存空串 | 否 |
| `port` | 无意义，存 0 | 否 |
| `database` | 无意义（SQLite 无连接级库），存 NULL | 否 |
| `service_name` | Oracle 专有，SQLite 恒 NULL | 否 |
| `db_path` | **数据库文件路径**（绝对路径或 `file:` URI）；NULL / 空 = 内存库 `:memory:` | **是** |
| `encryption_mode` | **加密方式**，`none \| sqlcipher4 \| sqlcipher3 \| sqlcipher2 \| sqlcipher-raw`，默认 `none` | **是** |
| `username` | 无意义，存 NULL | 否 |
| `password` | **加密口令**（`sqlcipher-raw` 模式填 64 位十六进制原始密钥） | 是（仅加密时） |
| `ssl_mode` | 无 TLS，恒 `'disable'` | 否 |
| `ssh_host_id` | 与其它引擎同：非空时 `db_connect` 直接拒绝（隧道未接入） | 否 |
| `env_level` / `readonly` / `color` / 时间戳 | 与其它引擎完全同构 | `readonly` 参与打开方式 |

两条新增列的取值域：

| 列 | 类型 | 取值 | 默认 |
| --- | --- | --- | --- |
| `db_path` | `TEXT`（可空） | 绝对路径 / `file:` URI / NULL / 空串（后两者 = `:memory:`） | NULL |
| `encryption_mode` | `TEXT NOT NULL` | `none` / `sqlcipher4` / `sqlcipher3` / `sqlcipher2` / `sqlcipher-raw` | `'none'` |

读取侧 `row_to_db_connection` 对 `encryption_mode` 做了双保险：`Option<String>` 取出后过滤空串，再回落 `default_encryption_mode()`（`none`），老库/异常值不会把非法字符串带进引擎层。

## 4. 加密方案（重点）

### 4.1 依赖形态

`rusqlite 0.32`（`libsqlite3-sys 0.30.1`）的 `bundled-sqlcipher` 特性把 SQLCipher 编译进二进制：**无运行期动态库依赖**，用户机器不需要预装 SQLCipher 或系统 libsqlite3。代价是构建期要编 C（已在本机与 CI 既有流程内，应用自身的 `veloterm.db` 一直走这条路）。

### 4.2 五种模式与 PRAGMA 序列

| `encryption_mode` | 适用场景 | PRAGMA 序列 |
| --- | --- | --- |
| `none` | 明文 SQLite（默认） | 无 |
| `sqlcipher4` | SQLCipher 4 默认参数（PBKDF2-HMAC-SHA512 / 256000 轮） | `PRAGMA key = '<口令>'` |
| `sqlcipher3` | 旧版 SQLCipher 3 生成的库 | `PRAGMA key = '<口令>'` → `PRAGMA cipher_compatibility = 3` |
| `sqlcipher2` | 旧版 SQLCipher 2 生成的库 | `PRAGMA key = '<口令>'` → `PRAGMA cipher_compatibility = 2` |
| `sqlcipher-raw` | 外部工具生成、已知原始密钥 | 传给 pragma 的值是文本 `x'<hex>'`（rusqlite 按单引号字面量渲染并双写内部单引号，SQLite 解析出的字符串仍是 `x'<hex>'`），SQLCipher 识别该形态即直用密钥、不经 KDF |

实现（`engines.rs::connect_sqlite`）：

```rust
if let Some(key) = key_material {
    conn.pragma_update(None, "key", key.as_str())
        .map_err(|e| format!("SQLite 设置加密口令失败：{e}"))?;
    let compat = match mode.as_str() {
        "sqlcipher2" => Some(2),
        "sqlcipher3" => Some(3),
        // sqlcipher4 即 SQLCipher 4 默认档位；raw 由密钥字面量决定，
        // 两者都不动 compatibility
        _ => None,
    };
    if let Some(v) = compat {
        conn.pragma_update(None, "cipher_compatibility", v)
            .map_err(|e| format!("SQLite 设置加密兼容档位失败：{e}"))?;
    }
}
```

### 4.3 为什么必须是「先 key、后 compatibility」

这是本实现最容易踩错的一点，顺序反了**不会报错**，只会静默按错误档位解密、随后以「口令错误」的形式失败。

- SQLCipher 的 `cipher_compatibility` 分支要求 codec 上下文已存在（源码 `sqlite3.c` 的 `sqlcipher_codec_pragma` 里是 `if (ctx)` 守卫）；而 codec 上下文由 `PRAGMA key` 经 `sqlcipherCodecAttach` 建立。
- 因此**先设 `cipher_compatibility` 会被静默忽略**——pragma 返回成功，但没有任何 codec 接收它。
- 反过来，`PRAGMA key` 之后的档位设置仍然有效，因为 KDF 是**惰性**的：attach 只保存口令，密钥要到首次读页才派生。key 之后、任何页访问之前设档位，档位参与的就是那次派生。

代码里「先 key 再 compatibility」的注释即以此为依据；后续维护不要为了「参数按字母序/按默认值优先」重排这两行。

### 4.4 口令存储与风险边界

`password` 列复用为加密口令，**明文存于应用自身的 SQLCipher 库**（`veloterm.db`，库本身由 `VELOTERM_DB_KEY` 或 `DEFAULT_MASTER_KEY` 加密）。这与 `zk_credentials`（ZK digest 用户名/密码同样明文存库）是同一安全水位，属于既有设计；本文档不改变它，但显式声明边界：

- 威胁模型内的保护是「磁盘上的 `veloterm.db` 被单独拷走」——没有主密钥打不开。
- 威胁模型外的是「应用运行期的本机进程」——能读应用内存或调用 Tauri 命令的本机进程可以拿到口令；这与所有保存密码的桌面客户端一致，不做额外加固。
- 用户侧缓解：对 SQLCipher 库可使用 `sqlcipher-raw` 模式，口令列只存 64 位十六进制密钥（仍是明文，但不再是可复用的口令短语）。

### 4.5 错误映射

打开后立刻执行 `SELECT count(*) FROM sqlite_master` 触发首块解密，把密钥问题挡在连接阶段而不是用户的第一条查询：

```rust
if let Err(e) = conn.query_row("SELECT count(*) FROM sqlite_master", [], |_| Ok(())) {
    if matches!(e.sqlite_error_code(), Some(rusqlite::ffi::ErrorCode::NotADatabase)) {
        return Err(format!("SQLite 打开失败：口令错误或不是加密数据库（{path}）"));
    }
    return Err(format!("SQLite 打开失败：{e}（{path}）"));
}
```

`SQLITE_NOTADB` 的语义是「文件头不是合法的 SQLite 数据库」，加密库用错口令时首块解密失败正是这个错误码。文案明确写成「口令错误**或**不是加密数据库」，避免用户把口令问题误判为文件损坏（反过来也成立：拿明文库配了 `sqlcipher4` 同样命中这一条）。

### 4.6 原始密钥模式

`sqlcipher-raw` 的输入必须是 64 位十六进制（256-bit 密钥），代码先剥掉可能存在的 `x'...'` 外壳，再校验长度与字符集，最后包成 `x'<hex>'` 交给 `PRAGMA key`：

```rust
let hex = password
    .strip_prefix("x'")
    .and_then(|s| s.strip_suffix('\''))
    .unwrap_or(password.as_str())
    .trim();
if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
    return Err("SQLite 原始密钥必须是 64 位十六进制字符串".to_string());
}
Some(format!("x'{hex}'"))
```

有意只接受 64 位（256-bit）一种形态，不接受 SQLCipher 也支持的 96 位（key + salt）写法——两种形态肉眼难辨，混用会得到「密钥错误」这种无从排查的报错。前端 `DbHostsScreen` 用 `/^[0-9a-fA-F]{64}$/` 先挡一层，后端仍独立校验（命令可被直接调用）。

> **待验证**：`rusqlite::pragma_update` 对字符串值渲染为单引号字面量（内部 `'` 双写），落到 SQLCipher 侧得到的文本正是 `x'<hex>'`；该形态被 SQLCipher 识别为原始密钥、跳过 PBKDF2 的行为**尚未在真机/自动化中确认**。

## 5. 连接与执行

### 5.1 打开标志

```rust
let flags = if readonly {
    rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
        | rusqlite::OpenFlags::SQLITE_OPEN_URI
        | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX
} else {
    rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE
        | rusqlite::OpenFlags::SQLITE_OPEN_URI
        | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX
};
```

- **不带 `SQLITE_OPEN_CREATE`**：路径写错或文件不存在时直接报错，绝不静默建一个空库让用户以为连上了。
- `SQLITE_OPEN_URI`：允许 `file:` URI（含 `file:...?mode=ro` 这类查询参数）。
- `SQLITE_OPEN_NO_MUTEX`：连接由 `with_sqlite` 串行使用（取出→执行→归还），不需要 SQLite 内部互斥。
- `readonly` 的第一层语义就是 `SQLITE_OPEN_READ_ONLY`（文件级只读）。第二层是既有语句级拦截——`db_query` 对所有引擎都检查 `meta.readonly` 并调 `sqlsplit::statement_is_readonly`，SQLite 同样生效。两层叠加是有意的：文件级兜住「任何写路径」，语句级给出「会话为只读模式」的明确文案。
- `busy_timeout` 设 5 秒（`conn.busy_timeout(Duration::from_secs(5))`，失败忽略——并发写锁等待只是优化，不影响连接成立）。

### 5.2 阻塞模型

`rusqlite::Connection` 是 `Send` 但**不是 `Sync`**，不能跨 `await` 直接持有。所有调用经 `with_sqlite` 包裹，与 Oracle 的 `with_oracle` 同型：

```rust
async fn with_sqlite<T, F>(&mut self, f: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(&rusqlite::Connection) -> Result<T, String> + Send + 'static,
{
    let taken = std::mem::replace(self, DbEngine::Closed);
    let DbEngine::Sqlite(engine) = taken else { /* 归还并报错 */ };
    let result = tokio::task::spawn_blocking(move || {
        let r = f(&engine.conn);
        (r, engine)
    })
    .await;
    // Ok：连接归还；Err（JoinError：panic/取消）：连接已无法取回，置 Closed
}
```

要点：

- **错误也归还连接**（闭包返回 `Result`，`engine` 随元组一起带回），一次查询失败不会把会话废掉。
- **panic 才置 `Closed`**：`spawn_blocking` 的 `JoinError` 说明闭包没有正常返回，连接状态不可信；会话层下一次操作会拿到「连接已关闭」。
- 连接建立本身（含 PBKDF2，SQLCipher 4 默认 256000 轮）也整段放进 `spawn_blocking`，不阻塞 tokio worker。

### 5.3 流式回传

`exec_streaming` 的 SQLite 臂在阻塞线程内完成全部行迭代与回调发射：

1. `conn.prepare(sql)`；`stmt.column_count() == 0` → 无结果集，`stmt.execute([])` 取 `changes()` 作为 `affected`，直接返回。
2. 有结果集：先回调列名（`QueryEvent::Columns`），再按 **300 行/批**（`ROWS_PER_BATCH`）回调 `QueryEvent::Rows`。
3. 读到 `max_rows` 后**再探一行**判定 `truncated`——`max_rows` 恰好等于总行数时不算截断。
4. 单元格字符串化按类型：`NULL` → `"NULL"`、整数直出、浮点走共用 `format_float`、文本 `from_utf8_lossy`、BLOB → `<BLOB n bytes>`；单格取值失败按 `NULL` 兜底，不中断整行。

行数上限由会话层统一给出：`max_rows.unwrap_or(5_000).clamp(1, 50_000)`；整体时间预算 `timeout_ms.unwrap_or(60_000).clamp(1_000, 3_600_000)`。

### 5.4 取消

SQLite 的取消是**协作式中断**，不是杀线程：

- `SqliteEngine` 持有 `cancelled: Arc<AtomicBool>`；`db_connect` 先建标志，再 `engine.bind_cancel_flag(cancelled.clone())`，同一个 `Arc` 也存进会话（`db_query_cancel` 只做 `store(true)`）。
- `bind_cancel` 经 `sqlite3_progress_handler(handle, 1000, trampoline, flag_ptr)` 注册回调：**每 1000 条 VDBE 指令**轮询一次标志，读到 `true` 返回非 0，SQLite 以 `SQLITE_INTERRUPT` 中止当前语句。走裸 ffi 是因为 rusqlite 的安全包装 `progress_handler` 需要 `hooks` feature，本工程未启用；`Arc` 的所有权留在 `self.cancelled`，保证回调持有的裸指针在连接存活期间有效。
- 执行臂用宏 `step!` 包住每个可能被中断的调用点（`prepare` / `execute` / `query` / `rows.next()`）：命中 `sqlite_is_interrupt`（错误码 `OperationInterrupted`，**不靠错误文案匹配**）→ 置 `summary.cancelled = true` 并正常返回，保留已读行数与列名；其它错误才上报。
- 会话层在每条语句开始前复位标志（`cancelled.store(false)`），上一条的正常结束不会吃掉下一条的取消。

取消的粒度是「语句级」：单条语句内部靠 progress handler 中断，多语句脚本靠会话循环在语句之间检查标志。

## 6. 对象树与元数据

对象树的分层懒加载（根 → 分组 → 对象 → 列）与另三引擎同构，SQLite 侧全部基于 PRAGMA 与 `sqlite_master`：

| 层级 | 实现 | 说明 |
| --- | --- | --- |
| 根层（schema） | `PRAGMA database_list` 的 `name` 列 | `main` + 所有 `ATTACH` 进来的库（`temp` 视是否已实例化而定）；`kind: 'schema'` |
| 分组层 | `SELECT type, COUNT(*) FROM <schema>.sqlite_master WHERE type IN ('table','view') AND name NOT LIKE 'sqlite_%' GROUP BY type` | 只有 `TABLE` / `VIEW` 两组；`sqlite_%` 是内部表（`sqlite_sequence` 等）不计数；计数为 0 的分组由 `group_nodes` 过滤掉 |
| 对象层 | `SELECT name FROM <schema>.sqlite_master WHERE type = ?1 AND name NOT LIKE 'sqlite_%' ORDER BY name` | `function` / `procedure` / `sequence` 三组直接返回空清单 |
| 列层 | `PRAGMA <schema>.table_info('<table>')` | 表与视图都适用，`cid` 顺序即列序 |

标识符与字面量严格分离：

- `quote_ident`：schema 名拼进 SQL 用。仅「首字符字母/下划线 + 其余 `[A-Za-z0-9_]`」可裸用，其余双引号包裹并把内部 `"` 双写；空名与含 `\0` 直接报错。禁止裸拼用户输入。
- `quote_string_literal`：PRAGMA 的表名/索引名参数是**字符串字面量**不是标识符，单引号包裹、内部 `'` 双写。

表详情（`table_detail`）：

- DDL 取 `<schema>.sqlite_master.sql`（建表原句，含列定义与约束）；`sql` 为 NULL（内部表或异常形态）时用 `table_info` 的列元数据拼一个 `CREATE TABLE` 兜底。
- 列：`PRAGMA table_info`，表头 `Column / Type / Nullable / Default / PK`，`notnull` 映射 `NO`/`YES`，`dflt_value` 为空显示 `NULL`。
- 索引：`PRAGMA index_list`（`seq` / `name` / `unique` / `origin` / `partial`）+ 逐索引 `PRAGMA index_info`（`seqno` / `cid` / `name`），按 `seqno` 排序后拼列名。**表达式索引的列名为 NULL**（SQLite 对表达式列传 0 → `OP_Null`），直接跳过，否则空项会混进列清单。表头 `Index / Unique / Columns`。

函数/过程/序列：SQLite 不支持这三类对象，`object_definition` 返回 `Err("SQLite 不支持 {kind} 对象")`，`catalog_objects` 对三组返回空 `Vec`（树上也不会出现这三个分组）。

## 7. SQL 分割与只读防护

### 7.1 方言与双路

`sqlsplit::dialect_for_engine("sqlite")` 返回 `SQLiteDialect`（`sqlparser 0.53`），PRAGMA / AUTOINCREMENT / 双引号标识符都能被主路解析。主路解析失败时仍走既有降级路：手写状态机按分号分割（字符串/注释/反引号内的分号不算），只读判定退回前缀白名单。

主路成功但语句数与朴素分割数不一致时，整段文本作为**一条**语句执行（`parsed: false`）——这是既有通用行为，不是 SQLite 专属。

### 7.2 PRAGMA 双态判定

PRAGMA 在 SQLite 里既可能是查询（`PRAGMA table_info(t)`），也可能是写（`PRAGMA journal_mode=WAL`、`PRAGMA key=...`）。只读判定分三层：

```rust
fn statement_kind_readonly(stmt: &Statement) -> bool {
    if let Statement::Pragma { name, value, .. } = stmt {
        let pragma = pragma_name(name);
        // ① 内省型：括号形态也是查询，直接放行
        if READONLY_PRAGMAS.contains(&pragma.as_str()) {
            return true;
        }
        // ② 加密族与维护动作：无论有无赋值一律拒绝
        if WRITE_PRAGMAS.contains(&pragma.as_str()) {
            return false;
        }
        // ③ 其余双态指令：无值 = 查询，带值 = 写
        return value.is_none();
    }
    // …其余走节点名前缀白名单（Query / Explain / Show / Analyze）
}
```

**① 内省白名单**（`READONLY_PRAGMAS`）收的是**不可设置**的纯查询指令：`table_info` / `table_xinfo` / `table_list` / `index_info` / `index_xinfo` / `index_list` / `foreign_key_list` / `database_list` / `collation_list` / `function_list` / `module_list` / `pragma_list` / `compile_options` / `integrity_check` / `quick_check` / `freelist_count` / `data_version` / `sqlite_version`。这批指令的括号形态（`PRAGMA table_info('t')`）在 `sqlparser` 里同样落进 `value`（`parse_pragma` 的 `is_eq: false` 分支），若无白名单会被「带值 = 写」误杀——这是只读会话里最常用的一类查询，必须放行。

可设置的指令（`cache_size` / `schema_version` / `page_size` / `user_version` / `application_id` / `mmap_size` / `max_page_count` / `encoding` / `foreign_keys` / `synchronous` / `journal_mode` …）**不在**白名单内，落到第 ③ 层：无值当查询、带值当写。因此 `PRAGMA cache_size` 可读、`PRAGMA cache_size = -2000` 被拒。

**② 无条件判写清单**（`WRITE_PRAGMAS`）分两类：

- 加密族：`key` / `rekey` / `cipher` / `cipher_migrate` / `cipher_compatibility` / `cipher_page_size` / `cipher_kdf_iter` / `cipher_hmac_algorithm` / `cipher_kdf_algorithm` / `cipher_plaintext_header_size` / `cipher_salt` / `cipher_default_kdf_iter`。只读会话里没有合法用途，且 `PRAGMA key` 的裸形态语义不明确，一律拒绝。
- 维护动作族：`optimize` / `incremental_vacuum` / `wal_checkpoint`。这些**即便不带赋值也会写盘**（`optimize` 写 `sqlite_stat1`、`incremental_vacuum` 释放页、`wal_checkpoint` 回写 WAL），不能靠「有无赋值」区分。

**降级路（无 AST）仍不放行 PRAGMA**：只有裸文本时无法区分双态，放进去等于给 `PRAGMA key=...` 开只读后门。好消息是 `PRAGMA table_info(t)`（不带引号）在 `SQLiteDialect` 下也解析失败（`parse_pragma_value` 不接受裸标识符），所以只读会话里查表结构请用带引号形态 `PRAGMA table_info('t')` 或 `PRAGMA table_list`——两者都走 AST 主路并被第 ① 层放行。

一个顺带的跨引擎影响：`statement_kind_readonly` 的 PRAGMA 分支不判引擎（`sqlparser` 的 `Keyword::PRAGMA` 也不按方言门控），所以 MySQL/PG/Oracle 会话里的 PRAGMA 现在同样按上述三层判定。这三家的服务器都不支持 PRAGMA 语法，语句会在执行阶段报错，实际影响仅是「错误从只读拦截变成引擎报错」。

### 7.3 降级路为什么**不**放 PRAGMA

降级路的前缀白名单是 `SELECT / WITH / EXPLAIN / SHOW / DESC / DESCRIBE / TABLE / VALUES`，**有意不含 PRAGMA**：降级路只有裸文本、没有 AST，无法区分「查询型」与「设置型」两种 PRAGMA，放进去等于给 `PRAGMA key=...`、`PRAGMA journal_mode=WAL` 开只读会话的后门。宁可解析失败时保守拒绝——绝大多数 PRAGMA 都能被 `SQLiteDialect` 解析（`Keyword::PRAGMA => self.parse_pragma()` 不按方言门控），走 AST 主路即可；解析不了的那些（裸词值形态）本就不该在只读会话里执行。

### 7.4 已知边界：触发器体

`CREATE TRIGGER ... BEGIN ... END;` 的体内部有分号（每条触发语句一个），而分割器对 SQLite **没有** Oracle 那样的块整段化处理（`contains_plsql_block` 仅 `engine == "oracle"` 生效）。当前行为：触发器定义在多语句编辑器里会被按分号切开，需要**整段单独执行**（单条语句的编辑器内容会整段送进 `prepare`，能正常建触发器；与其它语句混排时不行）。

为什么暂不处理：`sqlparser 0.53` 的 `parse_create_trigger` 只对 `PostgreSqlDialect | GenericDialect` 开放，`SQLiteDialect` 走进去会直接返回「expected an object type after CREATE」——也就是说 SQLite 的 `CREATE TRIGGER` **必然**落到降级路的分号分割，不存在「主路能解析」的可能。要可靠支持需要写一个 SQLite 专用的 `BEGIN ... END` 嵌套计数器（覆盖 `CASE WHEN ... END`、字符串与注释中的 `END`），属于独立工作量。

## 8. 前端

### 8.1 连接表单（`DbHostsScreen`）

- 引擎按钮组从三项扩到四项（`mysql / postgres / oracle / sqlite`），`ENGINE_DEFAULT_PORTS` 补 `sqlite: 0`，`ENGINE_LABELS` 补 `SQLite`；切到 SQLite 时把 `sslMode` 归一为 `disable`。
- SQLite 专属字段：数据库文件路径（`TextInput` + 文件夹按钮调 `sqlite_file_pick`，rfd 过滤器 `db / sqlite / sqlite3 / db3 / s3db` 与「所有文件」）、加密方式下拉（五项，`SQLITE_ENCRYPTION_MODES`）、口令（`PasswordInput`，**仅 `encryptionMode !== 'none'` 时条件渲染**，不是 disabled）。
- 对 SQLite 隐藏：host / port、database 选择器（`DatabasePickerField`）、SSL、用户名、密码（口令字段已在加密区块接管）。
- 校验：`canSave` / `canTest` 对 SQLite 走 `rawKeyOk && sqlitePasswordOk`——原始密钥必须 64 位十六进制；加密模式下新建必须填口令，编辑态留空 = 保留旧口令（后端 `fill_password_from_store` 会补）。测试连接允许 `dbPath` 留空（内存库）。
- **口令的「保留旧值」有两处例外**（都是 SQLite 特有，避免把口令带到不该带的地方）：
  1. 旧记录本来是**不加密**（`encryptionMode === 'none'`）的，用户改成加密后必须重新填口令——库里没有旧口令可保留，否则保存成功、连接时才报「需要提供口令」。判定用 `hasStoredKey = node != null && node.encryptionMode !== 'none'`。
  2. 用户把加密连接**改回不加密**时，前端在 `buildRecord()` 里显式传空串（而非 `null`）清掉残留口令；后端 `fill_password_from_store` 对 `engine == "sqlite" && encryption_mode == "none"` 直接跳过补全。否则残留口令会被拿去打开明文库，SQLCipher 报 `SQLITE_NOTADB`（「口令错误或不是加密数据库」），用户会误以为文件损坏。
- 列表行副标题：`SQLite · <dbPath 或 :memory:>`，不拼 host:port。

### 8.2 会话视图（`DbSessionView`）

- 表/视图点击生成 `SELECT * FROM "schema"."table" LIMIT 200;`——SQLite 的标识符引用用双引号（内部 `"` 双写），限行子句用 `LIMIT 200`（与 MySQL/PG 同，Oracle 才走 `ROWNUM`）。
- 右键「创建索引」骨架：`CREATE INDEX "idx_<表名>" ON "<schema>"."<表名>" (col);`，索引名也加双引号，避免与关键字冲突或大小写混用时的解析歧义。
- 「当前数据库」下拉对 SQLite **禁用**（`disabled={databases.length === 0 || meta.engine === 'sqlite'}`）但保留展示：SQLite 无连接级切库，`db_use_database` 的 SQLite 臂只更新元信息不报错（保持命令语义一致），`db_connect` 的会话元信息把 `database` 固定为 `main`。
- 欢迎语句不做分支：`SELECT 1;` 在 SQLite 合法（只有 Oracle 需要 `SELECT 1 FROM DUAL`）。

### 8.3 i18n

`dbHosts.*` 新增 18 个 key，`zh-CN` 与 `en-US` 同构：

`engine_sqlite`、`dbPathLabel`、`dbPathPlaceholder`、`dbPathBrowse`、`dbPathPickTitle`、`dbPathMemoryHint`、`encryptionLabel`、`encryption_none`、`encryption_sqlcipher4`、`encryption_sqlcipher3`、`encryption_sqlcipher2`、`encryption_sqlcipher_raw`、`sqlitePasswordLabel`、`sqlitePasswordHint`、`sqlitePasswordKeepHint`、`sqlitePasswordRequired`、`sqliteFileHint`、`sqliteRawKeyInvalid`。

## 9. 已知边界与后续工作

| # | 边界 | 当前行为 | 为什么暂不处理 |
| --- | --- | --- | --- |
| 1 | 触发器体分割 | `CREATE TRIGGER ... BEGIN ... END;` 与其它语句混排时按分号切开；需整段单独执行 | SQLite 触发器体无固定结束标记，需专用 `BEGIN`/`CASE...END` 嵌套词法器；见 7.4 |
| 2 | 未接入 SSH 隧道 | `ssh_host_id` 非空时 `db_connect` 直接拒绝（与另三引擎同一处判断） | SQLite 是本地文件，隧道无意义；若未来支持「远端文件」，语义是先把文件同步到本地或经 SFTP 读写，需另设计而非复用 TCP 隧道 |
| 3 | 无 WAL 模式切换 UI | 用户可在 SQL 编辑器执行 `PRAGMA journal_mode=WAL`；只读会话会拒绝（语句层 + 文件层） | 属连接级调优，不是工作台核心能力；做成 UI 要决定「每个连接是否持久化 PRAGMA」 |
| 4 | `PRAGMA cipher_migrate` 未做 UI 引导 | 只有 `cipher_compatibility` 档位选择；从 SQLCipher 3 升级到 4 需用户自行在外部工具执行 `cipher_migrate` | `cipher_migrate` 会重写整库、不可回退，做成按钮风险高；需要先有备份/确认流程 |
| 5 | 无 ATTACH / DETACH 的 UI | 可在 SQL 编辑器执行 `ATTACH DATABASE ... AS ...`，但对象树需手动刷新（重新展开根层） | 对象树根层每次展开都会重新 `PRAGMA database_list`，刷新即生效；加 UI 需要路径选择器 + 别名管理，收益低于成本 |
| 6 | 口令明文存于应用 SQLCipher 库 | 与 `zk_credentials` 同级安全水位（见 4.4） | 既有设计，改动会牵动整个凭证存储模型；本文档只声明风险边界 |
| 7 | 未做真机 / 自动化验证 | 代码已落地，门禁未执行 | 本次只交付代码与文档；见第 10 节 |

补充一条实现层边界（不属于产品能力）：`bind_cancel` 的 progress handler 经裸 ffi 注册，回调里的 `Arc<AtomicBool>` 裸指针有效性依赖 `self.cancelled` 的生命周期——后续若重构 `SqliteEngine` 的字段顺序或把 `cancelled` 移出结构体，必须同时保证指针在连接关闭前不被释放。

## 10. 验证（待执行）

本次**未运行**任何测试或构建命令。以下门禁全部标注为待验证，需在合并前逐条执行并记录真实结果：

| 检查 | 命令 | 要求 | 状态 |
| --- | --- | --- | --- |
| 前端类型 | `pnpm run typecheck` | 零错误 | 待验证 |
| 前端构建 | `pnpm run build` | 成功 | 待验证 |
| Rust 编译 | `cd src-tauri && cargo check --workspace --all-targets` | 零错误 | 待验证 |
| Rust 测试 | `cd src-tauri && cargo test --workspace` | 全绿 | 待验证 |
| Lint | `cd src-tauri && cargo clippy --workspace --all-targets` | 自有代码零警告 | 待验证 |

功能层需要人工/补充验证的点（当前无自动化覆盖）：

1. **明文库**：新建 SQLite 连接指向一个已有 `.db`，能列出表/视图、点表出 `SELECT * FROM "main"."t" LIMIT 200;`、表详情三栏有内容。
2. **加密库**：`sqlcipher4` 建库 → 断开重连 → 口令正确可读、口令错误命中「口令错误或不是加密数据库」文案。
3. **兼容档位**：用 `sqlcipher3` / `sqlcipher2` 打开对应旧库；以及**故意把顺序写反的对照实验**（先 compatibility 后 key）应复现「静默忽略 → 口令错误」，用于确认 4.3 的结论。
4. **原始密钥**：`sqlcipher-raw` + 64 位十六进制能打开由外部工具以 raw key 生成的库；长度/非十六进制输入被前后端各挡一次。
5. **只读**：`readonly` 连接下 `SELECT` 通过、`INSERT` 被语句层拒绝、`PRAGMA journal_mode=WAL` 被拒绝；文件权限层面确认不可写。
6. **取消**：对超大表跑 `SELECT * FROM big`，执行中点取消，应在 progress handler 的粒度内返回 `cancelled: true` 且保留已读行数。
7. **不建库**：指向不存在的路径应报「SQLite 打开失败」，且该路径下**没有**被创建出空文件。
8. **ATTACH**：执行 `ATTACH DATABASE '<path>' AS aux;` 后重新展开根层，应出现 `aux` 且其分组/对象可浏览。
9. **内存库**：`dbPath` 留空可连接，关闭会话即丢（无持久化预期）。

## 11. 文件清单

| 文件 | 职责 |
| --- | --- |
| `src-tauri/src/db.rs` | `db_connections` 建表新增 `db_path` / `encryption_mode`；`init_db_at_path` 幂等 ALTER 补列；`DbConnectionRecord` 两字段 + `default_encryption_mode`；`supported_engine` / `validate_db_connection` 的 SQLite 分支；`DB_CONNECTION_COLUMNS` 与 CRUD 语句补列；`row_to_db_connection` 空串回落 |
| `src-tauri/src/db_browser/engines.rs` | `DbEngine::Sqlite(SqliteEngine)` 变体；`connect_sqlite`（打开标志、加密 PRAGMA 顺序、首块解密校验、版本串）；`with_sqlite` 阻塞包裹；progress handler 取消（`bind_cancel` / trampoline）；`exec_streaming` 的流式回传与中断映射；`catalog_root` / `catalog_groups` / `catalog_objects` / `catalog_columns` / `table_detail` / `object_definition` / `list_databases` / `select_database` 的 SQLite 臂；`quote_ident` / `quote_string_literal` / `sqlite_cell` / `sqlite_err` / `sqlite_is_interrupt` / `sqlite_database_names` 辅助函数 |
| `src-tauri/src/db_browser/sqlsplit.rs` | `dialect_for_engine` 加 `SQLiteDialect`；`statement_kind_readonly` 的 PRAGMA 双态判定；降级白名单不含 PRAGMA 的说明 |
| `src-tauri/src/db_browser/mod.rs` | `db_connect` 建取消标志并绑定引擎、会话元信息 `database` 固定 `main`；`db_use_database` 的 SQLite 空操作臂；`sqlite_file_pick` 命令（rfd 文件选择） |
| `src-tauri/src/lib.rs` | 注册 `db_browser::sqlite_file_pick` 到 invoke handler |
| `src/services/dbConnectionsService.ts` | `DbEngineKind` 加 `'sqlite'`；`SqliteEncryptionMode` 类型；`DbConnectionRecord` 加 `dbPath` / `encryptionMode`；`DbSessionMeta.database` 注释；`sqliteFilePick()` invoke 封装 |
| `src/components/dbhosts/DbHostsScreen.tsx` | 引擎按钮加 SQLite；`ENGINE_DEFAULT_PORTS` / `ENGINE_LABELS` 补项；`SQLITE_ENCRYPTION_MODES`；SQLite 专属表单区块（路径 + 浏览 + 加密方式 + 口令）；隐藏 host/port/database/SSL/用户名/密码；`canSave` / `canTest` 分支；`buildRecord` 归一化；列表行副标题与图标色 |
| `src/components/db/DbSessionView.tsx` | 表点击生成语句的 SQLite 双引号引用；索引模板的 SQLite 分支；「当前数据库」下拉对 SQLite 禁用；欢迎语句注释 |
| `src/i18n/locales/zh-CN.ts` | `dbHosts.*` 新增 18 个中文 key |
| `src/i18n/locales/en-US.ts` | 同构的 18 个英文 key |
| `docs/sql/2026-10-22-db-connections-sqlite.sql` | 本次 DDL 变更的可审计留档（幂等 ALTER + 回滚 + 校验查询） |
| `docs/sqlite-database-support.md` | 本文档 |
