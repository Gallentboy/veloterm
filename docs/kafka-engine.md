# Kafka 引擎接入设计（数据库工作台第六引擎）

> 状态：设计稿（待评审）。调研结论见前次对话：rdkafka 0.39.0（内置 librdkafka 2.12.1），
> features `["cmake-build", "libz-static", "zstd", "ssl", "tokio"]`，许可 MIT + BSD-2
> 无 GPL 负担。本文档只定契约：表结构、参数模型、命令面、对象树映射、UI 布局、
> i18n 键、CI 改动点与分期；实现按分期另起提交。

## 0. 摘要与定位

Kafka 作为数据库工作台的第六引擎（`engine = 'kafka'`）接入：连接记录复用
`db_connections` 表 + 新增一个**引擎无关的 JSON 扩展列** `extra_options`；后端复用
既有会话/对象树/流式查询三套管道（`db_connect` / `db_catalog` / `db_table_detail` /
`db_query` 零新增命令）；前端在连接表单与 DbSessionView 各加 kafka 分支，主内容区
沿用「左树 + 查询框 + 流式结果」骨架，树节点点击生成 `CONSUME` 读命令（redis 点 key
生成命令的同型交互）。

- 一期（MVP）：连接管理、集群浏览（Brokers / Topics / Consumer Groups 三组树）、
  topic 与 group 详情、消息查看（CONSUME 按分区/offset/时间点拉取）、只读保障。
- 二期：消息发送（`SEND`，readonly=false 放行）、topic 管理（建删/配置变更，
  AdminClient）、SASL OAUTHBEARER。
- 不做：schema registry / Kafka Connect 管理、GSSAPI（librdkafka Windows 不支持
  cyrus SASL，跨平台行为不一致，直接不做）。

## 1. 存储设计

### 1.1 新列：`extra_options TEXT`（JSON，引擎无关扩展位）

`db_connections` 幂等 ALTER 加一列（建表批次后的列级迁移，duplicate column 吞掉幂等，
照抄 `compatible_mode` 段）：

```sql
ALTER TABLE db_connections ADD COLUMN extra_options TEXT
```

命名取舍（推荐前者）：

- **`extra_options`（通用扩展位，推荐）**：列已 14 个，service_name / mode / db_path /
  compatible_mode 的「一引擎一列」趋势不可持续；通用 JSON 位一次加列终身受用，
  后续第八引擎的非关系型参数（如 ClickHouse / Mongo）不再动表。风险由强类型解析
  控制（§1.3）。
- `kafka_options`（引擎专属列）：延续现有惯例、语义自文档，但每引擎加列的路线
  长期不经济。

核对文档落 `docs/sql/2026-10-23-db-connections-extra-options.sql`（redis-db-changes.sql
先例：结论 + 人工核对 SQL）。

### 1.2 既有列的 Kafka 语义映射

| 列 | Kafka 语义 |
| --- | --- |
| `host` | **bootstrap servers 列表**：逗号分隔的 `host[:port]` 条目（例 `10.0.0.1:9092, 10.0.0.2:9092`）；条目不带端口时补 `port` 列值。`bootstrap.servers` 拼接时整体 trim、条目内空格剔除 |
| `port` | 默认端口（表单默认 **9092**）；仅对不带端口的条目生效。列本身 NOT NULL，语义 = 「缺省端口」而非「唯一端口」 |
| `username` / `password` | SASL 用户名/口令（PLAIN / SCRAM 用；OAUTHBEARER 二期另议） |
| `ssl_mode` | 与 redis 语义完全同构的四档（§1.4） |
| `database` | 恒 NULL（Kafka 无库概念；`service_name` 同 redis 恒 NULL、`mode` / `db_path` / `encryption_mode` / `compatible_mode` 不参与连接） |
| `readonly` | 沿用既有语义：一期查询面天然只读（无 SEND 命令）；二期 SEND 在 sqlsplit 层受 readonly 白名单拦截 |
| `extra_options` | `{"saslMechanism": "PLAIN", "clientId": "VeloTerm"}`（camelCase，§1.3） |

### 1.3 `extra_options` 的 Kafka schema

```jsonc
{
  "saslMechanism": "PLAIN",     // PLAIN | SCRAM-SHA-256 | SCRAM-SHA-512（默认 PLAIN；
                                 // 有 username 时才消费；OAUTHBEARER 二期）
  "clientId": "VeloTerm"        // 可选；缺省 "VeloTerm"
}
```

- 结构体形态：`DbConnectionRecord` 加 `#[serde(default)] pub extra_options:
  Option<serde_json::Value>`（原样存取，list/save/roundtrip 四处同步——HostRecord
  系惯例）；前端不提供裸 JSON 编辑器，表单字段 ↔ JSON 的组装在 dbConnectionsService
  完成（见 §4.2），用户感知不到 JSON。
- 后端解析：`connect_kafka` 处经 serde 反序列化成强类型 `KafkaOptions`（未知键忽略，
  前向兼容）；校验落在 `validate_db_connection` 的 kafka 分支（mechanism 合法值、
  host 可解析出 ≥1 条目、端口 1–65535）。

### 1.4 安全协议推导（表单免选 security.protocol）

librdkafka 的 `security.protocol` 由「ssl_mode × 是否有 username」推导，表单不下发
该配置，降低用户心智：

| ssl_mode | username | security.protocol | 附加配置 |
| --- | --- | --- | --- |
| disable | 无 | PLAINTEXT | — |
| disable | 有 | SASL_PLAINTEXT | 表单 hint 警示「凭证明文传输」 |
| prefer / require | 无 | SSL | `enable.ssl.certificate.verify=false` + `ssl.endpoint.identification.algorithm=none`（加密但接受任意证书，对齐 redis `TcpTls{insecure:true}` 语义；prefer 不回退明文） |
| prefer / require | 有 | SASL_SSL | 同上 |
| verify-full | 无/有 | SSL / SASL_SSL | 系统默认 CA + 默认 hostname 校验（librdkafka 默认即系统证书库：Windows 走系统证书存储、macOS 走 Keychain/系统探针路径） |

## 2. 后端设计（src-tauri/src/db_browser/）

### 2.1 依赖与构建接线

- `src-tauri/Cargo.toml`：

  ```toml
  # Kafka 引擎：rdkafka 0.39（futures 异步壳 + librdkafka 2.12.1 C 内核随包分发）。
  # cmake-build：Windows 仅支持 CMake 构建路径；libz-static/zstd：压缩栈经 libz-sys/
  # zstd-sys 的 bundled C 源码自包含（lz4/snappy 为 librdkafka 内置，无需开关）；
  # ssl：OpenSSL 经 openssl-sys，同时自动启用 SASL SCRAM/OAUTHBEARER（内置实现，
  # 不需要 libsasl2）；tokio：默认 feature，与 app 运行时匹配。
  # 注意：禁用 0.38 系——其内置 librdkafka 对 OpenSSL 3.6 有 LNK2019 问题
  # （rust-rdkafka#819），2.12.1 已改用 SSL_get1_peer_certificate 分支。
  rdkafka = { version = "0.39", default-features = false, features = [
    "cmake-build", "libz-static", "zstd", "ssl", "tokio",
  ] }
  ```

- Windows CI：vcpkg OpenSSL 3.6.4（publish.yml 已有，SQLCipher 同源）零新增步骤；
  macOS CI：publish mac 双腿新增 `brew install openssl@3` + `OPENSSL_DIR` /
  `OPENSSL_ROOT_DIR` / `OPENSSL_STATIC=1`（obclient 车道已验证的组合；打破「mac 腿
  零工具链依赖」，是本接入在 CI 侧的唯一结构性代价）。
- 新车道 `test-kafka.yml`（tag `test_kafka` 触发）：四腿矩阵 + vcpkg/brew 段照抄
  test-obclient.yml；每腿对探针 crate 跑 `cargo build --release` + 运行期冒烟
  （`rd_kafka_version()` 断言 + 指向 127.0.0.1:1 的必败 metadata 调用验证 TLS/压缩
  符号齐备）；win-x64 加一腿 debug（回归 MSVC CRT 匹配）。

### 2.2 引擎变体与连接

- `DbEngine::Kafka(KafkaEngine)`：`KafkaEngine` 持有会话级 `rdkafka::config::ClientConfig`
  （连接参数的单一事实来源），`BaseConsumer` 惰性创建（metadata / CONSUME 共用），
  二期再挂 `FutureProducer`。**Kafka 没有连接级握手**：connect = 用 bootstrap 首地址
  `fetch_metadata`（timeout 8s，spawn_blocking 包裹——阻塞 API，with_oracle 先例）
  验证可达性与凭证，成功即建会话。
- 版本串：Kafka broker 不自报版本，取 `"Kafka · N brokers · <controller 主机>"`。
- `("kafka", _)` 臂进 `DbEngine::connect` 的元组分发；`db_connect` / `db_disconnect`
  零改动（consumer Drop 即断）。
- `list_databases`：返回空（前端对 kafka 隐藏库切换器，redis 同型的命令兼容空实现）。
- `bind_cancel_flag`：空操作（取消在 CONSUME 轮询间检查，见 §2.5）。

### 2.3 对象树映射（复用 CatalogNode / CatalogParent）

树的 kind 全部落进前端 `EXPANDABLE_KINDS` / `OBJECT_KINDS` 既有值域：

```
根层（CatalogParent::Root）
├─ Brokers        kind:'group'   detail: 集群 controller / broker 数
│   └─ broker     kind:'column'  detail: id · host:port · rack · [controller]
├─ Topics         kind:'group'   detail: topic 总数
│   └─ topic      kind:'table'   detail: 分区数 · 副本数 · 内部主题标记
│       └─ 分区   kind:'column'  detail: leader · ISR · watermark 区间
└─ Consumer Groups kind:'group'  detail: 活跃组数
    └─ group      kind:'table'   detail: 成员数 · 订阅 topic 数
        └─ topic  kind:'column'  detail: 该 group 在此 topic 的总 lag
```

- 内部主题（`__consumer_offsets` 等）默认折叠进「Topics」下的内部子组或加标记
  （一期：保留展示 + `internal` 标记，不做开关）。
- `CatalogParent::Schema { schema }` 的 schema 承载组名（`brokers` / `topics` /
  `groups`），`CatalogParent::Group` 复用为 topic→分区、group→topic 的第二层，
  `CatalogParent::Table` 供列扩展——与 redis 四层映射同一手法，前端不感知引擎差异。

### 2.4 详情（table_detail / object_definition 复用）

- **topic 详情**（`table_detail(schema="topics", table=<topic>)`）：分区表
  （partition · leader · replicas · ISR · begin/end offset · 消息数=end-begin）+
  配置项表（ConfigResource TOPIC 级，`cleanup.policy` / `retention.ms` 等）+ 汇总头。
  `TableDetail` 的 `ddl` 字段填 topic 配置的 JSON 视图（前端「定义」tab 展示配置清单，
  语义对齐「这个对象是什么」）。
- **group 详情**：assignment 表（topic · partition · committed offset · end offset ·
  lag · consumer-id · host），lag 红色高亮（warning_red 语义）。
- **broker 详情**：`object_definition` 出 id/host/rack/controller 标记的键值清单。

### 2.5 查询 DSL（db_query 复用，sqlsplit kafka 键早退）

查询框不是 SQL，是单行命令（sqlsplit 的 kafka 分支按行切分、`#` 注释丢弃，与 redis
分支同位早退）。命令集：

| 命令 | 说明 | 只读 |
| --- | --- | --- |
| `TOPICS` / `BROKERS` / `GROUPS` | 列表（列：名称/分区数/副本数…） | ✓ |
| `DESCRIBE TOPIC <t>` | 分区 + 配置（同树详情） | ✓ |
| `DESCRIBE GROUP <g>` | assignment + lag（同树详情） | ✓ |
| `CONSUME <topic> [PARTITION <p>] [FROM earliest\|latest\|<offset>\|ts=<ISO8601>] [LIMIT <n>]` | 消息查看：assign + seek → poll 循环 | ✓ |
| `SEND <topic> [KEY <k>] [PARTITION <p>] <payload>` | **二期**，readonly=false 才放行 | ✗ |

- CONSUME 实现要点：`BaseConsumer` 非阻塞 poll（100ms 间隔），行 = 消息
  （key · value · partition · offset · timestamp · headers 展开列）；二进制 key/value
  显示 `<N bytes>`（redis 值转换先例）；`FROM` 缺省 `earliest`，未指定 PARTITION 时
  全分区 assign；批流回批走既有 Channel 模型（300 行/批），`LIMIT` 受 `max_rows`
  口径约束（上限 50000 全局既有值）。
- **取消粒度 = poll 间**（比 redis 的单命令不可取消更强）：轮询循环每 100ms 查会话
  `AtomicBool`，命中即 `cancelled=true` 正常收尾。
- 只读白名单：`statement_is_readonly` 的 kafka 分支 = 命令动词 ∈
  {TOPICS, BROKERS, GROUPS, DESCRIBE, CONSUME}；`SEND` 与未知命令拒绝（保守放行，
  与 redis 同口径）。

### 2.6 lib.rs / mod.rs 改动面

**一期零新增 tauri command**——连接、树、详情、查询、取消、断开全走既有六命令；
二期 admin（建删 topic / 改配置）再议 `db_kafka_admin` 专项命令。

## 3. 校验（validate_db_connection kafka 分支）

- `host` 能解析出 ≥1 个 `host[:port]` 条目（host 段非空、端口 1–65535、总条目 ≤ 32）；
- `extra_options.saslMechanism` ∈ {PLAIN, SCRAM-SHA-256, SCRAM-SHA-512}；
- `ssl_mode=disable` 且有 username：允许保存但表单侧带 SASL_PLAINTEXT 警示 hint
  （不阻断——测试环境常见）。

## 4. 前端设计

### 4.1 引擎登记（DbHostsScreen 四张 Record + 类型）

- `dbConnectionsService`：`DbEngineKind` 加 `'kafka'`；`DbConnectionRecord` 加
  `extraOptions?: string`（JSON 文本，service 层负责字段 ↔ JSON 互转）。
- `ENGINE_DEFAULT_PORTS.kafka = 9092`；`ENGINE_LABELS.kafka = 'Kafka'`（产品名不走
  i18n）；`ENGINE_ICONS.kafka = KafkaBrandIcon`（simple-icons `apachekafka`，CC0，
  engineIcons.tsx 的 brandIcon 同型）；`ENGINE_COLORS.kafka` 用现有七色外的一档
  （`terminal-orange`，styles.css 若无该 token 则补——Kafka 品牌意象是黑白，取橙是
  终端色板的可辨识空位）。

### 4.2 连接表单 kafka 分支

```
┌ 引擎选择器（第八个按钮，Kafka 图标 + 橙色）
├ 名称 / 颜色 / 环境级别 / 只读开关        ← 既有公共字段
├ Bootstrap 服务器   [多行输入]            ← host 列；hint「每行一条 host[:port]，
│                                            不带端口的条目用下方端口」
├ 端口               [9092]               ← hint「仅对未带端口的地址生效」
├ SASL               [用户名][密码]
│                    [机制 ▾]（PLAIN/SCRAM-SHA-256/SCRAM-SHA-512；用户名空时禁用）
│                    （disable 档显示明文警示 hint）
├ TLS                [四档下拉 ▾]          ← SSL_MODES 复用
├ 高级               [client_id 输入]      ← extra_options.clientId
└ SSH 隧道           ← 既有字段（一期连接层仍报「后续版本接入」）
```

- 机制下拉用自绘下拉组件（项目已替换原生 select 的同款）；
- 多行 bootstrap 输入在 service 层 join 成逗号分隔串存 host 列（读时 split 还原）；
- **不提供裸 JSON 编辑器**——extra_options 的存在对用户透明。

### 4.3 会话主内容区（DbSessionView kafka 分支）

沿用「左树 + 上查询框 + 下流式结果」骨架（复用流式管道与多结果 tab），引擎差异点：

- 欢迎语句：连上后自动 `TOPICS` 摘要（redis 连接后 PING 的同型）；
- 树图标：Brokers=`Server`、Topics=`Layers`、Groups=`Users`、topic=`MessagesSquare`、
  分区/column=`AlignHorizontalJustifyStart`（lucide，色板沿用现有树语义）；
- **点 topic → 按分区数生成 `CONSUME <topic> FROM latest LIMIT 100` 填入查询框并执行**
  （redis 点 key 按类型生成读命令的同型交互）；右键 topic/group →「详情」（table_detail
  弹层，分区/配置或 assignment 三 tab）；
- 查询框占位提示：`CONSUME orders FROM ts=2026-10-23T00:00:00Z LIMIT 500`；
- 结果表列随命令变体（TOPICS：名称/分区/副本/内部标记；CONSUME：offset/partition/
  key/value/timestamp/headers…），key/value 二进制渲染 `<N bytes>`。

### 4.4 i18n（zh-CN / en-US 同构，双语言必须同批落）

新增键（前缀 `dbHosts.` / `dbSession.`）：

```
dbHosts.kafkaBootstrapLabel / kafkaBootstrapHint / kafkaPortHint
dbHosts.kafkaSaslSection / kafkaSaslMechanism / kafkaSaslPlaintextWarn
dbHosts.kafkaAdvanced / kafkaClientId
dbSession.kafkaWelcome / kafkaConsumePlaceholder
dbSession.kafkaGroupBrokers / kafkaGroupTopics / kafkaGroupGroups
dbSession.kafkaTabPartitions / kafkaTabConfig / kafkaTabAssignment
dbSession.kafkaLag / kafkaInternalTopic / kafkaViewMessages
```

（实现时按最终 UI 微调，中英同构、键清单以 diff 为准。）

## 5. 分期与改动文件清单

| 分期 | 文件 | 改动 |
| --- | --- | --- |
| 一期 | `src-tauri/Cargo.toml` | rdkafka 依赖（§2.1） |
| | `src-tauri/src/db.rs` | 建表列 + 幂等 ALTER `extra_options` + Record 字段 + 校验分支 + kafka_options 解析 |
| | `src-tauri/src/db_browser/engines.rs` | `KafkaEngine` + connect/catalog/table_detail/exec_streaming/list_databases 臂 |
| | `src-tauri/src/db_browser/sqlsplit.rs` | kafka 键早退（行切分）+ CONSUME 参数解析 + 只读白名单 |
| | `src-tauri/src/db_browser/mod.rs` | 模块头文档更新（六引擎） |
| | `src/components/dbhosts/engineIcons.tsx` | KafkaBrandIcon |
| | `src/components/dbhosts/DbHostsScreen.tsx` | 四张 Record + 表单 kafka 分支 |
| | `src/components/db/DbSessionView.tsx` | kafka 分支（欢迎/树图标/命令生成/结果列） |
| | `src/services/dbConnectionsService.ts` | 类型 + extraOptions 组装/还原 |
| | `src/i18n/locales/{zh-CN,en-US}.ts` | §4.4 键 |
| | `.github/workflows/publish.yml` | mac 双腿 brew openssl@3 + env |
| | `.github/workflows/test-kafka.yml` | 新车道（tag `test_kafka`） |
| | `docs/kafka-engine.md` | 本文档 + 人工验证清单（§6） |
| | `docs/sql/2026-10-23-db-connections-extra-options.sql` | 表结构核对 |
| 二期 | engines.rs / sqlsplit.rs | `SEND` + FutureProducer + readonly 拦截 |
| | mod.rs + 前端 | `db_kafka_admin`（建删 topic / 配置变更）+ OAUTHBEARER |

门禁（局部口径）：`cargo check -p app --lib` / `cargo test -p app --lib kafka` /
`cargo clippy -p app --all-targets` + `pnpm run typecheck`；真集群行为走 §6 人工清单，
门禁不依赖。

## 6. 人工验证清单（需真实 Kafka，实现后填实测结果）

1. 明文无认证集群（PLAINTEXT）：连接 → 树三组 → CONSUME earliest/latest/offset/ts。
2. SASL/PLAIN + TLS（`prefer` 档接受自签）：连接、树、消费；`verify-full` 对自签证书
   报错（预期失败路径）。
3. SCRAM-SHA-512 集群：凭证错误报错文案可诊断。
4. 只读会话输入 `SEND`（二期前输入未知命令）：拒绝文案。
5. 大分区 LIMIT 截断（truncated 标记）与取消（消费中途点停止）。
6. 老库升级：已有 db_connections 数据的库启动 → `extra_options` 列补齐、老引擎连接
   不受影响。
7. Windows 真机（CI 车道外）：安装包形态下 kafka 连接（TLS 走系统证书库路径）。

## 7. 已知边界与风险

- **CI 侧待实测**（调研报告可信度分级的遗留项）：cmake crate 在 win-arm64 的
  generator 传参；MSVC debug CRT 匹配（车道加 debug 腿回归）；mac 腿 brew 依赖引入后
  的冷缓存时长。
- `fetch_metadata` / ConfigResource 拉取是阻塞 API，一律 spawn_blocking（with_oracle
  同型）；rdkafka 的 AdminClient 二期接入时在 blocking 线程上驱动。
- 只读拦截是客户端层面（与 redis 同声明）：生产集群应配 ACL，勿依赖本工具拦截。
- 内部主题（`__consumer_offsets`）消息格式不可读（二进制批次），CONSUME 该类 topic
  结果即 `<N bytes>`——属预期行为不是缺陷。
- GSSAPI / OAUTHBEARER（一期）/ 事务 API / schema registry 不在范围。
