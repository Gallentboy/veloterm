# obclient-lib：libobclient 的预编译静态档案

obclnt-sys 的链接来源（build.rs 只发旗标不编译，gif2mp4 的 ffmpeg-lib
同型）。**目录按需填充**——档案不进库时（如 mac-x86_64 / win-arm64 尚未
产出）对应平台的构建会在校验步早失败并指引到本 README。

| 目录 | 档案 | 产物方式 |
| --- | --- | --- |
| `mac-aarch64/` | `libobclnt.a`（自包含：zlib + OpenSSL 已合并） | `test_obclient` 车道 macos-26 腿（`scripts/build-obclient.mjs` 的 mac 臂，OpenSSL 用 brew openssl@3 的静态 .a——keg 自带 libssl.a/libcrypto.a，是 CMake .a 合并分支的硬前提） |
| `mac-x86_64/` | 同上（Intel 腿） | 同上（macos-26-intel 腿，brew 前缀 /usr/local，脚本经 `brew --prefix openssl@3` 解析） |
| `win-x64/` | `obclnt.lib`（仅连接器对象，OpenSSL 不在内） | `test_obclient` 车道 windows-latest 腿：`src-tauri/obclient-build/libmariadb/Release/obclnt.lib`。**只取 obclnt.lib**——libobclnt.dll/libobclnt.lib 是 DLL 形态，静态链消费方不需要 |
| `win-arm64/` | 同上（ARM64 腿） | 同上（windows-11-arm 腿，vcpkg arm64-windows-static + cmake -A ARM64） |

Windows 消费方还需 vcpkg 静态 `libssl.lib`/`libcrypto.lib`（经 `OPENSSL_DIR`
环境变量提供，与 publish 的 app 构建同一约定）——obclnt.lib 未内置
OpenSSL（上游只在 Unix 的 .a 合并分支并入），车道冒烟已实证。

## OpenSSL 版本口径

两平台同代 3.x（ABI 兼容），补丁位不强求一致：win 侧 vcpkg manifest 锁
3.6.4#1；mac 侧 brew openssl@3 公式跟上游发布走（无 3.6.4 配方，当前
3.5.9）。mac 若需本地自出档案也可走上游官方路径（deps/3rd 的 1.1.1u
静态，仅 darwin.arm64 清单），但车道口径统一为 brew。
