/**
 * SQL 格式化双路（SqlEditor 与 ReadOnlySqlView 共用）：
 * - 主路：后端 `db_format_sql`（sqlformat，语句级 SQL；Oracle/达梦引擎的
 *   PL/SQL 块会被后端拒绝——sqlformat 是语句级格式化器，块内 := / loop /
 *   end if 会打乱）；
 * - 回落路：前端 sql-formatter（MIT）的方言解析（懒加载独立 chunk，不走
 *   回落零成本；方言默认 plsql——回落实际只发生在 Oracle/达梦的 PL/SQL
 *   块）。关键字大小写保留、2 空格缩进，两路口径一致。
 */

import type { SqlLanguage } from 'sql-formatter';

import { dbFormatSql } from '@/services/dbConnectionsService';

/** 回落方言映射（引擎名 → sql-formatter language；未知引擎默认 plsql——
 * 后端只对 Oracle/达梦拒绝，回落即 PL/SQL 场景） */
export function fallbackLanguage(engine?: string): SqlLanguage {
  switch (engine) {
    case 'mysql':
      return 'mysql';
    case 'sqlite':
      return 'sqlite';
    case 'postgres':
      return 'postgresql';
    default:
      return 'plsql';
  }
}

/** 格式化一段 SQL 文本（双路自动择优；失败向上抛错由调用方展示） */
export async function formatSqlText(source: string, engine?: string): Promise<string> {
  try {
    return await dbFormatSql(source, engine);
  } catch {
    const { format } = await import('sql-formatter');
    return format(source, { language: fallbackLanguage(engine), tabWidth: 2 });
  }
}
