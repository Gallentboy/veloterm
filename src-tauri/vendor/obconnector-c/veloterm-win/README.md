# obconnector-c（OceanBase Connector/C，libobclient）

OceanBase 的 C 客户端驱动，MariaDB Connector/C 3.1.x 的 fork（README.md 自述
"ported from MariaDB Connector/C with some OceanBase protocol support"）。
veloterm 的数据库工作台引入它是为 **OB Oracle 模式租户**（MySQL 模式租户说
标准 MySQL 线缆协议，mysql_async 直连即可，不经本库）——Oracle 模式的握手
能力位（CLIENT_SUPPORT_ORACLE_MODE = 1<<27）、OB2.0 协议 extra 头 + CRC、
SM3 认证、NUMBER/DATE 格式模型、LOB locator、TNS 名解析全部是 OB 私有协议
层，标准 MySQL 驱动无法替代。

## 源码与版本

- 上游：https://github.com/oceanbase/obconnector-c （master 分支整树）
- 版本：`rpm/libobclient-VER.txt`（= 2.2.14）是单一版本源——根 CMakeLists
  以 `file(STRINGS)` 读它拼 CPACK_PACKAGE_VERSION / mariadb_version.h，
  故本树受 .gitattributes 整树 `text eol=lf` 保护（CRLF 会把 \r 拼进版本串）
- 许可证：**LGPL-2.1**（COPYING.LIB）。随包/静态链入商用发布前需项目方
  确认（libjxl GPL / nmap NPSL 同清单）；动态链形态合规负担更轻

## 构建形态（app 侧，不进 coreutils 工具链）

- **Windows（本目录所在的段）**：MSVC + CMake（VS 多配置生成器，仅
  Release），OpenSSL 用 vcpkg 静态库（manifest 模式，与 publish.yml 同源，
  版本锁 `src-tauri/vcpkg.json` 的 overrides 3.6.4#1）。入口：**独立脚本
  `scripts/build-obclient.mjs`**（不经 build-tools.mjs——其尾段 nmap 无条件
  构建，与 obclient 无关）；车道 `.github/workflows/test-obclient.yml`
  （tag `test_obclient` 触发）。关键 CMake 旗标（上游树全外参）：
  - `-DCMAKE_POLICY_VERSION_MINIMUM=3.5`：CMake 4 移除 <3.5 兼容，上游
    非 APPLE 分支声明 `cmake_minimum_required(2.8)` 会硬报错，这是官方
    逃生口（对 CMake 3 是无害的多余缓存变量）
  - `-DWITH_SSL=OPENSSL` + 三件套显式路径（OPENSSL_INCLUDE_DIR /
    OPENSSL_SSL_LIBRARY / OPENSSL_CRYPTO_LIBRARY，上游 build.sh 同型，
    绕开 FindOpenSSL 探测不确定性）；不用 WIN32 默认的 SCHANNEL——与
    macOS 腿同走 OpenSSL 路径，行为一致
  - `-DWITH_CURL=OFF`：remote_io 插件整体跳过（plugins/io 的
    `IF(WITH_CURL)` 门），不给产物引入 curl 依赖
- **macOS**：上游官方路径可用——`deps/3rd/dep_create.sh`（按
  `libobclient.darwin.arm64.deps` 清单从阿里云镜像拉 OpenSSL 1.1.1u 静态
  库）+ `build.sh`，已在 Apple Silicon 实测编通（libobclnt.dylib /
  libobclnt.a，.a 已合并 ssl+crypto）。mac 的 vcpkg 等价物未做

### 产物

| 产物 | 路径（构建树） | 内容 |
| --- | --- | --- |
| `libobclnt.dll` + 导入库 `libobclnt.lib` | `libmariadb/Release/` | DLL 已静态链入 vcpkg OpenSSL，运行期零额外 DLL |
| `obclnt.lib` | `libmariadb/Release/` | 静态库仅连接器对象（zlib/win-iconv 在内，**OpenSSL 不在内**——Unix 才走 .a 合并分支），消费方需自带 ssl/crypto + ws2_32/advapi32/crypt32/shlwapi |

veloterm-win/ 冒烟工程（本目录 CMakeLists.txt + smoke.c）静态链 obclnt.lib
+ vcpkg ssl/crypto 跑 `mysql_library_init → mysql_init → mysql_close`
生命周期（WIN32 初始化路径含 WSAStartup），验证产物可链可跑。

## 尚未接线

app 侧暂无 sys-crate / FFI 消费方，本段只在车道验证（产物不拷
`src-tauri/binaries/`）。接线时：加 `vendor/obclient-sys`（发链接旗标）+
`db_browser` 引擎第三臂。

## 本地修补（两处源码 + 一处构建旗标，升级时重放）

上游从未发布过 Windows 产物（GitHub releases 只有 Linux RPM + macOS 包），
MSVC 编译有三处坑：

1. `libmariadb/ob_protocol20.c`：静态 mutex + init_flag 双检锁 → Windows 臂
   改 `INIT_ONCE`（`INIT_ONCE_STATIC_INIT` 静态可构造，语义等价），POSIX 臂
   原样保留（`#ifdef _WIN32` 分枝）；受回调引用的数组声明前置
2. `libmariadb/ob_full_link_trace.c`：同型同改
3. **构建旗标**（非源码修补）：MSVC 的 C `inline`（ma_global.h 在 _MSC_VER
   下映射为 `__inline`）不发射外部符号，`ob_serialize.c` 的 40 个
   `encode_*/decode_*` inline 定义被其他编译单元引用即 LNK2019（CI 实测
   `encode_i64`/`decode_i64`，ob_object.c 等还有同类隐患）。解法走上游
   ma_global.h 自带的逃生开关 `-DHAVE_BROKEN_INLINE`（`#undef inline` /
   `#define inline` 空），经 env `CFLAGS` 注入（见
   `scripts/build-obclient.mjs` 的配置调用注释）

其余源码零改动（OB 增量代码的 `_WIN32` 守卫齐全，实测仅上述遗漏）。

## 升级

换整树（对照上游 release，`rpm/libobclient-VER.txt` 取新版本）→ 全树扫描
确认零 CRLF → **重放上述两处 INIT_ONCE 补丁**（构建旗标在脚本里无需重放）
→ 重跑车道。veloterm 侧资产：本目录（veloterm-win/）、
`scripts/build-obclient.mjs`、外层 .gitattributes 规则。
