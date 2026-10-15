# OpenSSL CLI 源码（锁死 3.6.5）

本目录是 OpenSSL **3.6.5** 源码的最小子集，供 `vendor/openssl-app/build.rs`
现场编译 `openssl` CLI（apps/openssl.c 的 main）并静态链接 vcpkg 安装的
libcrypto/libssl。版本与 CI 安装的 `openssl:x64-windows-static@3.6.5`
严格一致（`VELOTERM_VERSION` 文件是单一事实来源，CI 会校验漂移）。

包含内容：
- `apps/`：CLI 全部源码 + `apps/include` 头 + `progs.pl`（命令表生成脚本）
- `include/internal/`：apps 引用的内部头（含 `e_os.h`）

不包含公共头（`include/openssl`）——编译时使用 vcpkg 已安装目录的
include（其中含 Configure 生成的配置头，版本一致）。

## 版本升级流程

1. CI 的 `Verify vcpkg OpenSSL version` 步骤失败（提示版本不一致）
2. 下载新版源码，替换本目录（apps + include/internal），更新 `VELOTERM_VERSION`
3. 确认 CI vcpkg 实装版本同号后重新出包

来源：https://github.com/openssl/openssl/releases/tag/openssl-3.6.5
（OpenSSL 许可证见上游 LICENSE；本目录随工程分发需保留该许可声明）
