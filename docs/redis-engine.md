# Redis 引擎（数据库工作台）

> 数据库工作台在 MySQL / PostgreSQL / Oracle 之外的第四个引擎。复用现有工作台的连接表单、对象树懒加载、查询编辑器、结果表格与只读拦截，不新建独立页面。
>
> **驱动**：crates.io `redis = "1.7.1"`（纯 Rust 异步客户端），features `tokio-comp` + `tokio-rustls-comp` + `tls-rustls-insecure` + `cluster-async` + `streams`。TLS 走 rustls 0.23，与 app 全局 aws-lc-rs provider 同源——**`native-tls` / `ring` 变体一律不得启用**，同进程出现两个 crypto provider 会在运行期 panic；`cluster-async` 提供集群拓扑的异步连接（MOVED/ASK 重定向与 slot 路由）。
>
> **表结构**：一条新列 `db_connections.extra_options TEXT`（JSON，幂等 ALTER 自动补列；详见第 9 节与 `docs/redis-db-changes.sql`）。
>
> **拓扑**：单机（默认）/ 集群（Cluster）/ 哨兵（Sentinel），见第 2 节；拓扑与节点清单存在 extra_options 列。

---

## 1. 概述

Redis 不是关系型数据库：没有 schema、没有 SQL、没有列定义。接入工作台时按「一个引擎适配层」处理，与另三个引擎的差异全部收敛在 `db_browser` 的引擎分支里：

| 维度 | 关系型三引擎 | Redis |
|---|---|---|
| 顶层对象 | database / schema | 逻辑库 `db0`..`dbN` |
| 第二层 | 表 / 视图 / 函数 / 过程 / 序列 | 数据类型分组（string/hash/list/set/zset/stream） |
| 第三层 | 表 | key |
| 第四层 | 列 | 字段 / 成员 / 元素 |
| 查询语言 | SQL（sqlparser 分句） | 逐行原始 Redis 命令 |
| 结果形状 | 行 × 列 | 由 `redis::Value` 形态决定（见第 4 节） |

单会话仍是「一会话一连接、串行执行」模型，不做连接池。这直接决定了第 8 节的若干边界（阻塞命令、交互订阅）。

---

## 2. 连接配置

### 字段

表单与另三个引擎共用，Redis 语义如下：

| 字段 | 必填 | Redis 语义 | 缺省 |
|---|---|---|---|
| 名称 | 是 | 连接记录显示名，如 `prod-redis` | — |
| 引擎 | 是 | 选 `redis` | — |
| 地址 | 是 | 服务器主机名或 IP | — |
| 端口 | 是 | Redis 服务端口 | `6379` |
| 用户名 | 否 | ACL 用户名（Redis 6+）。空 = 只用密码认证（`AUTH <password>`，等价 default 用户） | 空 |
| 密码 | 否 | `requirepass` 或该 ACL 用户的密码；明文存 SQLCipher 库（与 `zk_credentials` 同级安全水位） | 空 |
| 逻辑库 | 否 | `db0`..`dbN`，落 `db_connections.database` 列。空 = 连接时用 db0 | 空（= db0） |
| SSL | 是 | 见下方映射 | `disable` |
| 环境 / 只读 / 颜色 | 否 | 与另三个引擎同义；**只读默认开启**（表列默认 1） | — |

逻辑库取值只写库名（`db2`），不写数字、不写斜杠。`SELECT` 由引擎层在连接建立后执行。

### 拓扑与 mTLS（extra_options）

连接表单在引擎 = redis 时提供「拓扑」选择，配置落在 `extra_options` 列（JSON 文本，结构对应后端 `RedisExtraOptions`，serde camelCase）：

| 字段 | 取值 | 说明 |
|---|---|---|
| `topology` | `single`（默认/缺省）\| `cluster` \| `sentinel` | 连接路径分发 |
| `nodes` | `host:port` 数组 | 集群 = 种子节点（可只给一个，slot 表自动发现其余）；哨兵 = 哨兵节点地址 |
| `masterName` | 字符串 | 哨兵拓扑必填：`SENTINEL get-master-addr-by-name` 解析当前 master |
| `clientCertPath` / `clientKeyPath` | PEM 文件路径 | mTLS 客户端证书与私钥；两路径成对、仅 SSL ≠ disable 时生效 |

行为要点：

- **集群**：全部节点作为初始节点交给 redis-rs `ClusterClient`（cluster-async）；只认 db0（切库下拉禁用，`SELECT` 报错拦截）。**无 key 命令（SCAN/CONFIG/INFO/DBSIZE）只路由到单个节点**——对象树与大 key 分析在集群下是单节点口径；键操作命令（GET/HSET/DEL…）按 slot 全集群路由，MOVED/ASK 自动重定向。
- **哨兵**：逐个哨兵问 `SENTINEL get-master-addr-by-name`，先答先得；解析出 master 后按单机拓扑连接（TLS 档位/凭据/mTLS 沿用连接记录）。主从切换后重连即指向新 master（不做运行期跟踪）。哨兵通常无认证：带凭据连失败时自动按无凭据重试一轮（master 的认证始终用记录凭据）。
- **mTLS**：证书经 `Client::build_with_tls` 挂进地址（redis-rs 的 `TlsConnParams` 不对外构造，这是唯一通道）；集群侧先对单节点 build 再取回 `ConnectionInfo` 喂给 `ClusterClient`。自建 CA 请装进系统信任库（不开 root 证书路径字段）。

### SSL 与 URL 判定

`ssl_mode` 决定 scheme，其余字段拼成连接串：

| `ssl_mode` | scheme | TLS 行为 |
|---|---|---|
| `disable` | `redis://` | 明文 |
| `prefer` | `rediss://` | 建立 TLS，**接受任意证书**（不做校验）；客户端实现上与 `require` 等价，不做明文回退 |
| `require` | `rediss://` | 同上（加密即可，不验证身份） |
| `verify-full` | `rediss://` | 用系统根证书校验服务端证书与主机名 |

举例（主机 `10.0.0.5`、端口 6379、逻辑库 `db2`）：

```
ssl_mode = disable      → redis://10.0.0.5:6379/2
ssl_mode = require      → rediss://10.0.0.5:6379/2
ssl_mode = verify-full  → rediss://10.0.0.5:6379/2   （额外校验系统根证书）
```

带 ACL 用户名与密码时：

```
redis://appuser:s3cret@10.0.0.5:6379/0
```

> `verify-full` 要求服务器证书由系统信任的 CA 签发且主机名匹配。自签证书场景请用 `require`（加密但不验证），否则连接会被拒绝——这是有意设计，不做「校验失败自动降级」。

---

## 3. 对象树映射

### 四层

对象树复用现有 catalog 协议（`CatalogNode` / `CatalogParent`），只是把关系型语义换成了 Redis 语义：

| 层 | Redis 语义 | `CatalogNode.kind` | 说明 |
|---|---|---|---|
| 1 | 逻辑库 `db0`..`dbN` | `database` | 根层，一行一个库 |
| 2 | 数据类型分组 string/hash/list/set/zset/stream | `group` | 空分组不返回（该库没有该类型的 key 就不出现） |
| 3 | key | `table` | `detail` 携带 TYPE（`string`/`hash`/…） |
| 4 | 字段 / 成员 / 元素 | `column` | 该类型的成员视图，`detail` 携带成员类型或分值等附加信息 |

前端不感知引擎差异：展开节点时带父游标请求下一层，节点 `kind` 决定渲染图标与右键菜单项。

### 懒加载与 5000 key 上限

- **根层**：`CONFIG GET databases` 读 `databases` 配置项得到逻辑库数量，生成 `db0`..`dbN`。CONFIG 命令被 `rename-command` 改名或禁用时，回退为固定的 16 个库（Redis 默认值），不报错。
- **分组层**：固定六项，只列出该库中真实存在的类型——先 `SCAN` 一遍拿到 key 与其 TYPE，再按类型归并；某类型一个 key 都没有则该分组不出现。
- **key 层**：`SCAN` 游标迭代收集 key 名，同时用管道（pipeline）批量 `TYPE` 归类，避免每 key 一次往返。
- **key 上限 5000**：单个类型分组最多收集 5000 个 key，达到上限即停止迭代并按截断呈现。原因是大库 `SCAN` 全量会把对象树展开拖到分钟级；5000 已足够定位日常 key，全量枚举请用查询框的 `SCAN` 命令自行翻页。
- **成员层**：按 key 的实际类型调用对应读命令（见第 6 节），一次取回。

> `SCAN` 是增量迭代，不保证同一游标周期内的强一致快照：展开过程中新增/删除的 key 可能出现在也可能不出现在本次结果里。这是 Redis 的固有语义，不是客户端缺陷。

---

## 4. 查询与结果

### 逐行命令

查询框接受**原始 Redis 命令，一行一条，不是 SQL**。`sqlsplit` 的 redis 分支按换行切分：

- 空行丢弃；
- `#` 开头的行按注释丢弃；
- 行内不按分号切分（Redis 命令本就不以分号结尾，值里出现分号也很常见）；
- 每行原样作为一条命令发给服务端，结果按行分块呈现（与 SQL 编辑器的多语句结果同构）。

参数按 Redis 协议的空格分隔规则解析，值里有空格请用双引号包裹（`SET k "hello world"`）。

### 示例

```
GET user:1001
HGETALL user:1001
ZRANGE leaderboard 0 9 WITHSCORES
SCAN 0 COUNT 100
TYPE user:1001
TTL user:1001
INFO memory
```

写命令（非只读会话）：

```
SET user:1001:name "alice"
HSET user:1001 name alice age 30
EXPIRE user:1001 3600
```

### `redis::Value` → 表格

| `redis::Value` 形态 | 表头 | 说明 |
|---|---|---|
| RESP2 扁平数组（普通多值返回，如 `LRANGE`、`KEYS`） | `index` / `value` | 每个元素一行，`index` 从 0 起 |
| RESP2 扁平数组（field-value 成对语义，如 `HGETALL`、`CONFIG GET`） | `field` / `value` | 每两个元素折成一行 |
| RESP3 Map（`HGETALL` 在 RESP3 下的返回） | `key` / `value` | 直接按 Map 展开 |
| Set（RESP3 的 `SMEMBERS`） | `member` | 每成员一行 |
| 标量（Bulk / Simple String / Integer / Double / Boolean / BigNumber / Verbatim） | `result` | 单行单列 |
| Nil | `result` | 单元格显示 `(nil)` |
| **INFO 的 bulk 文本**（命令感知分支） | `field` / `value` | 按行拆 `# Section` 分节（占 field 列）与 `k:v` 行，不再整块塞一格 |
| **SCAN 的 `[cursor, [key…]]`**（命令感知分支） | `cursor` / `key` | 游标只在首行首列出现；`'0'` 表示迭代完毕，空页保留一行游标供前端判断 |

数组元素若自身是嵌套数组（如 `XRANGE` 的 entry、`CLIENT LIST` 之外的复合结构），按 JSON 文本压进单元格，不递归建表——需要拆字段时改用更窄的命令（如 `XRANGE k - + COUNT 10` 后自行取列）。

### SCAN 游标翻页与命令补全

- **翻页**：SCAN 结果的 footer 附「继续迭代 ▸」按钮，点击自动以首行游标执行 `SCAN <cursor> COUNT <原命令的 COUNT，缺省 100>`；游标为 `'0'`（迭代完毕）时隐藏。
- **补全**：Redis 会话的查询编辑器模型语言换 plaintext（逐行命令不是 SQL），挂静态命令词表补全（`src/components/db/redisCommands.ts`，约 200 条按分组整理自 redis.io/commands，离线无网络依赖）。

### 行数上限与取消

- **行数上限**：沿用工作台口径，`max_rows` 默认 5000、可选 1000/5000/20000/50000，超限置 `truncated` 标记。截断发生在客户端组装表格时，服务端仍会返回完整回复（Redis 协议无分页，命令级没有 LIMIT 语义）。
- **取消**：粒度为「每条命令」——一条命令是单次往返，发出后只能等回复，没有逐行取消。点取消后当前命令的回复仍会被读完（或连接被放弃），后续行不再执行。长耗时命令请自行加 `COUNT` / `LIMIT` 约束，别把 `KEYS *` 这类全量命令丢进大库。

---

## 5. 只读模式白名单

只读会话（连接记录 `readonly = 1`，新建连接默认如此）在执行前逐行判定，**白名单默认拒绝**：不在表内的命令直接报错，不发送到服务端。

| 类别 | 放行命令 |
|---|---|
| string | `GET` `MGET` `STRLEN` `GETRANGE` |
| hash | `HGET` `HMGET` `HGETALL` `HKEYS` `HVALS` `HLEN` `HEXISTS` `HSCAN` |
| list | `LRANGE` `LLEN` `LINDEX` |
| set | `SMEMBERS` `SCARD` `SISMEMBER` `SMISMEMBER` `SSCAN` |
| zset | `ZRANGE` `ZREVRANGE` `ZSCORE` `ZMSCORE` `ZCARD` `ZCOUNT` `ZRANK` `ZSCAN` |
| stream | `XRANGE` `XREVRANGE` `XLEN` `XINFO` |
| 通用 | `SCAN` `TYPE` `TTL` `PTTL` `EXISTS` `DBSIZE` `INFO` |

**子命令容器规则**：`CONFIG` / `CLIENT` / `MEMORY` / `OBJECT` 四个命令本身既有读也有写，只放行读子命令：

| 命令 | 放行子命令 | 拒绝示例 |
|---|---|---|
| `CONFIG` | `GET` | `CONFIG SET`、`CONFIG REWRITE`、`CONFIG RESETSTAT` |
| `CLIENT` | `LIST` `INFO` `GETNAME` `ID` | `CLIENT KILL`、`CLIENT NO-EVICT`、`CLIENT PAUSE` |
| `MEMORY` | `USAGE` `STATS` `DOCTOR` | `MEMORY PURGE` |
| `OBJECT` | `ENCODING` `REFCOUNT` `IDLETIME` `FREQ` | —（OBJECT 本身只读，仍按子命令枚举） |

> 白名单是「保守放行」而不是「危险命令黑名单」：未列出的读命令同样会被拒绝。需要时用非只读会话，或补进白名单（实现集中在 `sqlsplit` 的 redis 分支，只改一处）。
>
> 只读是**客户端拦截**，不是服务端 ACL。生产库请同时在 Redis 侧建只读 ACL 用户，两道防线不互相替代。

---

## 6. 键详情与命令模板

对象树里双击 key（或右键「键详情」）走 `table_detail` / `object_definition`，返回一个信息块加一份可直接执行的读命令：

| 行 | 来源 | 说明 |
|---|---|---|
| `KEY` | key 名 | — |
| `TYPE` | `TYPE k` | string/hash/list/set/zset/stream/none |
| `TTL` | `TTL k` | `-1` = 无过期，`-2` = key 不存在 |
| `MEMORY USAGE` | `MEMORY USAGE k` | 字节数；命令被禁用时该行省略 |
| `ENCODING` | `OBJECT ENCODING k` | 内部编码（`embstr`/`listpack`/`quicklist`/`intset`/`hashtable`/`skiplist`/`stream` 等） |
| `LENGTH` | 类型专属命令 | 见下表 |

按类型生成的读命令（前端一键填入查询框）：

| 类型 | LENGTH 来源 | 生成的读命令 |
|---|---|---|
| string | `STRLEN k` | `GET k` |
| hash | `HLEN k` | `HGETALL k` |
| list | `LLEN k` | `LRANGE k 0 -1` |
| set | `SCARD k` | `SMEMBERS k` |
| zset | `ZCARD k` | `ZRANGE k 0 -1 WITHSCORES` |
| stream | `XLEN k` | `XRANGE k - +` |

> 命令模板一律是**读命令**，即使会话非只读也不生成写命令。`LRANGE k 0 -1` 在大 list 上会一次拉全量，列表很长时请改成分页区间（`LRANGE k 0 99`）。

---

## 7. 键管理 / 慢日志 / 大 Key 分析（UI）

对象树右键 Redis key（写入口仅非只读会话显示；「键详情」即原有检视块）：

| 入口 | 行为 | 后端 |
|---|---|---|
| 编辑值 | 按类型呈现表单（string=多行文本；hash/zset=两列行编辑；list=可上下移的行编辑；set=成员行编辑），保存 = **DEL + 按类型重写 + 恢复原 TTL** 的 MULTI/EXEC 原子管道（同 key 同 slot，集群下也成立）；保存后自动回看该键 | `redis_read_key` / `redis_write_key` |
| 修改 TTL | 显示当前 TTL；输入秒数 `EXPIRE`，或「移除过期」`PERSIST` | dbQuery 组行命令（跨库时 SELECT 夹心） |
| 删除键 | 确认弹窗 → `DEL` | dbQuery 组行命令 |

安全闸：**成员超 5000（REDIS_EDIT_CAP）的键 truncated=true，表单只读禁存**——整体覆写语义下截断保存会丢数据；清空全部成员 = 删除该键；只读会话后端拒绝 `redis_write_key`。stream 等无表单语义的类型提示用命令。

执行工具栏（Redis 会话专属，只读可用）：

- **慢日志**：`SLOWLOG GET 200` 结构化为 id/时间/耗时/命令/客户端 五列表（可过滤、可刷新）；
- **大 Key 分析**：SCAN 采样至多 1000 个 key，批量 `MEMORY USAGE` + `TYPE`，按内存倒序取前 50；footer 显示实扫数与「采样触及上限」标记。托管实例禁用 MEMORY USAGE 时单 key 按 0 字节计（排序靠后）。

---

## 8. 已知边界

以下能力本期不做，各自的原因只有一条：

| 不支持 | 原因 |
|---|---|
| pub/sub 交互订阅（`SUBSCRIBE` / `PSUBSCRIBE`）、MONITOR 实时流 | 订阅会独占连接并持续推送，与串行「发一条等一条」的会话模型互斥 |
| 阻塞命令（`BLPOP` / `BRPOP` / `BLMOVE` / `WAIT` 等） | 会挂死串行会话，后续命令全部排队等它超时（未做黑名单拦截：只读会话被白名单默认拒绝挡住，可写会话会真挂到超时/取消） |
| Lua 脚本调试（`SCRIPT DEBUG`） | 调试是阻塞式交互会话，同 pub/sub 的连接独占问题 |
| Streams 消费者组管理 | 无专用表单（建组/认领/ack 只能用查询框裸命令 `XGROUP` / `XACK` / `XAUTOCLAIM`） |
| 命令内分页 | 协议层无分页，`max_rows` 只在客户端截断显示，服务端回复仍完整返回 |
| 集群下的全库枚举 | redis-rs 集群连接把无 key 命令（SCAN/CONFIG/INFO）路由到单个节点——对象树/大 key 分析是单节点口径；键操作命令全集群路由 |
| 哨兵运行期主从切换跟踪 | 连接时解析一次 master 地址；故障转移后需重连（重连即指向新 master） |
| RESP3 协商 | 保持 RESP2（老版本 Redis 无 HELLO）；值转换层对两种形态都兼容 |

另有两个使用层面的注意点（不是缺陷）：

- **`SCAN` 无强一致快照**：迭代期间库在变，结果可能重复或遗漏，需按业务容忍。
- **`KEYS` 不在只读白名单**：大库上会阻塞服务端，请用 `SCAN`。

---

## 9. 数据库变更

**一条新列，幂等 ALTER 应用启动自动补列，零手工迁移**：

| 项 | 结论 |
|---|---|
| 新表 | 无 |
| 新列 | `db_connections.extra_options TEXT`（可空 JSON）——引擎通用扩展槽位，当前仅 Redis 消费：拓扑/节点/master 名/mTLS 证书路径（结构 = 后端 `RedisExtraOptions`） |
| 迁移脚本 | 无需手工执行（db.rs 幂等 ALTER 批次内置；核对语句见 `docs/redis-db-changes.sql`） |
| `database` 列 | 存 Redis 逻辑库 `dbN`（可空，空 = 连接时 db0；集群恒 db0） |
| `service_name` 列 | 对 Redis 恒为 `NULL`（该列是 Oracle 服务名专用） |
| `engine` 列取值 | `'redis'`（列是 TEXT，无 CHECK 约束） |
| `port` | 表列无默认值，`6379` 只是表单缺省；集群/哨兵拓扑下 host/port 仅展示用（真实地址在 extra_options.nodes） |

---

## 10. 验证清单

> **需要真实 Redis 实例**（本机 `redis-server` 或容器均可，建议 6.x+ 以覆盖 ACL 与 RESP3 路径）。以下为人工冒烟步骤，门禁不依赖。

| # | 操作 | 预期 |
|---|---|---|
| 1 | 新建连接：引擎选 `redis`，地址/端口填真实实例，SSL = 不加密，只读关闭 | 保存成功；列表项显示 `redis` 引擎与端口 |
| 2 | 点「测试连接」 | 返回成功与耗时；密码错误时返回认证失败而非超时 |
| 3 | 连接进入工作台 | 顶栏显示当前库 `db0`；只读徽标按记录配置显示 |
| 4 | 展开对象树根层 | 列出 `db0`..`db15`（或 `CONFIG GET databases` 的实际值）；CONFIG 被禁用时应回退为 16 个且不报错 |
| 5 | 展开 `db0` | 只出现该库真实存在的类型分组（如 `string`、`hash`） |
| 6 | 展开某分组 | 列出 key，`detail` 显示类型；超过 5000 key 的库出现截断标记且展开不卡死 |
| 7 | 展开某 key | 列出字段/成员/元素，数量与该类型 LENGTH 一致 |
| 8 | 查询框执行 `GET <存在的 key>` | 结果表头 `result`，单元格为值 |
| 9 | 执行 `HGETALL <hash key>` | 结果表头 `field` / `value`，行数等于字段数 |
| 10 | 执行 `ZRANGE <zset key> 0 -1 WITHSCORES` | 结果表头 `index` / `value`（或 `field`/`value` 形态），分值可见 |
| 11 | 执行 `SCAN 0 COUNT 100` | 首行是游标，其后是 key 列表 |
| 12 | 双击 key 打开键详情 | 信息块含 KEY/TYPE/TTL/MEMORY USAGE/ENCODING/LENGTH；命令模板与类型匹配 |
| 13 | 工具栏切到 `db1`，再执行 `DBSIZE` | 对象树刷新为 db1；`DBSIZE` 返回 db1 的 key 数（与 db0 不同即为切库生效） |
| 14 | 打开只读会话，执行 `SET k v` | 被客户端拦截并提示「只读模式」；服务端未收到该命令（可在 `MONITOR` 侧确认） |
| 15 | 只读会话执行 `CONFIG GET maxmemory` | 放行并返回结果 |
| 16 | 只读会话执行 `CONFIG SET maxmemory 0` | 被拦截 |
| 17 | 非只读会话执行 `BLPOP q 0` | 会话阻塞至超时或取消；验证第 8 节的串行模型预期 |
| 18 | 把 SSL 改为 `verify-full` 连自签实例 | 连接被拒并给出证书校验错误（不回退明文） |
| 19 | 执行 `INFO memory` | 结果为 field/value 两列表，`# Memory` 分节行可见，不再整块塞一格 |
| 20 | 执行 `SCAN 0 COUNT 100` | 表头 cursor/key，游标在首行；footer 出现「继续迭代 ▸」，点击后以该游标翻页；迭代完结（游标 0）后按钮消失 |
| 21 | 查询框敲入 `HGET` | 补全弹出 hash 分组的命令建议（带参数骨架）；模型语言为 plaintext（SQL 高亮/补全不再出现） |
| 22 | 右键某 key → 编辑值（非只读） | 表单按类型呈现；改值保存后对象树刷新、自动回看该键；TTL 保留（可先 EXPIRE 验证） |
| 23 | 对 5000+ 成员的 key 打开编辑 | 表单只读并提示截断，保存按钮隐藏 |
| 24 | 右键 key → 修改 TTL | 输入秒数应用后 `TTL k` 生效；「移除过期」后 TTL 回 -1 |
| 25 | 右键 key → 删除键 | 确认后 key 从树上消失 |
| 26 | 只读会话右键 key | 无编辑/TTL/删除菜单项；工具栏「慢日志/大 Key」仍可用 |
| 27 | 工具栏「慢日志」 | 五列表呈现，可过滤可刷新 |
| 28 | 工具栏「大 Key 分析」 | 按内存倒序 Top50，footer 显示实扫数 |
| 29 | 新建集群连接（拓扑=集群，节点列表填种子节点） | 连接成功；切库下拉禁用（仅 db0）；键命令全集群路由；树/大 Key 为单节点口径 |
| 30 | 新建哨兵连接（拓扑=哨兵，节点=哨兵地址，master 名必填） | 连接成功（解析当前 master）；master 名写错时报「哨兵未找到 master」 |
| 31 | SSL=require + 证书/私钥路径（mTLS） | 握手携带客户端证书；只填其一时表单拒保存；disable + 证书时连接报配置矛盾 |
