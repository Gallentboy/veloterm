# OpenSSL CLI 源码（锁死 3.6.4）

本目录是 OpenSSL **3.6.4** 源码的最小子集，供 `vendor/openssl-app/build.rs`
现场编译 `openssl` CLI（apps/openssl.c 的 main）并静态链接 vcpkg 安装的
libcrypto/libssl。

版本锁定是**双源手维护**（升级时两处必须同步改）：

- 本目录 `VELOTERM_VERSION`（源码锁版本）
- `src-tauri/vcpkg.json` 的 overrides（CI 安装侧，manifest 模式按 vcpkg
  官方版本控制精确安装该版本）

CI 校验步骤核实三方一致（实装头文件 / 本文件 / manifest override）。

**锁版本必须是 vcpkg 上游版本数据库实有的版本**（`versions/o-/openssl.json`），
不是 OpenSSL 官方发布过就行——曾错锁 3.6.5：classic 模式跟随 runner ports
快照装出过它，但上游 versions 数据库从未收录，manifest 解析直接报
`no version database entry for openssl`。升级前先确认版本实有。

包含内容：
- `apps/`：CLI 全部源码 + `apps/include` 头 + `progs.pl`（命令表生成脚本）
- `include/internal/`：apps 引用的内部头（含 `e_os.h`）

不包含公共头（`include/openssl`）——编译时使用 vcpkg 已安装目录的
include（其中含 Configure 生成的配置头，版本一致）。

## 版本升级流程

1. 确认目标版本在 vcpkg 上游版本数据库实有（高于当前 3.6.4#1 时）
2. 下载新版源码，替换本目录（apps + include/internal），**同步更新两处版本**：
   `VELOTERM_VERSION` 与 `src-tauri/vcpkg.json` 的 overrides
3. 推送重跑 CI：安装步骤按 manifest override 精确装新版本；若 builtin-baseline
   SHA 在旧镜像不可达，workflow 已内置 `git fetch origin master` 补齐

来源：https://github.com/openssl/openssl/releases/tag/openssl-3.6.4
（OpenSSL 许可证见上游 LICENSE；本目录随工程分发需保留该许可声明）
