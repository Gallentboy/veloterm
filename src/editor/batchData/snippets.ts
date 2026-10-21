/**
 * Windows Batch 补全片段（31 条，Community 集）。
 *
 * 移植自 rech-editor-batch 0.0.26 的 snippets/bat-community.json（MIT，来源
 * 与升级指引见本目录 README.md）。body 为 VS Code snippet 语法（`$n` 占位），
 * 与 Monaco InsertAsSnippet 同源解析引擎，原文直用；三空格缩进是上游
 * Batch 编辑约定的原样保留。
 */

export interface BatchSnippet {
  /** 触发前缀 */
  prefix: string;
  /** 用途一句话（上游 description 原文） */
  description: string;
  /** VS Code snippet 语法的插入体 */
  body: string;
}

export const BATCH_SNIPPETS: readonly BatchSnippet[] = [
  {
    prefix: 'if',
    description: 'Conditional with else clause',
    body: 'if "%$1%"=="$2" (\n   $3\n) else (\n   $4\n)',
  },
  {
    prefix: 'ifc',
    description: "Check computer's name of the user who is executing the Batch",
    body: 'if /i "%COMPUTERNAME%"=="$1" (\n   $2\n)',
  },
  {
    prefix: 'ife',
    description: 'Check if a file exists',
    body: 'if exist "$1" (\n   $2\n) else (\n   $3\n)',
  },
  {
    prefix: 'ifeq',
    description: 'Check equality',
    body: 'if $1 EQU  (\n   $2\n)',
  },
  {
    prefix: 'ifi',
    description: 'Check equality ignoring case',
    body: 'if /i "%$1%"=="$2" (\n   $3\n) else (\n   $4\n)',
  },
  {
    prefix: 'ifin',
    description: 'Check negative equality ignoring case',
    body: 'if /i not "%$1%"=="$2" (\n   $3\n) else (\n   $4\n)',
  },
  {
    prefix: 'ifg',
    description: 'If greater than',
    body: 'if $1 GTR $2 (\n   $3\n)',
  },
  {
    prefix: 'ifge',
    description: 'If greater than or equal to',
    body: 'if $1 GEQ $2 (\n   $3\n)',
  },
  {
    prefix: 'ifl',
    description: 'If less than',
    body: 'if $1 LSS $2 (\n   $3\n)',
  },
  {
    prefix: 'iflq',
    description: 'If less than or equal to',
    body: 'if $1 LEQ $2 (\n   $3\n)',
  },
  {
    prefix: 'ifn',
    description: 'Check negative equality',
    body: 'if not "%$1%"=="$2" (\n   $3\n) else (\n   $4\n)',
  },
  {
    prefix: 'ifne',
    description: 'Check if a file does not exists',
    body: 'if not exist "$1" (\n   $2\n) else (\n   $3\n)',
  },
  {
    prefix: 'ifnq',
    description: 'Check inequality',
    body: 'if $1 NEQ $2 (\n   $3\n)',
  },
  {
    prefix: 'ifs',
    description: 'Simple conditional (without else clause)',
    body: 'if "%$1%"=="$2" (\n   $3\n)',
  },
  {
    // 上游原文使用 ${CURRENT_USERNAME}——非 VS Code/Monaco 标准片段变量
    // （标准是 TM_USERNAME），两侧都会展开为空串，原样保留不修正
    prefix: 'ifu',
    description: 'Check the username of the user who is executing the Batch',
    body: 'if /i "%USERNAME%"=="${CURRENT_USERNAME}" (\n   $1\n)',
  },
  {
    prefix: 'for',
    description: 'Simple loop',
    body: 'for %%i in ($1) do (\n   $2\n)',
  },
  {
    prefix: 'fora',
    description: 'Loop reading line by line from a file',
    body: 'for /f %%i in ($1) do (\n   $2\n)',
  },
  {
    prefix: 'forc',
    description: 'Iterates over a command output',
    body: "for /f %%i in ('$1') do (\n   $2\n)",
  },
  {
    prefix: 'ford',
    description: 'Loop through a comma separated list',
    body: 'for /d %%i in ($1,,) do (\n   $2\n)',
  },
  {
    prefix: 'fore',
    description: 'Loop reading line by line from a file with options',
    body: 'for /f "tokens=$1 delims=," %%i in () do (\n   $2\n)',
  },
  {
    prefix: 'fori',
    description: 'Incremental for loop',
    body: 'for /l %%i in (1,1,$1) do (\n   $2\n)',
  },
  {
    prefix: 'forl',
    description: 'Infinite loop',
    body: 'for /l %%i in (0,0,0) do (\n   $1\n)',
  },
  {
    prefix: 'forn',
    description:
      'Loop reading line by line from a file with options, ignoring the first n lines',
    body: 'for /f "skip=$1 tokens= delims=," %%i in () do (\n   $2\n)',
  },
  {
    prefix: 'foro',
    description: 'Loop for simulating a logical OR',
    body: 'for %%a in ("$1" "" "" "") do (\n   if /i "%1"==%%a (\n      $2\n   )\n)',
  },
  {
    prefix: 'forv',
    description: 'Set a variable with the content of a command output',
    body: "for /f %%i in ('$1') do set VARIABLE=%%i",
  },
  {
    prefix: 'goeof',
    description: 'Command goto to go out of a label (goto :eof)',
    body: 'goto :eof',
  },
  {
    prefix: 'delay',
    description: 'Enable the delayed environment variable expansion mode',
    body: 'setlocal EnableDelayedExpansion',
  },
  {
    prefix: 'enable',
    description: 'Enable the delayed environment variable expansion mode',
    body: 'setlocal EnableDelayedExpansion',
  },
  {
    prefix: 'replace',
    description: 'Change the content of a variable',
    body: '    set VAR=%VAR:$1=%',
  },
  {
    prefix: 'substring',
    description: 'Substring skipping (d) characters and extracting the next (n)',
    body: ':~d,n',
  },
  {
    prefix: 'substringd',
    description: 'Substring skipping (d) characters and extracting until the end',
    body: ':~d',
  },
];
