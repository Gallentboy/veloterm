# oceanbase_async（mysql_async 0.35.1 的 veloterm 分叉）

libobclient（`src-tauri/vendor/obconnector-c`，LGPL-2.1 C 库）的**纯 Rust 对位
实现**——OB Oracle 租户模式不再依赖 C FFI。分叉自 mysql_async 0.35.1
（MIT/Apache-2.0）：OB 的线缆协议就是「标准 MySQL 握手/认证 + 附加层」，故
复刻上游作底座，只补附加层。

**同一个 crate 双角色**：app 的 `mysql_async` 依赖经 Cargo 重命名指向本树
（`package = "oceanbase_async"`），因此 MySQL 与 OB Oracle 共用一套连接/查询
类型（9 处 match 臂零分叉）。`ob_protocol20` 选项默认关——MySQL 行为与上游
完全一致；只有 OB Oracle 连接（`extra_options.driver = "rust"`）才打开。

## 相对上游的补丁清单（升级 mysql_async 时逐条重放）

| 文件 | 改动 |
|---|---|
| `src/ob20.rs`（新增） | OB20 协议层：24B 头（magic 0x20AB/version 20/conn_id/request_id(u24)/pkt_seq/payload_len/flags/CRC16-ARC）、CRC32 尾包、extra 区跳过、能力位 shift 0..=20、connect attrs 键值、`scan_capability_flag`（OK 包 session-track 解析）；`sm3` 子模块（GB/T 32905 SM3 + `ob_sm3_password` 应答算法） |
| `src/io/ob20_codec.rs`（新增） | `Ob20Codec`：写 = MySQL 分块 → ob20 包 → zlib → 外层压缩帧；读 = 解外层帧 → 解压 → unwrap → 多块重组；内/外序号与 request_id 管理 |
| `src/io/mod.rs` | `PacketCodec` → `ConnCodec` 枚举（Plain/Ob20）贯穿 Framed；`Stream::enable_ob20` 热换 codec（`make_secure` 同型）；`ConnCodec` 的 Decoder/Encoder/seq/max_packet/compress 转发 |
| `src/opts/mod.rs` | 新增 `ob_protocol20` 字段 + getter + builder 方法 + `Default` 初值 |
| `src/conn/mod.rs` | `ConnInner.ob_capability`；`handle_ok` 解析能力回显；`do_handshake_response` 上报 connect attrs（`opts.ob_protocol20()` 时）；`ob_sm3_auth_data`（SM3 应答，借道 `AuthPluginData::Sha2` 的 32 字节载体）；`perform_auth_switch`/`continue_auth` 支持 `ob_sm3_password` 插件；`switch_to_ob20`（认证后按能力位切换，`lib.rs` 连接流程里紧跟 `switch_to_compression`） |
| `Cargo.toml` | 改名 `oceanbase_async`、lib 名同步；加 `crc32fast`；删 native-tls 分支与 `Cargo.toml.orig`/`Cargo.lock`（features 只留 rustls 形态） |
| `src/lib.rs` | `pub mod ob20;` |

## 协议事实（对照 obconnector-c 源码）

- **握手完全标准 MySQL**：OB 能力位经 connect attrs（`__proxy_capability_flag`，
  含 `OB_PROTOCOL_V2 = 1<<8`）上报；服务端在 OK 包的
  `SESSION_TRACK_SYSTEM_VARIABLES` 里回显 `ob_capability_flag`（十进制 u64），
  客户端据此切换。libobclient 的 `ENABLE_PROTOCOL_OB20` 缺省 AUTO=开。
- **ob20 包** = `[24B 头][extra_len:4?][extra?][MySQL 包(含 4B 头)][crc32:4]`；
  头 payload_len = extra 区 + MySQL 包体（**不含**头与尾包）；extra 长度域
  **不含自身 4 字节**；CRC32 覆盖 extra 区（含长度域）+ 包体；头 CRC16
  （CRC-16/ARC）覆盖「外层 7 字节前缀 + 头前 22 字节」，**complen 口径恒 0**
  （libobclient 读写两侧一致：写侧填 0，读侧 `decode_protocol20_header(..., 0)`）。
- **外层帧** = MySQL 压缩协议 `[compressed_len:3][seq:1][uncompressed_len:3][body]`；
  `uncompressed_len == 0` 表示 body 未压缩（小包直接透传）。ob20 头的 pkt_seq
  与帧 seq 同源（libobclient 的 `compress_pkt_nr`）。
- **SM3 认证**：`d1 = SM3(pw)`；`d2 = SM3(d1)`；`r = SM3(d2 ‖ salt)`；
  应答 `d1 ⊕ r`（32 字节）；插件名 `ob_sm3_password`。

## 测试

```sh
cd src-tauri/vendor/mysql-async-ob && cargo test --lib ob20
cd src-tauri/vendor/mysql-async-ob && cargo test --lib io::ob20_codec
```

12 项纯单测覆盖：CRC-16/ARC 标准校验值、头/尾包编解码往返与篡改检测、extra
区跳过、request_id 回绕、能力位锚定、connect attrs 键值、SM3 国标向量
（`SM3("abc")`）、SM3 应答形态、session-track 能力回显扫描、外层帧全链路
往返与命令边界。

上游自带的 `conn::*` / `pool::*` 测试需要真实 MySQL 服务器（ConnectionRefused），
与分叉无关。

## 已知边界

- 未实现：全链路追踪（FLT）/会话同步（SESS_INFO）的 extra 区**发送**——只跳过
  接收侧 extra 区；LOB locator v2、Oracle 扩展类型二进制协议（文本协议下多数
  类型以字符串返回，够工作台用）。
- `ob20` 头 pkt_seq 用外层帧序号；若真实服务端要求两者独立递增，需按抓包调整
  （当前按 libobclient 同计数器实现）。
- 真机验证（OB Oracle 租户 + Rust 驱动）尚未执行，门禁不依赖。

## 许可证

上游 mysql_async：MIT / Apache-2.0（保留原 LICENSE-*）。ob20 实现按线缆行为
独立编写（常量与格式 facts），未逐行翻译 LGPL 的 obconnector-c；若后续需要
对照翻译其代码，先过项目方许可评估（AGENTS.md 的 GPL 商用确认先例）。
