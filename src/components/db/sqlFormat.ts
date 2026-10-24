/**
 * SQL 格式化路由（SqlEditor 与 ReadOnlySqlView 共用）：
 * - **≤ 200 行：直接前端** sql-formatter（MIT，方言解析质量更好；懒加载
 *   独立 chunk，首次使用后常驻）；
 * - **> 200 行：先走后端** `db_format_sql`（sqlformat，native 快）；后端
 *   检出 PL/SQL 块（函数/过程/触发器体，词界 BEGIN/DECLARE）仍让位回
 *   前端——sqlformat 是语句级格式化器，块内 := / loop / end if 会打乱，
 *   大存储过程也不例外。
 * 关键字大小写保留、2 空格缩进，两路口径一致。
 */

import type { SqlLanguage } from 'sql-formatter';

import { dbFormatSql } from '@/services/dbConnectionsService';

/** 超过此行数先走后端（native），否则前端直格 */
const FORMAT_FRONTEND_MAX_LINES = 200;

/** 前端方言映射（引擎名 → sql-formatter language；未知引擎默认 plsql——
 * 让位场景即 Oracle/达梦的 PL/SQL 块） */
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

/** 格式化一段 SQL 文本（行数路由 + PL/SQL 兜底，见模块头注释） */
export async function formatSqlText(source: string, engine?: string): Promise<string> {
  const lineCount = source.split('\n').length;
  if (lineCount > FORMAT_FRONTEND_MAX_LINES) {
    const outcome = await dbFormatSql(source, engine);
    if (outcome.type === 'formatted') {
      return outcome.text;
    }
    // PL/SQL 块：后端让位，落到下方前端方言格式化
  }
  const { format } = await import('sql-formatter');
  return format(source, { language: fallbackLanguage(engine), tabWidth: 2 });
}
