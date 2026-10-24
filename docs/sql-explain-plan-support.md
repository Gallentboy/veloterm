# SQL 执行计划（EXPLAIN）后端支持调研报告

> **落地状态（已更新）**：`db_explain` 命令已实现——方言装配全部对齐 DBeaver 开源实现（MySQL 裸 `EXPLAIN`（对 DBeaver `EXPLAIN EXTENDED` 的有据偏离：MySQL 8.0 已移除该关键字）/ PG `EXPLAIN [(ANALYZE, BUFFERS)]` / SQLite `EXPLAIN QUERY PLAN` / 达梦 `EXPLAIN` / Oracle `EXPLAIN PLAN SET STATEMENT_ID INTO <plan_table> FOR` + 原始 PLAN_TABLE 行读取，plan 表探测 PLAN_TABLE/TOAD_PLAN_TABLE 候选同 DBeaver getPlanTableName）；OB Oracle 模式明确拒绝（待 obclient 通道稳定）；Redis 拒绝。analyze 模式（真实执行语句）事前拒绝 DML（DBeaver 靠事后 rollback，我们无事务管理）。执行走 db_query 抽取出的同一条流式通道（取消/超时/行数上限全复用）；Oracle 的 EXPLAIN PLAN 落表动词是「辅助语句」（不占结果位次，失败整体报诊断错误）。**下文的真机验证清单仍然有效**——oracle_async 对 EXPLAIN PLAN 无结果集 verb 的响应帧、达梦 EXPLAIN 返回列类型两项需连真实实例确认。
>
> 结论先行：**MySQL / PostgreSQL / SQLite / OceanBase MySQL 模式的执行计划，现有 `db_query` 通道（sqlsplit 切分 + 只读白名单 + 流式结果）已经完备，前端拼 EXPLAIN 前缀即可用，后端零改动**；Oracle 系（Oracle / OB Oracle 模式）与达梦需要真机验证 3 个点（见各节「验证清单」）；唯一建议的后端增量是一个 `db_explain` 统一封装命令（已按此实现）。
>
> 配套前端：SQL 编辑器右键菜单「查看执行计划」调 `db_explain`，计划以结果集表格展示。

## 一、通道现状（全部引擎共用的底座）

| 组件 | 现状 | 对执行计划的意义 |
| --- | --- | --- |
| `db_query`（db_browser/mod.rs） | sqlsplit 切分 → 逐条执行 → Channel 流式回批 | 各引擎的 EXPLAIN **都是返回结果集的语句**，天然走通 |
| `sqlsplit::split_statements` | postgres 方言有 `EXPLAIN SELECT 1` 切分单测（sqlsplit.rs:579） | EXPLAIN 前缀不会被切分器误拆 |
| 只读白名单 AST 路（`READONLY_KIND_PREFIXES`） | 含 `"Explain"`、`"Analyze"`（sqlsplit.rs:319） | 只读会话（readonly=ON）可执行 EXPLAIN |
| 只读白名单降级路（`prefix_readonly`） | 前缀含 `EXPLAIN`（sqlsplit.rs:502） | 方言解析失败时 EXPLAIN 仍放行 |
| 行数上限 / 超时 / 取消 | 既有 `maxRows` / `timeoutMs` / `db_query_cancel` | 大计划输出天然受控 |

## 二、逐引擎分析

### MySQL（mysql-async-ob，含 OceanBase MySQL 模式）——无缺口

- `EXPLAIN <sql>`（传统表格式）、`EXPLAIN FORMAT=JSON <sql>`、`EXPLAIN ANALYZE <sql>`（8.0.18+，OB 4.x 支持）都是**普通结果集**，现有通道直接返回。
- **风险提示**：`EXPLAIN ANALYZE` 会**真实执行语句**（SELECT 真跑、DML 真改数据）。前端临时拼前缀形态只拼裸 `EXPLAIN`（只做优化器估算、不执行），是安全的选择；第二期若开放 ANALYZE 需对 DML 加拦截提示（`statement_is_readonly` 的 DML 判定可复用）。
- MySQL 5.7：裸 `EXPLAIN` 可用，FORMAT=JSON 可用，ANALYZE 不存在（语法报错，属预期行为）。

### PostgreSQL（tokio-postgres）——无缺口

- `EXPLAIN <sql>` 默认 TEXT 输出（单列逐行），结果面板按表格展示即可。
- `EXPLAIN (ANALYZE, BUFFERS) <sql>` 同为 TEXT 行；`FORMAT JSON` 返回 **json 类型**——已核实 `pg_cell` 值转换链含 `serde_json::Value` 臂（engines/sql_engines.rs:141），json/jsonb 列能正常字符串化。
- ANALYZE 同样真实执行，DML 场景注意事项同上。

### SQLite（rusqlite）——无缺口（已核实白名单）

- `EXPLAIN QUERY PLAN <sql>` 返回四列（id/parent/notused/detail）的只读结果。
- 已核实两条只读判定路径都放行 EXPLAIN（AST 路 `READONLY_KIND_PREFIXES` 含 Explain；降级路前缀含 EXPLAIN），**只读会话可用**。
- 裸 `EXPLAIN <sql>`（字节码形态）同理可用，但可读性差，前端默认用 QUERY PLAN 形态。

### Oracle（oracle_async 纯 Rust 驱动）——通道可用，3 个真机验证点

执行计划是**两段式**：`EXPLAIN PLAN FOR <sql>`（把计划写入 PLAN_TABLE，**隐式提交**——工作台无事务管理，无实际影响）→ `SELECT PLAN_TABLE_OUTPUT FROM TABLE(DBMS_XPLAN.DISPLAY())` 取格式化文本。前端把这两条语句一起提交，sqlsplit 切成两条逐条执行，第二个结果集即计划文本。

**验证清单（无真机 Oracle 环境无法在本机覆盖）：**

1. **oracle_async 对 `EXPLAIN PLAN FOR` 的执行路径**：它是非查询 verb（无结果集回包，类似 DML 的 affected 形态）。驱动当前对这类语句的响应帧处理需真机确认（预期走 affected=0 路径，与 DML 同型，风险低）。
2. **PLAN_TABLE 前提**：11g+ 默认有全局 `PLAN_TABLE$` 的同义词，普通用户可直接用；10g 及手工裁剪过的实例需要先执行 `@?/rdbms/admin/utlxplan.sql` 建表。第二期封装时应先探测（`SELECT 1 FROM PLAN_TABLE WHERE ROWNUM=1`），缺失时报可诊断提示而不是裸 ORA-00942。
3. **`TABLE(DBMS_XPLAN.DISPLAY())` 集合函数**：对驱动而言是普通 SELECT（输出列 `PLAN_TABLE_OUTPUT VARCHAR2(300+)`），但 table-function 结果集的列元数据形态未在 oracle_async 上实测过。

### 达梦（dm-pure）——通道可用，1 个真机验证点

- 达梦 `EXPLAIN <sql>` 直接返回结果集（JDBC 侧 getResultSet 拿到计划文本行），语法与前缀式一致，前端拼 `EXPLAIN` 已可用。
- **验证清单**：dm-pure 协议层对 EXPLAIN 返回的列类型（可能含长文本说明列）的解码——与普通查询同帧格式，风险低，但 EXPLAIN 的返回列集没有公开文档，需连真实 DM8 实例确认一次。
- 后续可选增强：达梦的 ET（Event Trace，`SET ET ON` + V$SQL_HISTORY 系）能拿运行时统计，属第二期以后。

### OceanBase Oracle 模式——通道理论同 Oracle，建议缓一步

- 语法同 Oracle（`EXPLAIN PLAN FOR` + `DBMS_XPLAN`），PLAN_TABLE 有兼容视图。
- 但 OB Oracle 的工作台对象层目前是 P0 未接入状态，且**另有并行会话正在改造 mysql-async-ob 的 OB Oracle 握手**（CLIENT_SUPPORT_ORACLE 能力位）——建议等该改造合入、OB Oracle 通道稳定后再做真机验证，避免双重返工。

### Redis——不适用

无执行计划概念，前端右键菜单在该模式下已隐藏「查看执行计划」。

## 三、第二期后端落地建议（本期不做）

1. **`db_explain` 统一命令**：入参 `(sessionId, sql, { analyze?: bool })`，后端按方言拼装（前端拼 SQL 的临时形态升级）：MySQL/OB `EXPLAIN [FORMAT=JSON]`、PG `EXPLAIN [(ANALYZE, BUFFERS)]`、SQLite `EXPLAIN QUERY PLAN`、Oracle 系两段式编排、达梦 `EXPLAIN`；结果复用 QueryEvent 流（前端零改动升级）。收益：多语句选区的逐条 EXPLAIN、Oracle PLAN_TABLE 探测降级、ANALYZE 的 DML 拦截都收口在一处。
2. **Oracle PLAN_TABLE 探测与降级提示**（见上文验证点 2）。
3. **ANALYZE 开关与 DML 防护**：`EXPLAIN ANALYZE`（MySQL 8 / OB4 / PG）真执行语句，命令层对 DML 入参默认拒绝、需显式开关。
4. **达梦 EXPLAIN 返回列解码的实测补丁**（如需要）。
5. **OB Oracle**：待 ob20 握手改造合入后验证 PLAN_TABLE$ / DBMS_XPLAN 兼容性，随 OB Oracle 工作台一并接入。
