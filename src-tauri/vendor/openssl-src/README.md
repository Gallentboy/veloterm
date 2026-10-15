# OpenSSL CLI 源码（锁死 3.6.5）

本目录是 OpenSSL **3.6.5** 源码的最小子集，供 `vendor/openssl-app/build.rs`
现场编译 `openssl` CLI（apps/openssl.c 的 main）并静态链接 vcpkg 安装的
libcrypto/libssl。版本锁定是**双源手维护**：本目录 `VELOTERM_VERSION`（源码
锁版本）+ `src-tauri/vcpkg.json` 的 overrides（CI 安装侧，manifest 模式按
官方版本控制精确装该版本，不跟随 runner 基线——不同世代镜像的 vcpkg 基线
会在版本间摇摆）。CI 校验步骤核实三方一致（实装头文件 / 本文件 / manifest）。

包含内容：
- `apps/`：CLI 全部源码 + `apps/include` 头 + `progs.pl`（命令表生成脚本）
- `include/internal/`：apps 引用的内部头（含 `e_os.h`）

不包含公共头（`include/openssl`）——编译时使用 vcpkg 已安装目录的
include（其中含 Configure 生成的配置头，版本一致）。

## 版本升级流程

1. CI 的 `Verify vcpkg OpenSSL version` 步骤失败（三方任一不一致）
2. 下载新版源码，替换本目录（apps + include/internal），**同步更新两处版本**：
   `VELOTERM_VERSION` 与 `src-tauri/vcpkg.json` 的 overrides
3. 推送重跑 CI：安装步骤按 manifest 的 override 精确装新版本，无需关心
   runner 当前基线是哪个版本

来源：https://github.com/openssl/openssl/releases/tag/openssl-3.6.5
（OpenSSL 许可证见上游 LICENSE；本目录随工程分发需保留该许可声明）
