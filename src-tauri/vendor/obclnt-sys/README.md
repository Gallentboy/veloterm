# obclnt-sys

libobclient（OceanBase Connector/C）的原始 C API 绑定（sys crate，
gif2mp4-sys 同型：**不编译任何 C 代码**，只做预编译静态档案的校验与
链接旗标；extern 声明手写，本仓库无 bindgen 先例）。

- 库源码树：`../obconnector-c`（版本单一来源 `rpm/libobclient-VER.txt`，
  当前 2.2.14；许可证 LGPL-2.1，商用发布前需项目方确认）
- 静态档案：`../obclient-lib/<平台>/`（布局与产物方式见该目录 README）
- 安全层（spawn_blocking 包装、行流、错误转 String）在 app 侧 db_browser
  引擎臂，不在本 crate——sys 层只有类型镜像 + 常量 + extern 声明

## 声明口径

`src/lib.rs` 的 extern 逐条对照 `../obconnector-c/include/mysql.h` 抄写：
调用约定统一 `extern "C"`（上游 STDCALL 在 x64/ARM64 MSVC 是单调用约定
别名）；`my_bool`=char、`my_ulonglong`=unsigned long long；C enum 统一
c_uint。**覆盖面是引擎臂所需子集**，非全量镜像——需要新符号时按头文件
增量补，并在模块头的覆盖面注记里同步。

`MysqlField` 是 mysql.h:122-160 的完整 repr(C) 镜像（含 OB 的 owner/elem
族扩展字段）——**不得删减字段**（布局必须与 C 严格一致），上游加字段时
同步镜像。

## 链接（build.rs）

| 平台 | 档案 | OpenSSL | 系统库 |
| --- | --- | --- | --- |
| macOS（双架构） | `libobclnt.a`（自包含：zlib + OpenSSL 已合并，四腿车道产出） | 合并在内（brew openssl@3 的静态 .a，3.x 线） | 无需 |
| Windows（双架构） | `obclnt.lib`（仅连接器对象） | 需 `OPENSSL_DIR` 指向 vcpkg `*-windows-static`（补链静态 libssl/libcrypto，与 publish 的 app 构建同一约定） | ws2_32/advapi32/crypt32/shlwapi/user32/bcrypt（bcrypt 来自静态插件 caching_sha2/ob_sm3 的 CNG 引用） |

取档位：`OBCLIENT_LIB_DIR`（整目录覆盖）→ `../obclient-lib/<平台>/`。

CRT 注记：上游把 MSVC 默认 /MD 全量替换为 /MT（与 vcpkg static triplet
一致），链入 /MD 宿主会有 LNK4098 双 CRT 混链警告——与本仓库既有实践
（libsqlite3-sys 的 sqlcipher + vcpkg 静态 OpenSSL）同型，先容忍；根治
（宿主 crt-static 或上游回 /MD）另议。

## 升级

1. 换 `../obconnector-c` 树（重放两处 INIT_ONCE 补丁，见其
   veloterm-win/README.md）→ 车道出 Windows 档案、build.sh 出 mac 档案
   → 更新 `../obclient-lib/` 四目录；
2. **对照新头文件复核 `src/lib.rs`**：函数签名逐条 diff、`MysqlField`
   字段序、枚举值（mysql_option / field_type / CLIENT_*）——上游加枚举
   值会移位后续序号；
3. 本 crate 无其他状态（无生成物、无依赖）。
