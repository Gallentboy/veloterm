/**
 * bash 保留字清单。
 *
 * 移植自 bash-language-server 5.7.0 的 reserved-words.ts（MIT，来源与升级
 * 指引见本目录 README.md）；上游按 GNU bash 手册 Reserved-Word-Index 整理。
 */

export const BASH_RESERVED_WORDS: readonly string[] = [
  '!',
  '[[',
  ']]',
  '{',
  '}',
  'case',
  'do',
  'done',
  'elif',
  'else',
  'esac',
  'fi',
  'for',
  'function',
  'if',
  'in',
  'select',
  'then',
  'time',
  'until',
  'while',
];
