# systemd 语言数据（手工整理）

Monaco `systemd` 语言（`.service`/`.socket`/`.timer` 等单元文件）补全与 hover 的
数据资产 `directives.json`。**非 vendor 原样拷贝，而是按 systemd 官方 man 手册
手工整理的事实性清单**（指令名与取值域是接口事实，说明文字为自写英文表述，
无版权负担）。

- 来源（整理口径，逐节对照）：
  - `systemd.unit(5)`：[Unit] 元数据/依赖/排序、Condition*/Assert* 条件与断言、
    [Install] 启用接线；
  - `systemd.service(5)` + 共享的 `systemd.exec(5)`（WorkingDirectory/沙箱/
    资源限额/日志/环境）、`systemd.kill(5)`（KillMode/KillSignal 等）、
    `systemd.resource-control(5)`（CPU/Memory/Tasks/IO cgroup 控制）：[Service]
    与 [Slice]/[Scope] 的指令；
  - `systemd.socket(5)`、`systemd.timer(5)`、`systemd.mount(5)`、
    `systemd.automount(5)`、`systemd.path(5)`、`systemd.swap(5)`：各专属节；
  - [Target]/[Device] 无专属节指令（sections 里保留说明，directives 为空对象）。
- 文件与结构：
  - `sections`：13 个节名 → 一句话说明（节标题补全与 hover 用）；
  - `directives`：`{ 节名: { 指令名: 一句话说明 } }` 双层结构（节内指令补全
    按当前 `[Section]` 过滤；hover 显示说明 + 所属节）；
  - `values`：`{ 节名: { 指令名: { 值: 说明 } } }` 已知枚举值（Type=/Restart=/
    KillMode=/StandardInput|Output|Error= 等），供 `=` 后的值补全与值 hover。
    按节嵌套是因为同名指令语义不同（如 `Type=` 在 [Service] 是进程类型枚举、
    在 [Mount] 是文件系统类型字符串，后者无枚举）。
- 覆盖规模：13 节 / 330+ 指令条目 / 20 组枚举值（核心指令全集，省略了
  credential/keyring 细分等冷门指令）。

## 升级流程

1. 按新版本 man 手册补订指令与枚举（手工整理资产，无上游 byte-copy 可跑）；
2. 核对 `sections` 键集合仍为 13 个（`systemdLanguage.ts` 的节标题补全依赖）；
3. 跑 `pnpm run typecheck` 与 `pnpm run build`；
4. 冒烟：打开任意 `.service` 文件 → 节标题/指令/枚举值补全与 hover 正常。

## 消费方

`src/editor/systemdLanguage.ts`（懒加载 dynamic import，独立 async chunk，
不打开 systemd 单元文件的会话零加载成本）。
