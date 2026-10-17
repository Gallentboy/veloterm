# vendor/nmap（nmap / ncat / nping 三件套）

上游：https://github.com/nmap/nmap （nmap.org；上游自述见 `README.upstream.md`）
版本：**7.992**（锁在 `VELOTERM_VERSION`；2026-10-12 发布的最新稳定版，
GitHub tag v7.992）
来源：官方 dist 源码包 `https://nmap.org/dist/nmap-7.992.tar.bz2`
（sha256 `0c12660eddbf7eabeff6996cb0c044a4eadf2bb0c6fbcb340d6cf72b1caee402`），
整目录解压收入。**已删减 `zenmap/`**（Python GUI 前端，按「只嵌入非 GUI
部分」的要求裁掉）。

**本工程构建其中三个命令**（同属 Nmap 项目、共享 nbase/nsock/OpenSSL 依赖，
由 `veloterm-win/CMakeLists.txt` 一次 CMake 构建出三个 DLL）：

| 命令 | DLL | 说明 |
| --- | --- | --- |
| `nmap` | `nmapffi.dll` | 网络扫描器（本体） |
| `ncat` | `ncatffi.dll` | netcat 替代（TCP/UDP/SSL 连接、代理、监听） |
| `nping` | `npingffi.dll` | 报文生成/探测（类 hping） |

`ndiff`（纯 Python 的 XML 比对工具）未集成——它需要 Python 运行时，本项目
工具链不绑 Python；`nmap` 已能输出 XML，用户可自行 diff。

## 为什么是 MSVC + CMake 自建工程（官方 Windows 路径）

上游官方 Windows 构建就是 **MSVC**（`mswin32/Build.bat` + `nmap.sln`），它链
Npcap SDK 的 `wpcap.lib`（`pcap_*`）+ `packet.lib`（`Packet*`），并以 MSVC 原生
`DelayLoadDLLs=packet.dll;wpcap.dll` 让二者延迟加载——没装 Npcap 的机器上仍能
启动，运行期由 `mswin32/winfix.cc` 的 SEH 探测降级 connect 扫描。本工程**照搬
官方这条路径**，故：

- **不编译 bundled libpcap**（pcap 符号由 Npcap SDK 提供），MinGW 路线下踩过的
  flex/bison 缺失、asprintf 隐式声明、man 页缺失、pcap 运行期探测、静态库依赖
  顺序等全部问题不存在；
- **winfix.cc 用上游原版**（该文件本就是 MSVC 专用：`#include <nmap_winconfig.h>`
  + `_MSC_VER` 门控的 SEH/delayimp 降级），无本地补丁；
- 配置头（`nmap_winconfig.h` / `nbase/nbase_winconfig.h` / `nsock/include/
  nsock_winconfig.h`）都在树内，无需 autoconf。

官方 sln 唯一的问题是**只有 Win32(x86) 平台**（`Release|x64` 映射回
`Release|Win32`），产物是 32 位 exe——无法载入 64 位宿主 coreutils.exe。故以
`veloterm-win/CMakeLists.txt`（本仓库新增）复刻官方 vcxproj 的源清单/宏/include
路径/库清单，用 `-A x64` 出 **x64 DLL**，C 入口 `veloterm_nmap_main` 由
`veloterm_ffi.cc` 提供（分发臂在 `crates/coreutils/src/nmap_impl.rs`）。

## 构建形态（`scripts/build-tools.mjs --only nmap`，仅 Windows）

在 **windows-latest 的 MSVC + CMake** 环境（不需要 MSYS2/MinGW），依赖只有
Npcap SDK 与 OpenSSL 静态库：

1. **Npcap SDK**：下载 `https://npcap.com/dist/npcap-sdk-1.13.zip`（与上游
   `mswin32/Build.bat` 同版），解压得 `Include/` + `Lib/x64/`。两份导入库
   （`wpcap.lib` / `Packet.lib`）直接进最终链接；libdnet 的 `FindPacket`
   也以 `-DPacket_ROOT=<SDK>` 指向它。
2. **OpenSSL 静态库**：复用 CI 的 vcpkg `x64-windows-static`（与 SQLCipher /
   openssl-app 同源，`OPENSSL_DIR` 环境变量；二进制缓存已预热）。
3. **CMake 配置 + 构建**：`cmake -S vendor/nmap/veloterm-win -B <build> -A x64
   -DNPCAP_SDK_DIR=<SDK> -DOPENSSL_DIR=<OPENSSL_DIR>`，`cmake --build
   --config Release`。工程内以 `add_subdirectory` 消费 libpcre(PCRE2 8-bit
   静态) / libz(静态) / libdnet-stripped(静态，Packet 来自 SDK) 的官方
   CMakeLists；nbase / nsock / libnetutil / liblua / liblinear 直接编源
   （清单照搬官方各 vcxproj）。**一次构建产出三个 DLL**：`nmapffi.dll`、
   `ncatffi.dll`、`npingffi.dll`（均 `/DELAYLOAD:packet.dll;wpcap.dll`、
   静态 CRT `/MT` 对齐 vcpkg triplet）。ncat 复用 nbase/nsock/liblua/
   wpcap/OpenSSL，nping 复用 nbase/nsock/libnetutil/dnet/wpcap+packet/OpenSSL。
4. **runner exe + 数据 staging + 冒烟**：同工程另出三个 runner
   （`nmap-runner`/`ncat-runner`/`nping-runner`，各用上游入口文件原样，
   构建期工具、不随包分发）——`nmap-runner` 跑 `--script-updatedb` 生成
   `scripts/script.db` 并把数据文件收进 `nmap-data/`（清单见下）；三个 runner
   各跑 `--version` 与基础功能冒烟（nmap `-sT 127.0.0.1 -p 1`、ncat `-z`、
   nping TCP 探测；CI 无 Npcap，正好回归 `/DELAYLOAD` 降级路径）。
5. 产物复制到 `src-tauri/binaries/`：三个 DLL + `nmap-data/` + `ca-bundle.crt`
   （ncat 的 SSL 客户端证书束，它在 exe 同目录查找；DLL 形态下
   `GetModuleHandle(0)` 取到宿主 coreutils.exe 路径，故放安装根即可命中），
   经 `tauri.windows.conf.json` 的 resources 放安装根目录。

数据文件清单（`nmap-data/`，取上游 `Makefile.in` 的 `install-nmap`/`install-nse`
目标）：`nmap-services`、`nmap-os-db`、`nmap-service-probes`、`nmap-protocols`、
`nmap-rpc`、`nmap-mac-prefixes`、`nse_main.lua`、`nmap.xsl`、`nmap.dtd`、
`scripts/`、`nselib/`。（旧版的 `nmap-payloads` 在 7.992 已不存在——payload
数据内置在 `payload.cc`。）

## 运行时行为

- coreutils.exe 的三个分发臂（硬链接短名 `tools\nmap.exe` / `tools\ncat.exe` /
  `tools\nping.exe` 或子命令模式）各按 exe 同目录 → 上级目录搜索对应 DLL
  （与 yqffi/qalcffi 同一套约定）。
- **nmap** 调用前置 `NMAPDIR`（若用户未设）指向安装根的 `nmap-data/`
  ——上游在 Windows 下默认找 **exe 同目录** 的数据文件，而 DLL 形态下
  `GetModuleFileName(GetModuleHandle(0))` 取到的是宿主 coreutils.exe 路径
  （tools\ 子目录），必须经 NMAPDIR 兜底；用户显式 `--datadir`/`NMAPDIR`
  优先级仍在上游搜索序的前面，不受影响。
- **ncat** 的 `ca-bundle.crt` 同样在 exe 同目录查找（缺失仅 debug 告警、
  不致命，但 SSL 客户端证书验证会失效）——随包放安装根即命中。
- **Npcap 依赖是运行期的**：未装 Npcap 时三命令仍可用（nmap `--version`、
  `-sT` connect 扫描、服务/版本探测；ncat 的 TCP/UDP/SSL 连接与监听不依赖
  原始包；nping 退化为可用模式），SYN/UDP/OS 指纹等原始包能力提示安装
  Npcap（https://npcap.com）；装了 Npcap（含 WinPcap 兼容模式）则全功能。
  三者的进度输出直接写继承的 ConPTY stdio，无 TUI，进程内直跑安全。
- **三个命令的 main 内均有 exit() 调用**（ncat 22 处、nping 16 处、nmap 10 处）
  ——coreutils 是「单命令生命周期进程」，exit() 等价于返回退出码（宿主随之
  结束），无需像 eza/zoxide 那样改造为返回值。
- NSE 用户级脚本目录 `%APPDATA%\nmap/scripts`、`--datadir` 覆盖等上游
  语义原样保留。安装目录（Program Files）通常只读，用户跑
  `--script-updatedb` 会因无法回写 script.db 报错——script.db 已在构建期
  预生成随包分发，正常使用不需要重算。

## 本地补丁（升级时需逐条复核重放）

升级 = 换 vendor 源码 + 改 `VELOTERM_VERSION` + 重放下列补丁 + 复核
`veloterm-win/CMakeLists.txt` 的源清单/宏/库清单仍然成立：

1. `veloterm_ffi.cc`（本仓库新增文件）：`#define main` 改名包含 `main.cc`
   + 导出 `veloterm_nmap_main`（DLL 入口）。
2. `veloterm-win/CMakeLists.txt`（本仓库新增文件）：MSVC x64 自建工程，
   源清单/宏/include/库清单复刻官方 `mswin32/nmap.vcxproj` 与各子项目
   vcxproj；不编译 bundled libpcap（pcap 符号由 Npcap SDK 提供）。
3. `nmap_winconfig.h`（搜 `VeloTerm patch`，两处）：
   - `NMAP_PLATFORM` 按实际架构分（`_M_X64` → x86_64，否则保持上游 i686）
     ——仅影响 `--version` 展示与 OS 指纹匹配字符串；
   - `HAVE_LIBSSH2` / `LIBSSH2_INCLUDED` 注释掉（不集成 libssh2，其需单独
     CMake 构建 + OpenSSL 集成；NSE 的 ssh 脚本报 unavailable，其余不受影响）。
4. `nse_fs.cc`（搜 `VeloTerm patch`）：`dir_data.hFile` 由 `long` 改
   `intptr_t`（并显式引 `<stdint.h>`，`_findfirst` 返回值比较同步改）。
   **x64 移植必需**：MSVC 的 `_findfirst()` 在 x64 返回 `intptr_t`（64 位
   句柄），存进 `long`（LLP64 下 32 位）会截断，随后 `_findnext` 用无效句柄
   → 访问违例 0xC0000005（`nmap --script-updatedb` 遍历 scripts/ 时崩，
   CI 实测）。官方 Windows 构建只有 Win32 平台，故上游未暴露此问题。
5. **删除上游 `.gitignore`**：nmap 的 git 仓忽略规则面向他们自己的开发仓
   （那里 `/libpcap/bpf_filter.c`、`/libz/zconf.h`、`/libpcap/*.3pcap`、
   `/libpcap/grammar.y` 等是生成物/不入库文件），而 dist tarball 实际携带
   这些文件——vendor 入库时被静默吞掉 43 个。删除该文件并从 tarball 恢复
   全部被吞文件入库（保持源码树完整；MSVC 路线下这些文件不参与构建）。
6. 删减：`zenmap/` 整目录（GUI，不构建不分发）；上游 `README.md` 挪为
   `README.upstream.md` 给本文件让位。

`mswin32/winfix.cc` 与 `libpcap/**` 均为**上游原版**（MSVC 路线下 winfix.cc
开箱即用；libpcap 根本不参与构建），无本地补丁。

## 许可（NPSL，重要）

nmap 以 **Nmap Public Source License 0.95** 分发（`LICENSE` 全文随树），
= GPLv2 + 附加条款（冲突时附加条款优先）。与 qalc（GPL-2.0+）、parallel
（GPLv3+）同属 GPL 系内嵌组件，但 NPSL 有几条**更宽**的定义需要注意：

- §3 视「以执行 Nmap 并解析其结果为目的的软件」（通用 shell/菜单执行器
  除外）为衍生作品：本仓库的 veloterm_ffi.cc 薄封装按此大概率构成衍生，
  分发时该部分须以 NPSL 提供（含对应源码——vendor 树即完整源码——与
  许可证全文，不得叠加额外限制）；
- §7 不授予 Nmap 商标：衍生/修改版命名需避开 "Nmap" 字样（本项目仅原样
  分发未修改的上游 + 独立封装文件，命令名 `nmap` 指向上游本体，未改名）；
- §10 官方 Windows 安装包因捆绑 Npcap 整体禁止再分发——**本构建不捆绑
  Npcap**（用户自行安装），不受此条约束；
- 商业产品内嵌/再分发需 Nmap OEM 付费许可（https://nmap.org/oem/），
  与 libqalculate/parallel 的纯 GPL 义务不同，商用发布前需项目方确认。
