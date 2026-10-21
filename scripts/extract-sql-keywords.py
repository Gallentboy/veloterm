#!/usr/bin/env python3
"""从 sqls-server/sqls 源码树提取 SQL 补全静态数据（一次性/升级时重跑）。

用法：
    python3 scripts/extract-sql-keywords.py /path/to/sqls src/editor/sqlData

产物（均为上游数据的机械转换，不做人工修订）：
    keywords.json   基础 ANSI 关键字 → 分类（matched / dml / ddl / dcl）
    dialects.json   方言关键字表 {mysql8, postgresql13, sqlite3}
    functions.json  MySQL 8 函数名数组

来源与版本锚定记录在 src/editor/sqlData/README.md。
"""

import json
import re
import sys
from pathlib import Path

BASE_MAP_RE = re.compile(r'"([A-Z_0-9]+)":\s*(Matched|DDL|DML|DCL)\b')


def extract_base_keywords(keyword_go: str) -> dict[str, str]:
    pairs = BASE_MAP_RE.findall(keyword_go)
    return {name: cls.lower() for name, cls in pairs}


def extract_string_list(source: str, var_name: str) -> list[str]:
    m = re.search(rf'var {var_name} = \[\]string\{{(.*?)\n\}}', source, re.S)
    if not m:
        raise SystemExit(f'未找到 var {var_name}')
    return re.findall(r'"([^"]+)"', m.group(1))


def main() -> None:
    if len(sys.argv) != 3:
        raise SystemExit(__doc__)
    sqls_root = Path(sys.argv[1])
    out_dir = Path(sys.argv[2])
    out_dir.mkdir(parents=True, exist_ok=True)

    keyword_go = (sqls_root / 'dialect' / 'keyword.go').read_text()
    mysql_go = (sqls_root / 'dialect' / 'mysql.go').read_text()
    postgresql_go = (sqls_root / 'dialect' / 'postgresql.go').read_text()
    sqlite_go = (sqls_root / 'dialect' / 'sqlite.go').read_text()

    keywords = extract_base_keywords(keyword_go)
    dialects = {
        'mysql8': extract_string_list(mysql_go, 'mysql8Keyword'),
        'postgresql13': extract_string_list(postgresql_go, 'postgresql13Keywords'),
        'sqlite3': extract_string_list(sqlite_go, 'sqliteKeywords'),
    }
    # 上游函数表混有运算符（% & -> := 等）与残缺项（"(JSON"）：
    # 只保留标识符形态的条目（机械形状过滤，非内容修订）
    functions = [
        name
        for name in extract_string_list(mysql_go, 'mysql8Function')
        if re.fullmatch(r'[A-Za-z_][A-Za-z0-9_]*', name)
    ]

    (out_dir / 'keywords.json').write_text(json.dumps(keywords, indent=1) + '\n')
    (out_dir / 'dialects.json').write_text(json.dumps(dialects, indent=1) + '\n')
    (out_dir / 'functions.json').write_text(json.dumps(functions, indent=1) + '\n')
    print(
        f'keywords={len(keywords)} '
        f'mysql8={len(dialects["mysql8"])} postgresql13={len(dialects["postgresql13"])} '
        f'sqlite3={len(dialects["sqlite3"])} functions={len(functions)}'
    )


if __name__ == '__main__':
    main()
