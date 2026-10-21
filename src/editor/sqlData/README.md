# SQL 语言数据（vendor + 机械提取）

Monaco `sql` 语言补全的数据资产，对应 `src/editor/sqlLanguage.ts`（纯前端、
纯离线，nginxData / dockerData 同型约定）。

## 文件与来源

三份 JSON 均由 `scripts/extract-sql-keywords.py` 从
[sqls-server/sqls](https://github.com/sqls-server/sqls) 的 Go 源码
（`dialect/` 包）**机械提取**生成，不做人工内容修订：

- `keywords.json`：基础 ANSI 关键字 349 条 → 分类（matched / dml / ddl / dcl，
  上游 `dialect/keyword.go` 的 map，DML/DDL/DCL 分类随数据保留）。
- `dialects.json`：方言关键字表（`mysql8Keyword` 492 / `postgresql13Keywords`
  450 / `sqliteKeywords` 145，来自各方言文件）。
- `functions.json`：MySQL 8 函数名 483 条（上游 `mysql8Function` 507 条经
  **标识符形态过滤**——上游混有运算符 `%`/`&`/`->`/`:=` 与残缺项 `(JSON`，
  仅保留 `[A-Za-z_][A-Za-z0-9_]*` 形态条目）。

来源版本：commit `920fde7`（2026-08-20 克隆时的 master HEAD）。

sqls 的动态能力（表/列/视图的 schema 感知补全）依赖真实数据库连接内省
（十种驱动的 INFORMATION_SCHEMA 查询），**不在静态提取范围**——本目录只
承载其静态数据层。骨架 snippet 为项目自写，非上游内容。

## 许可证

sqls 为 **MIT License**，无 GPL 系商用确认负担。

## 消费方

`src/editor/sqlLanguage.ts`：dynamic import 本目录三份 JSON（独立 async
chunk，不打开 .sql 文件的会话零加载成本）。

## 升级流程

1. 克隆 sqls 新版本到临时目录；
2. `python3 scripts/extract-sql-keywords.py /path/to/sqls src/editor/sqlData`；
3. 核对输出条数与本文件记录；更新本文件的 commit 记录；
4. 跑 `pnpm run typecheck` 与 `pnpm run build`；
5. 冒烟：.sql 文件 → 关键字/函数/snippet 补全，FROM/JOIN 后不出函数。
