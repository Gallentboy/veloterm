# Oracle 错误消息全量中文译文（19c）

`src-tauri/assets/oracle-errors/zh-CN.json` —— Oracle Database 19c 官方错误
消息手册（`ORA-00000` ~ `ORA-65535`）全部 **20,362 条**的中文译文，构建期
压成嵌入表随应用分发。

## 数据性质（重要）

**这是机器辅助翻译产出，不是 Oracle 官方译文。** Oracle 官方文档站
（docs.oracle.com）**不提供中文错误消息**——实测 `zh-cn` / `zh` / `cn` / `zhs`
四种 locale 路径均返回 404，11g 与 21c 版本的错误消息手册同样只有英文。
Oracle 服务端返回的 ORA 文本本身也是英文。

因此本目录下的译文由 10 个并行翻译任务产出，经统一术语表约束与自动化质检
（条数/code 序列/占位符/空值镜像），但**未逐条人工复核语义地道程度**。用于
辅助理解报错含义足够，作为正式文档引用请以 Oracle 官方英文原文为准。

## 文件结构

```
assets/oracle-errors/
  zh-CN.json          # 译文（本目录，入库）
```

`zh-CN.json` 结构：

```json
{
  "_meta": {
    "source": "Oracle Database 19c Database Error Messages (English original)",
    "source_url": "https://docs.oracle.com/en/database/oracle/oracle-database/19/errmg/",
    "locale": "zh-CN",
    "entry_count": 20362,
    "note": "机器辅助翻译产出，非 Oracle 官方译文。..."
  },
  "entries": [
    { "code": "ORA-00942", "message": "表或视图不存在", "cause": "...", "action": "..." }
  ]
}
```

英文原文未入库（7.6 MB 明文）——需要时按下文「重新抓取」流程重新生成。

## 运行时形态（构建期生成）

`build.rs::build_oracle_error_table` 在构建时读取 `zh-CN.json`，产出两个
二进制到 `OUT_DIR`：

| 产物 | 大小 | 布局 |
|---|---|---|
| `oracle_errors.blob` | 约 3.6 MB | 每条 = `message \x1f cause \x1f action`（UTF-8 首尾相接） |
| `oracle_errors.idx` | 约 198 KB | 每条 10 字节小端：`u16 code` + `u32 offset` + `u32 len`，按 code 升序 |

运行期 `src/db_browser/oracle_messages.rs` 用 `include_bytes!` 嵌入两者：

```rust
pub fn lookup(code: u16) -> Option<OracleMessage<'static>>;
pub fn stats() -> (usize, usize);   // (条目数, blob 字节数)
```

**内存特性**：两者落在可执行文件只读段（`.rodata`），按需分页。查一条错误
只触达二分路径上的约 15 个索引元素（≤60 KB 页）与该条的 blob 字节，
**堆分配为零**（返回 `&'static str` 借用切片）。不建 HashMap、不做缓存。

二进制体积增加约 3.8 MB；稀疏查询下物理常驻内存增量可忽略，且被访问过的
只读页在内存压力下可被内核直接回收（下次访问再从磁盘读）。

## 构建期校验（硬失败）

`build.rs` 在生成前逐项校验，任一失败即中断构建（`fail()` → exit 1）：

- JSON 可解析、`entries` 数组存在
- `_meta.entry_count` 与实际条数一致（数据被截断/替换的早期信号）
- 每条四字段齐全且为字符串
- 错误码格式为 `ORA-NNNNN`、数值落在 `u16` 范围
- 错误码无重复（重复会让二分结果不确定）
- 字段内不含分隔符 `\x1f`（否则三段切分会错位）
- 索引长度 = 条数 × 10、末条区间精确覆盖到 blob 末尾

## 单元测试

`cargo test -p app --lib oracle_messages`（6 项）：

1. `stats_match_expected_scale` —— 条目数 20362、blob 尺寸区间
2. `lookup_common_codes` —— ORA-00942 / ORA-00001 命中且内容为中文
3. `lookup_boundaries` —— 首条 ORA-00000 与末条 ORA-65535
4. `lookup_missing_code_returns_none` —— 未收录号不误命中
5. `index_is_strictly_ascending` —— 索引严格升序（二分前提）
6. `sampled_entries_are_well_formed` —— 抽样 100 条三段切分干净 + 索引覆盖完整

## 重新抓取（需要更新到新版本 Oracle 时）

原始抓取逻辑：从 `ORA-00000.html` 起，逐页跟随 HTML 头部的
`<link rel="next" href="...">` 串成链表，直到 `next` 不再是 `ORA-` 前缀
（19c 共 26 页，末页后是 `ORADNFS-65000.html`）。每条错误在 HTML 里是
`<dt>ORA-NNNNN: message</dt>` + `<dd>Cause:</dd>` + `<dd class="ddexpand">Action:</dd>`；
`<span class="variable">string</span>` 占位符还原为 `string` 文本。

注意：原文有约 8.5%（1722 条）本身就没有 Cause/Action（`<p>` 为空），
译文对应留空——不是漏译。

## 相关

- 精简中文摘要表（80 条高频码，一句话说明）：`src-tauri/src/db_browser/engines/oracle_engine.rs` 的 `oracle_ora_hint`
- 查不到全量译文时，调用方应回落到该摘要表
