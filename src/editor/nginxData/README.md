# nginx 语言数据（vendor）

Monaco `nginx` 语言补全/hover 的数据资产，原样取自
[pappasam/nginx-language-server](https://github.com/pappasam/nginx-language-server)：

- 来源版本：**v0.9.0**（commit `b6c31a5`，2025-04-18）
- 文件与用途：
  - `directives.json`（794 条指令：name / syntax[] / def / contexts[] / desc / notes[] / since / module）
  - `variables.json`（174 条 `$` 变量：name / desc / module）
- 上游的 `directives_document.json`、`variables_document.json`、`snippets.json`
  未被其服务端代码引用（死资产），未随入。

## 许可证（重要）

上游仓库整体为 **GPL-3.0-only**，数据文件随仓库同许可分发。同 libjxl（GPL-3.0）
与 nmap（NPSL）的先例：**商用发布前需项目方确认**。

## 消费方

`src/editor/nginxLanguage.ts`（懒加载 dynamic import，独立 async chunk，
不打开 nginx 文件的会话零加载成本）。JSON 无法内嵌注释，来源与许可信息以
本文件为唯一记录处。

## 升级流程

1. 从上游新版本拷贝同名 JSON 覆盖本目录（原样 byte-copy，不手工修订）；
2. 更新本文件的版本号与 commit；
3. 跑 `pnpm run typecheck` 与 `pnpm run build`；
4. 冒烟：nginx.conf 打开 → 补全/hover 正常（上下文感知见 nginxLanguage.ts）。
