/**
 * bash 补全片段（86 条，TM snippet 语法——Monaco InsertAsSnippet 同源解析，
 * 含 `${1|a,b|}` 选择占位与 `\$` 字面转义）。
 *
 * 移植自 bash-language-server 5.7.0 的 snippets.ts（MIT，来源与升级指引见
 * 本目录 README.md）。documentation 是片段用途一句话（上游原文）；展示用的
 * Markdown 文档（用途 + insertText 的 bash 代码块）由 bashLanguage.ts 在
 * 注册时一次性生成，对齐上游 SNIPPETS.map 的文档拼装。
 */

export interface BashSnippet {
  label: string;
  /** 片段用途一句话（上游 documentation 字段原文） */
  documentation: string;
  /** TM snippet 语法的插入体 */
  insertText: string;
}

export const BASH_SNIPPETS: readonly BashSnippet[] = [
  {
    label: 'shebang',
    documentation: 'shebang',
    insertText: '#!/usr/bin/env ${1|bash,sh|}',
  },
  {
    label: 'shebang-with-arguments',
    documentation: 'shebang-with-arguments',
    insertText: '#!/usr/bin/env ${1|-S,--split-string|} ${2|bash,sh|} ${3|argument ...|}',
  },
  {
    label: 'and',
    documentation: 'and operator',
    insertText: '${1:first-expression} && ${2:second-expression}',
  },
  {
    label: 'or',
    documentation: 'or operator',
    insertText: '${1:first-expression} || ${2:second-expression}',
  },
  {
    label: 'if',
    documentation: 'if operator',
    insertText: 'if ${1:condition}; then\n\t${2:command ...}\nfi',
  },
  {
    label: 'if-else',
    documentation: 'if-else operator',
    insertText: 'if ${1:condition}; then\n\t${2:command ...}\nelse\n\t${3:command ...}\nfi',
  },
  {
    label: 'if-less',
    documentation: 'if with number comparison',
    insertText: 'if (( "${1:first-expression}" < "${2:second-expression}" )); then\n\t${3:command ...}\nfi',
  },
  {
    label: 'if-greater',
    documentation: 'if with number comparison',
    insertText: 'if (( "${1:first-expression}" > "${2:second-expression}" )); then\n\t${3:command ...}\nfi',
  },
  {
    label: 'if-less-or-equal',
    documentation: 'if with number comparison',
    insertText: 'if (( "${1:first-expression}" <= "${2:second-expression}" )); then\n\t${3:command ...}\nfi',
  },
  {
    label: 'if-greater-or-equal',
    documentation: 'if with number comparison',
    insertText: 'if (( "${1:first-expression}" >= "${2:second-expression}" )); then\n\t${3:command ...}\nfi',
  },
  {
    label: 'if-equal',
    documentation: 'if with number comparison',
    insertText: 'if (( "${1:first-expression}" == "${2:second-expression}" )); then\n\t${3:command ...}\nfi',
  },
  {
    label: 'if-not-equal',
    documentation: 'if with number comparison',
    insertText: 'if (( "${1:first-expression}" != "${2:second-expression}" )); then\n\t${3:command ...}\nfi',
  },
  {
    label: 'if-string-equal',
    documentation: 'if with string comparison',
    insertText: 'if [[ "${1:first-expression}" == "${2:second-expression}" ]]; then\n\t${3:command ...}\nfi',
  },
  {
    label: 'if-string-not-equal',
    documentation: 'if with string comparison',
    insertText: 'if [[ "${1:first-expression}" != "${2:second-expression}" ]]; then\n\t${3:command ...}\nfi',
  },
  {
    label: 'if-string-empty',
    documentation: 'if with string comparison (has [z]ero length)',
    insertText: 'if [[ -z "${1:expression}" ]]; then\n\t${2:command ...}\nfi',
  },
  {
    label: 'if-string-not-empty',
    documentation: 'if with string comparison ([n]ot empty)',
    insertText: 'if [[ -n "${1:expression}" ]]; then\n\t${2:command ...}\nfi',
  },
  {
    label: 'if-defined',
    documentation: 'if with variable existence check',
    insertText: 'if [[ -n "\\${${1:variable}+x}" ]]; then\n\t${2:command ...}\nfi',
  },
  {
    label: 'if-not-defined',
    documentation: 'if with variable existence check',
    insertText: 'if [[ -z "\\${${1:variable}+x}" ]]; then\n\t${2:command ...}\nfi',
  },
  {
    label: 'while',
    documentation: 'while operator',
    insertText: 'while ${1:condition}; do\n\t${2:command ...}\ndone',
  },
  {
    label: 'while-else',
    documentation: 'while-else operator',
    insertText: 'while ${1:condition}; do\n\t${2:command ...}\nelse\n\t${3:command ...}\ndone',
  },
  {
    label: 'while-less',
    documentation: 'while with number comparison',
    insertText: 'while (( "${1:first-expression}" < "${2:second-expression}" )); do\n\t${3:command ...}\ndone',
  },
  {
    label: 'while-greater',
    documentation: 'while with number comparison',
    insertText: 'while (( "${1:first-expression}" > "${2:second-expression}" )); do\n\t${3:command ...}\ndone',
  },
  {
    label: 'while-less-or-equal',
    documentation: 'while with number comparison',
    insertText: 'while (( "${1:first-expression}" <= "${2:second-expression}" )); do\n\t${3:command ...}\ndone',
  },
  {
    label: 'while-greater-or-equal',
    documentation: 'while with number comparison',
    insertText: 'while (( "${1:first-expression}" >= "${2:second-expression}" )); do\n\t${3:command ...}\ndone',
  },
  {
    label: 'while-equal',
    documentation: 'while with number comparison',
    insertText: 'while (( "${1:first-expression}" == "${2:second-expression}" )); do\n\t${3:command ...}\ndone',
  },
  {
    label: 'while-not-equal',
    documentation: 'while with number comparison',
    insertText: 'while (( "${1:first-expression}" != "${2:second-expression}" )); do\n\t${3:command ...}\ndone',
  },
  {
    label: 'while-string-equal',
    documentation: 'while with string comparison',
    insertText: 'while [[ "${1:first-expression}" == "${2:second-expression}" ]]; do\n\t${3:command ...}\ndone',
  },
  {
    label: 'while-string-not-equal',
    documentation: 'while with string comparison',
    insertText: 'while [[ "${1:first-expression}" != "${2:second-expression}" ]]; do\n\t${3:command ...}\ndone',
  },
  {
    label: 'while-string-empty',
    documentation: 'while with string comparison (has [z]ero length)',
    insertText: 'while [[ -z "${1:expression}" ]]; do\n\t${2:command ...}\ndone',
  },
  {
    label: 'while-string-not-empty',
    documentation: 'while with string comparison ([n]ot empty)',
    insertText: 'while [[ -n "${1:expression}" ]]; do\n\t${2:command ...}\ndone',
  },
  {
    label: 'while-defined',
    documentation: 'while with variable existence check',
    insertText: 'while [[ -n "\\${${1:variable}+x}" ]]\n\t${2:command ...}\ndone',
  },
  {
    label: 'while-not-defined',
    documentation: 'while with variable existence check',
    insertText: 'while [[ -z "\\${${1:variable}+x}" ]]\n\t${2:command ...}\ndone',
  },
  {
    label: 'until',
    documentation: 'until operator',
    insertText: 'until ${1:condition}; do\n\t${2:command ...}\ndone',
  },
  {
    label: 'until-else',
    documentation: 'until-else operator',
    insertText: 'until ${1:condition}; do\n\t${2:command ...}\nelse\n\t${3:command ...}\ndone',
  },
  {
    label: 'until-less',
    documentation: 'until with number comparison',
    insertText: 'until (( "${1:first-expression}" < "${2:second-expression}" )); do\n\t${3:command ...}\ndone',
  },
  {
    label: 'until-greater',
    documentation: 'until with number comparison',
    insertText: 'until (( "${1:first-expression}" > "${2:second-expression}" )); do\n\t${3:command ...}\ndone',
  },
  {
    label: 'until-less-or-equal',
    documentation: 'until with number comparison',
    insertText: 'until (( "${1:first-expression}" <= "${2:second-expression}" )); do\n\t${3:command ...}\ndone',
  },
  {
    label: 'until-greater-or-equal',
    documentation: 'until with number comparison',
    insertText: 'until (( "${1:first-expression}" >= "${2:second-expression}" )); do\n\t${3:command ...}\ndone',
  },
  {
    label: 'until-equal',
    documentation: 'until with number comparison',
    insertText: 'until (( "${1:first-expression}" == "${2:second-expression}" )); do\n\t${3:command ...}\ndone',
  },
  {
    label: 'until-not-equal',
    documentation: 'until with number comparison',
    insertText: 'until (( "${1:first-expression}" != "${2:second-expression}" )); do\n\t${3:command ...}\ndone',
  },
  {
    label: 'until-string-equal',
    documentation: 'until with string comparison',
    insertText: 'until [[ "${1:first-expression}" == "${2:second-expression}" ]]; do\n\t${3:command ...}\ndone',
  },
  {
    label: 'until-string-not-equal',
    documentation: 'until with string comparison',
    insertText: 'until [[ "${1:first-expression}" != "${2:second-expression}" ]]; do\n\t${3:command ...}\ndone',
  },
  {
    label: 'until-string-empty',
    documentation: 'until with string comparison (has [z]ero length)',
    insertText: 'until [[ -z "${1:expression}" ]]; do\n\t${2:command ...}\ndone',
  },
  {
    label: 'until-string-not-empty',
    documentation: 'until with string comparison ([n]ot empty)',
    insertText: 'until [[ -n "${1:expression}" ]]; do\n\t${2:command ...}\ndone',
  },
  {
    label: 'until-defined',
    documentation: 'until with variable existence check',
    insertText: 'until [[ -n "\\${${1:variable}+x}" ]]\n\t${2:command ...}\ndone',
  },
  {
    label: 'until-not-defined',
    documentation: 'until with variable existence check',
    insertText: 'until [[ -z "\\${${1:variable}+x}" ]]\n\t${2:command ...}\ndone',
  },
  {
    label: 'for',
    documentation: 'for operator',
    insertText: 'for ${1:item} in ${2:expression}; do\n\t${3:command ...}\ndone',
  },
  {
    label: 'for-range',
    documentation: 'for with range',
    insertText: 'for ${1:item} in \\$(seq ${2:from} ${3:to}); do\n\t${4:command ...}\ndone',
  },
  {
    label: 'for-stepped-range',
    documentation: 'for with stepped range',
    insertText: 'for ${1:item} in \\$(seq ${2:from} ${3:step} ${4:to}); do\n\t${5:command ...}\ndone',
  },
  {
    label: 'for-files',
    documentation: 'for with files',
    insertText: 'for ${1:item} in *.${2:extension}; do\n\t${4:command ...}\ndone',
  },
  {
    label: 'case',
    documentation: 'case operator',
    insertText: 'case "${1:expression}" in\n\t${2:pattern})\n\t\t${3:command ...}\n\t\t;;\n*)\n\t\t${4:command ...}\n\t\t;;\nesac',
  },
  {
    label: 'function',
    documentation: 'function definition',
    insertText: '${1:name}() {\n\t${2:command ...}\n}',
  },
  {
    label: 'documentation',
    documentation: 'documentation definition',
    insertText:
      '# ${1:function_name} ${2:function_parameters}\n# ${3:function_description}\n#\n# Output:\n#   ${4:function_output}\n#\n# Return:\n# - ${5:0} when ${6:all parameters are correct}\n# - ${7:1} ${8:otherwise}',
  },
  {
    label: 'block',
    documentation: 'block',
    insertText: '{\n\t${1:command ...}\n}',
  },
  {
    label: 'block-redirected',
    documentation: 'block redirected',
    insertText: '{\n\t${1:command ...}\n} > ${2:file}',
  },
  {
    label: 'block-stderr-redirected',
    documentation: 'block stderr redirected',
    insertText: '{\n\t${1:command ...}\n} 2> ${2:file}',
  },
  {
    label: 'variable',
    documentation: 'variable',
    insertText: 'declare ${1:variable}=${2:value}',
  },
  {
    label: 'variable-index',
    documentation: 'variable index',
    insertText: '${1:variable}[${2:index}]=${3:value}',
  },
  {
    label: 'variable-append',
    documentation: 'variable append',
    insertText: '${1:variable}+=${2:value}',
  },
  {
    label: 'variable-prepend',
    documentation: 'variable-prepend',
    insertText: '${1:variable}=${2:value}\\$$1',
  },
  {
    label: 'if-unset-or-null',
    documentation: 'if unset or null',
    insertText: '"\\${${1:variable}:-${2:default}}"',
  },
  {
    label: 'if-unset',
    documentation: 'if unset',
    insertText: '"\\${${1:variable}-${2:default}}"',
  },
  {
    label: 'set-if-unset-or-null',
    documentation: 'set if unset or null',
    insertText: '"\\${${1:variable}:=${2:default}}"',
  },
  {
    label: 'set-if-unset',
    documentation: 'set if unset',
    insertText: '"\\${${1:variable}=${2:default}}"',
  },
  {
    label: 'error-if-unset-or-null',
    documentation: 'error if unset or null',
    insertText: '"\\${${1:variable}:?${2:error_message}}"',
  },
  {
    label: 'error-if-unset',
    documentation: 'error if unset',
    insertText: '"\\${${1:variable}?${2:error_message}}"',
  },
  {
    label: 'if-set-or-not-null',
    documentation: 'if set or not null',
    insertText: '"\\${${1:variable}:+${2:alternative}}"',
  },
  {
    label: 'if-set',
    documentation: 'if set',
    insertText: '"\\${${1:variable}+${2:alternative}}"',
  },
  {
    label: 'string-remove-leading',
    documentation: 'string shortest leading replacement',
    insertText: '"\\${${1:variable}#${2:pattern}}"',
  },
  {
    label: 'string-remove-trailing',
    documentation: 'string shortest trailing replacement',
    insertText: '"\\${${1:variable}%${2:pattern}}"',
  },
  {
    label: 'string-match',
    documentation: 'string filtering',
    insertText: "sed ${1|-E -n,--regexp-extended --quiet|} '/${2:pattern}/p'",
  },
  {
    label: 'string-replace',
    documentation: 'string replacement',
    insertText: "sed ${1|-E,--regexp-extended|} 's/${2:pattern}/${3:replacement}/'",
  },
  {
    label: 'string-replace-all',
    documentation: 'string replacement',
    insertText: "sed ${1|-E,--regexp-extended|} 's/${2:pattern}/${3:replacement}/g'",
  },
  {
    label: 'string-transliterate',
    documentation: 'string transliterate',
    insertText:
      "sed ${1|-E,--regexp-extended|} 'y/${2:source-characters}/${3:replacement-characters}/g'",
  },
  {
    label: 'file-print',
    documentation: 'file print',
    insertText: "sed '' ${1:file}",
  },
  {
    label: 'file-read',
    documentation: 'file read',
    insertText:
      "sed ${1|-E,--regexp-extended|} ':${2:x} N $! b$2 ${3:command}' ${4:file}",
  },
  {
    label: 'skip-first',
    documentation: 'skip first',
    insertText: 'tail ${1|-n,-c,--lines,--bytes|} +${2:count}',
  },
  {
    label: 'skip-last',
    documentation: 'skip last',
    insertText: 'head ${1|-n,-c,--lines,--bytes|} -${2:count}',
  },
  {
    label: 'take-first',
    documentation: 'take first',
    insertText: 'head ${1|-n,-c,--lines,--bytes|} ${2:count}',
  },
  {
    label: 'take-last',
    documentation: 'take last',
    insertText: 'tail ${1|-n,-c,--lines,--bytes|} ${2:count}',
  },
  {
    label: 'take-range',
    documentation: 'take range',
    insertText: "sed ${1|-n,--quiet|} '${2:from},${3:to}p'",
  },
  {
    label: 'take-stepped-range',
    documentation: 'take stepped range',
    insertText: "sed ${1|-n,--quiet|} '${2:from},${3:to}p' | sed $1 '1~${4:step}p'",
  },
  {
    label: 'json-print',
    documentation: 'json print',
    insertText: "jq '.${1:node}' ${2:file}",
  },
  {
    label: 'device',
    documentation: 'device',
    insertText: '/dev/${1|null,stdin,stdout,stderr|}',
  },
  {
    label: 'completion definition',
    documentation: 'completion',
    insertText:
      '_$1_completions()\n{\n\treadarray -t COMPREPLY < <(compgen -W "-h --help -v --version" "\\${COMP_WORDS[1]}")\n}\n\ncomplete -F _$1_completions ${1:command}',
  },
  {
    label: 'comment definition',
    documentation: 'comment',
    insertText: '# ${1:description}',
  },
];
