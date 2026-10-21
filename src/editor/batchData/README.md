# batchData —— Windows Batch 补全静态数据

`batchLanguage.ts`（Monaco `batch` 自建语言）的静态数据来源：

| 文件 | 内容 | 上游源文件 |
| --- | --- | --- |
| `snippets.ts` | 补全片段 31 条（Community 集，`$n` 占位 VS Code snippet 语法） | `snippets/bat-community.json` |
| `keywords.ts` | Windows 系统命令 316 条 + cmd 内建控制流 21 条 | `syntaxes/batchfile.tmLanguage.json` 的 commands/controls 规则正则 |

## 来源与许可

- 移植自 [rech-editor-batch](https://github.com/RechInformatica/rech-editor-batch)
  v0.0.26，commit `fcce16377c726f95d5afd25d91a05cd29208e2a7`（2026-02-11）。
- 上游许可证 **MIT**（Copyright 2019 rechinformatica）——可商用。
- 语义对应关系：片段 = 上游 `SnippetsGenerator` 注册的补全集（上游唯一的
  补全来源）；命令/内建关键字 = 上游 TextMate 语法的着色词表（上游**不**用
  于补全——作为补全候选是有意增强，见 batchLanguage.ts 文件头）。

## 移植口径

- 片段取 **Community 集**（英文描述，上游 package.json 的默认配置）；
  Rech Internal 集（`bat.json` 37 条，葡语描述、含公司内部约定）不收录。
- 命令表保真保留上游笔误词条（brea/hep/defra/fonde/doin/ntackup/regentc/
  tacerpt/ysocmgr/eraseesentutl/servrmanagercmd/clscluadmin/ldifd/verif/rxec/
  mic 等）与多词条目（net computer 等）：与上游 diff 零噪音，升级时按上游
  新正则重放即可；高亮无害，补全端由 Monaco 前缀过滤自然吞掉。
- `ifu` 片段原文使用 `${CURRENT_USERNAME}`——非 VS Code/Monaco 标准片段
  变量（标准是 TM_USERNAME），两侧都展开为空串，原样保留不修正。
- 3-space 缩进与 `%$1%` 形态均上游原文。

## 已知边界（不属于本数据层）

- 上游默认 bat 文件编码 **windows-1252**（package.json configurationDefaults）：
  veloterm 按 UTF-8 读远端文件，CP1252 编码的 bat 中重音字符会 mojibake——
  属文件加载层的既有行为，不在语言特性范畴。
- TabStopper 列停靠（Tab 跳停靠列）、Alt+N 插入注释行：VS Code 交互约定，
  未移植（见 batchLanguage.ts 文件头的「不移植」清单）。

## 升级指引

1. 克隆新版 rech-editor-batch，diff `snippets/bat-community.json` 与
   `syntaxes/batchfile.tmLanguage.json`（commands/controls 规则）。
2. 同步数据后更新本文件的版本与 commit 记录（命令表重放时上游笔误词条
   若已修正，随之修正并在此说明）。
3. `batchLanguage.ts` 里的 Monarch 高亮是语法文件的人工移植（中等规模子集），
   升级时按新版 grammar 复核覆盖面。
