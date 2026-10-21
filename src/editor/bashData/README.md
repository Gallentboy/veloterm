# bashData —— bash 补全静态数据

`bashLanguage.ts`（Monaco `shell` 补全 provider）的静态词表来源，共三份：

| 文件 | 内容 | 上游源文件 |
| --- | --- | --- |
| `reservedWords.ts` | bash 保留字 21 条 | `reserved-words.ts`（GNU bash 手册 Reserved-Word-Index） |
| `builtins.ts` | bash 内建命令 61 条 | `builtins.ts`（由 `compgen -b` 生成） |
| `snippets.ts` | 补全片段 86 条（TM snippet 语法） | `snippets.ts` |

## 来源与许可

- 移植自 [bash-language-server](https://github.com/bash-lsp/bash-language-server)
  v5.7.0（npm 包 `bash-language-server@5.7.0`），commit
  `3218a314d333b96f00cbe28e073a75425083fcbd`（2026-03-06）。
- 上游许可证 **MIT**（Copyright Mads Hartmann 等，随包 LICENSE）——可商用，
  无 nmap/libjxl 式许可确认负担；移植时保留字段原文、未改动数据内容。
- 语义对应关系：静态词表 = 上游 `server.ts::onCompletion` 候选并集中的
  ReservedWords / Builtins / SNIPPETS 三源；其余三源（文档符号 / PATH 可执行 /
  命令选项）在 veloterm 的实现形态见 `bashLanguage.ts` 文件头注释
  （tree-sitter 下沉 Rust 后端 + 远端 compgen，Node/fs 子进程部分不移植）。

## 升级指引

1. 克隆新版 bash-language-server，比对 `server/src/{reserved-words,builtins,snippets}.ts`
   与本目录三文件（数据为纯字面量，diff 即清单）。
2. 同步数据后更新本文件的版本与 commit 记录。
3. `snippets.ts` 的展示文档（Markdown 拼装）在 `bashLanguage.ts` 注册时生成，
   数据文件只存原始字段，无双重维护点。
