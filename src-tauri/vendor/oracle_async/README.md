# oracle_async

纯 Rust 原生异步 Oracle 客户端驱动（基于 Tokio），零 C/C++ 动态库依赖（无需 Oracle Instant Client / OCI / ODPI-C）。

> **本地修补记录**（升级 / 重放提示：以下是对上游参照实现 sijms/go-ora **v2**（本地检出 `~/github/go-ora`）的偏差修正，换树或升级时需逐条复核）：
> 1. **TTC 消息 19（服务端流控）误并入 case 11 解析**（`src/conn/routines/exec.rs` / `fetch.rs`）：v2 的 `case 19` 是零载荷流控信号——客户端须**回写单字节 19 并继续读响应流**。移植版原先把 `11 | 19` 合并按「绑定方向」的 ResultSet.load 头 + 逐列方向字节解析，19 到来时读走不存在的字段导致 TTC 流错位——真机上长 SQL / 含中文语句的响应流以 `Type decode error: invalid int size: 66`（把后续文本字节当压缩 int 的 size）暴露。fetch 轮原先落 `other` 静默跳过（不回 ACK）同样错位，一并修复。
> 2. **case 11 列数组合公式**（`exec.rs`）：v2 `ResultSet.load` 的列数 = `i2 低段 + i4 高段 × 0x100`，移植版原先只取 i2（>255 列的宽结果集会截断方向字节循环）。
> 3. **命令级输入残留清理 / 防毒**（`conn/mod.rs` + `exec.rs` / `fetch.rs` / `ping.rs` / `close_cursor` / `simple_object`）：v2 每条命令发起前 `ResetBuffer()`（含 `resetRead` 清空输入缓冲），case 19 的流控回写同样先清再写。移植版原先从不清理输入残留——一次解码错误/提前退出后，残留字节永久毒化会话（真机症状：某条语句失败后，之前能跑的 SQL 全部持续报 `invalid int size`）。现新增 `send_request`（= 清残留 + 发送）用于所有请求发起点，`reset_read_buffer` 供 19 回写复用。
> 4. **case 6（行块头）的列存在位图**（`row_common.rs::read_row_header` + `exec.rs` / `fetch.rs` 调用点）：v2 case 6 = `ResultSet.load`，其**第一个 DLC 是列存在位图**并经 `setBitVector` 置每列 `getDataFromServer`（位序同 Msg 21：列 n → 字节 n/8 位 n%8）；移植版原先丢弃该 DLC——服务端省略 NULL 单元格时按全存在去读，读到不存在的单元格流即错位（真机 PLAN_TABLE 大量可空列以 `invalid int size: 67` 暴露）。现在位图非空时更新 `bit_vector`，空/NULL 位图沿用现值。


对标 `mysql_async` 的成熟架构设计，全面基于 **Tokio + `tokio_util::codec::Framed` + 异步 `Routine` 状态机** 实现 Oracle 专有的 **TNS（传输网络层）** 与 **TTC（双任务通用数据层）** 协议。

---

## 一、核心特性

1. **纯 Rust 实现（Pure Rust）**：
   - 零链接期与运行期 C 依赖（摆脱 `DPI-1047` 错误）。
   - 跨平台原生静态编译（macOS aarch64/x86_64, Windows, Linux musl）。
2. **现代 Oracle 身份认证**：
   - 支持 Oracle 12c+ (12.1/12.2/18c/19c/21c/23ai) 的 **PBKDF2-HMAC-SHA512** SpeedyKey 密钥拉伸 + **AES-256-CBC** 动态会话加密。
   - 向后兼容 11g（SHA-1 + AES-192）及 10g（O5Logon DES-CBC / MD5）。
3. **分层帧协议（TNS / TTC）**：
   - `TnsCodec` 帧管理，自动处理握手前后的 16-bit / 32-bit 大包 SDU 头切换（支持 64KB+ SDU）。
   - 自动处理多分片 DATA 包切片与拼接。
4. **强类型数据系统**：
   - 内部支持 Oracle 基数 100 浮点格式（`NUMBER`）向 `rust_decimal::Decimal`、`i64`、`f64` 转换。
   - 支持 `VARCHAR2`、`CHAR`、`NVARCHAR2`、`DATE`（7 字节）、`TIMESTAMP`（11 字节）、`TIMESTAMP WITH TIME ZONE`（13 字节）、`RAW`。
5. **分级清晰的日志系统**：
   - `trace`: 底层 TNS 原始报文十六进制分片、逐字节移动与 CLR Chunk 拆解。
   - `debug`: TNS 数据包头、TTC 协议交互字（Msg 1, 2, 3, 16, 7, 4）、协商能力位图、SCN 与 Session ID。
   - `info`: 连接建立、TNS Accept、用户认证成功（打印实例名/用户名/会话号）、连接池创建与归还。
   - `warn`: 收到 ORA 警告（Msg 15）、TNS 标记中断包、重定向回退、时区/字符集兜底。
   - `error`: 监听器拒连（ORA-12514 等）、密码校验失败（ORA-01017）、网络读写异常与协议格式违背。
6. **对标 `mysql_async` 异步设施**：
   - 包含异步连接池 `Pool` 与 `PooledConn`。
   - 支持 `Transaction` 事务生命周期（commit / rollback）。
   - 支持 `break_connection`（发送 TNS Marker 包异步取消长查询）。

---

## 二、架构设计与模块划分

```
oracle_async/
├── Cargo.toml
├── src/
│   ├── lib.rs                  // 顶层导出
│   ├── error.rs                // Error 与 OracleError (ORA-xxxxx)
│   ├── opts.rs                 // 连接参数构建器与 EZConnect URL 解析
│   ├── crypto/                 // 纯 Rust 加密算法库
│   │   ├── speedy_key.rs       // PBKDF2-HMAC-SHA512 SpeedyKey
│   │   ├── aes_cbc.rs          // AES-128/192/256-CBC (zero-IV / PKCS7)
│   │   └── o5logon.rs          // 10g/11g DES/MD5 兼容算法
│   ├── proto/                  // Oracle 线路数据结构
│   │   ├── tns_types.rs        // TNSType 枚举与标识
│   │   ├── caps.rs             // CompileCaps 与 RuntimeCaps 位图
│   │   ├── oracle_number.rs    // 基数 100 浮点高精度互转
│   │   └── oracle_date.rs      // 7/11/13 字节日期时区互转
│   ├── io/                     // 底层 I/O 与 Codec
│   │   ├── packet.rs           // TnsPacket 枚举与 Connect/Accept/Refuse 解析
│   │   ├── codec.rs            // TnsCodec: tokio_util::codec::Decoder / Encoder
│   │   ├── ttc_reader.rs       // TTC 二进制读取 (变长压缩整数/CLR/DLC/KeyVal)
│   │   └── ttc_writer.rs       // TTC 二进制构建与序列化
│   ├── conn/                   // 连接与协议 Routine 状态机
│   │   ├── mod.rs              // Conn 结构体与会话生命周期
│   │   ├── transaction.rs      // Transaction (commit/rollback)
│   │   ├── pool/               // 异步连接池 (Pool/PooledConn)
│   │   └── routines/           // 独立交互 Routine 状态机
│   │       ├── handshake.rs    // TNS Connect -> Accept
│   │       ├── ttc_nego.rs     // TTC MsgCode 1 协议协商
│   │       ├── data_type_nego.rs // TTC MsgCode 2 数据类型协商
│   │       ├── auth.rs         // TTCOAUTH 动态密钥认证
│   │       ├── exec.rs         // TTCOEXEC 执行与结果集描述
│   │       ├── fetch.rs        // 分批游标行抓取
│   │       └── ping.rs         // 心跳探活
│   └── queryable/              // 高级接口
│       ├── values.rs           // OracleValue 与 FromValue
│       ├── row.rs              // ColumnMeta 与 Row
│       ├── query_result.rs     // 惰性迭代结果集 QueryResult
│       └── mod.rs              // Queryable trait
└── tests/
    ├── crypto_tests.rs         // 密码学算法回归测试
    ├── proto_tests.rs          // 核心类型与 Number/Date 编解码测试
    └── codec_and_packet_tests.rs // TNS Codec 帧切分与封包测试
```

---

## 三、快速上手示例

### 1. 建立连接与执行查询

```rust
use oracle_async::{Conn, Opts, Queryable, Row};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 支持标准 URL 格式或 OptsBuilder
    let opts = Opts::from_url("oracle://scott:tiger@localhost:1521/XEPDB1")?;
    let mut conn = Conn::connect(opts).await?;

    println!("Connected to Oracle: {}", conn.server_version());

    // 流式查询
    let mut result = conn.query("SELECT banner FROM v$version").await?;
    while let Some(row) = result.next_row().await? {
        let banner: String = row.get(0)?;
        println!("Banner: {banner}");
    }

    Ok(())
}
```

### 2. 使用连接池（Pool）

```rust
use oracle_async::{Pool, Opts, Queryable};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let opts = Opts::from_url("oracle://scott:tiger@localhost:1521/XEPDB1")?;
    let pool = Pool::with_max_conns(opts, 10);

    // 从池中租借连接
    let mut conn = pool.get_conn().await?;
    let affected = conn.exec("UPDATE employees SET salary = salary + 100 WHERE department_id = 10").await?;
    println!("Updated rows: {affected}");

    // 离开作用域自动归还连接池
    Ok(())
}
```
