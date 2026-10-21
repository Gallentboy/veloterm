# Docker 语言数据（vendor + 整理）

Monaco Dockerfile / docker-compose 补全的数据资产，对应 `src/editor/dockerfileLanguage.ts`
与 `src/editor/composeLanguage.ts`（纯前端、纯离线，nginxData 同型约定）。

## 文件与来源

- `compose-spec.json`（74KB，draft-07 JSON Schema）：**原样 byte-copy**，取自
  [docker/docker-language-server](https://github.com/docker/docker-language-server)
  的 `internal/compose/compose-spec.json`。
  - 来源版本：commit `5187ff578db630f5df5b5922a10d8415a5bb7d32`（commit 日期
    2026-06-04，2026-10-21 克隆时的 master HEAD；最近 tag 0.20.1 / 2025-10-14，
    其后含未发版提交）
  - 该副本是其 Compose 补全引擎的单一数据源，**与上游
    [compose-spec/compose-spec](https://github.com/compose-spec/compose-spec) 的
    master schema 已分叉**（上游已迁 `$defs` 且新增 jobs/container_spec 等概念）。
    升级以 docker-language-server 的副本为准，不要直接拷 compose-spec master。
- `dockerfile.json`：**手工整理**（非上游拷贝）。docker-language-server 对 Dockerfile
  **没有补全能力**（其 completion 分发只覆盖 Compose 与 Bake），本文件按
  moby/buildkit `frontend/dockerfile/docs/reference.md`（即
  docs.docker.com/reference/dockerfile 的上游源，官方 Dockerfile 参考）整理：
  18 条指令（含 BuildKit flags：RUN --mount/--network/--security、COPY/ADD 的
  --from/--chmod/--chown/--link/--parents/--exclude/--checksum 等）、7 个预定义
  build ARG、5 个代理变量、2 条 parser 指令。文案为项目自写的中文摘要，
  每条指令留 docUrl 指向官方参考锚点。

## 许可证

- docker-language-server：**Apache-2.0**（compose-spec.json 随仓库同许可分发）。
- buildkit reference.md（dockerfile.json 的事实性内容来源）：Apache-2.0。
- 均为宽松许可，无 GPL 系商用确认负担（区别于 nginxData 的 GPL-3.0 先例）。

## 消费方

- `src/editor/composeLanguage.ts`：dynamic import 本目录 compose-spec.json
  （独立 async chunk，不打开 compose 文件的会话零加载成本）。
- `src/editor/dockerfileLanguage.ts`：dynamic import dockerfile.json（同上）。

## 升级流程

1. `compose-spec.json`：从 docker-language-server 新 commit 拷同名文件 byte-copy
   覆盖，更新本文件的 commit 记录；
2. `dockerfile.json`：按官方 Dockerfile 参考的新特性（新 flags / 新指令）手工修订；
3. 跑 `pnpm run typecheck` 与 `pnpm run build`；
4. 冒烟：docker-compose.yml → 顶层键/服务属性/枚举/依赖名补全；Dockerfile →
   指令/flags/stage 名/`$` 变量补全；普通 .yml 与 .gitlab-ci.yml 不出 compose 补全。
