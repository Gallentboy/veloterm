# vendor/nmap（nmap 网络扫描器 CLI）

上游：https://github.com/nmap/nmap （nmap.org；上游自述见 `README.upstream.md`）
版本：**7.992**（锁在 `VELOTERM_VERSION`；2026-10-12 发布的最新稳定版，
GitHub tag v7.992）
来源：官方 dist 源码包 `https://nmap.org/dist/nmap-7.992.tar.bz2`
（sha256 `0c12660eddbf7eabeff6996cb0c044a4eadf2bb0c6fbcb340d6cf72b1caee402`），
整目录解压收入。**已删减 `zenmap/`**（Python GUI 前端，按「只嵌入非 GUI
部分」的要求裁掉，其余原样保留——ncat/nping 源码留在树内但构建以
`--without-ncat/--without-nping` 跳过，未嵌入）。

## 为什么是 MinGW DLL，而不是 MSVC 或静态链入

上游官方 Windows 构建是 MSVC（`mswin32/nmap.sln` + Win32 平台 + SVN 预编译
依赖），无 MinGW 支持承诺（issue #2927）；但 nmap 是 C/C++ + autotools 体系，
同样不具备 jq 那种纯 C 自包含、cc 现场编译链入 MSVC 的条件。故沿用
qalc（libqalculate）的例外形态：**源码 vendor + Windows 上经 MSYS2 MinGW64
构建 + `nmapffi.dll` 运行时 LoadLibrary**（分发臂在
`crates/coreutils/src/nmap_impl.rs`）。

上游在 MinGW 下的两处已知缺口由本仓库补齐（见下「本地补丁」）：

- `mswin32/winfix.cc` 不在 autotools 构建清单里（Makefile.in 的 OBJS 不含
  它），WIN32 下 `win_pre_init/win_init` 会 undefined reference；
- 缺 Npcap 时的优雅降级路径（SEH + delayimp）被 `_MSC_VER` 门控。

## 构建形态（`scripts/build-tools.mjs --only nmap`，仅 Windows）

在 MSYS2 MinGW64 环境（windows-latest 预装 `C:\msys64`，依赖由 CI 的 pacman
步骤安装：gcc / make / pkg-config / cmake / ninja / openssl）。构建目录
`src-tauri/nmap-build/`（已 gitignore）。源码树**整目录复制到构建目录做
in-tree 构建**（nmap 有 libdnet/liblua/libpcre/liblinear/libz/nbase/nsock
七个子 configure，VPATH 构建未经验，in-tree 拷贝最稳），步骤：

1. **Npcap SDK**：下载 `https://npcap.com/dist/npcap-sdk-1.13.zip`（与上游
   `mswin32/Build.bat` 同版），解压出 `Include/` + `Lib/x64/`。两处消费：
   - `npcap-shim/`（include/ + lib/ 平铺布局）给 libdnet-stripped 的
     `--with-wpdpack` 用（其 configure 只认 `<dir>/include/packet32.h` +
     `<dir>/lib/packet.lib` 布局）；
   - `Packet_ROOT` 给 libpcap 的 CMake 用。
   注意：MSYS2 自带的 `mingw-w64-x86_64-libpcap` 以 `-DPCAP_TYPE=null` 构建、
   无真实抓包后端，**不能用**。
2. **libpcap 静态库**（bundled `libpcap/`，1.10.7，CMake 官方 Windows 路线）：
   `cmake -G Ninja -DBUILD_SHARED_LIBS=OFF -DPacket_ROOT=<SDK>` → `libpcap.a`，
   install 进 `pcap-stage/`（include + lib），再把 Npcap SDK 的 `Include/*`
   （含 pcap/ 子目录，递归拷）并进 `pcap-stage/include`（nmap 侧的
   winfix.cc 要 `Packet32.h`）。选 CMake 静态编而不是直链 Npcap SDK 的
   `wpcap.lib`：pcap_* 全部编进 DLL、唯一外部导入收敛为 Packet.dll，
   且不赌 GNU ld 对 MSVC 短导入库的兼容性。CMake 路线经树内补丁改用
   **预生成**的 scanner.c/grammar.c（见下「本地补丁」第 4 条），零
   flex/bison 依赖；该补丁已在 macOS 本地 cmake configure + 全量构建
   验证（libpcap.a 产出、构建目录零生成物）。
3. **nmap 主体**：configure
   `--with-libpcap=<pcap-stage> --with-wpdpack=<npcap-shim>
   --with-openssl=/mingw64 --with-liblua=included --with-libpcre=included
   --with-libz=included --with-liblinear=included
   --without-libssh2 --without-ncat --without-nping --without-zenmap
   --without-ndiff`，`LIBS=-lpacket LDFLAGS=-L<npcap-shim>/lib`
   （libdnet.a 与 winfix.o 的 Packet* 符号靠它进最终链接）→ `make -j`。
   OpenSSL 走 MSYS2 mingw64 静态库（最终链接 `-static` 解析到 .a）；
   lua/pcre2/zlib/liblinear/libdnet 全部 bundled 静态。
4. **runner exe 与数据 staging**：按上游链接规则产出 `nmap.exe`（追加
   winfix.o，仅构建期使用，不随包分发）——用它以 `NMAPDIR=<源码树>` 跑
   `--script-updatedb` 生成 `scripts/script.db`（`make install-nse` 的
   NSE_FILES 依赖它），再 `make install-nmap install-nse` 把数据文件装进
   `nmap-data/`（nmap-services、nmap-os-db、nmap-service-probes、
   nmap-protocols、nmap-rpc、nmap-mac-prefixes、docs/nmap.xsl、docs/nmap.dtd、
   nse_main.lua、scripts/、nselib/，另补上游安装清单遗漏的 nmap-payloads），
   并以 `NMAPDIR=<stage>` 跑 `./stage/bin/nmap.exe --version` 与
   `-sT 127.0.0.1 -p 1` 冒烟（CI 无 Npcap，正好回归优雅降级路径）。
5. **链接 nmapffi.dll**：捕获 `make -n nmap` 的最终链接行，去掉 `main.o`
   （其 main 由 veloterm_ffi.cc 以 `#define main` 改名包含 main.cc 替代），
   追加 veloterm_ffi.o + winfix.o，改驱动为
   `g++ -shared -static -static-libgcc -static-libstdc++
   -Wl,--delayload,Packet.dll -ldelayimp` —— 依赖全静态打入，Packet.dll
   以**延迟导入**进 DLL（未装 Npcap 的机器上 LoadLibrary(nmapffi.dll) 仍
   成功，运行期由打补丁的 win_init 探测降级到 connect() 扫描）。
   `objdump` 依赖断言：除 Windows 系统库与 Packet.dll（延迟导入，也会被
   objdump 列出）外不得有任何 MinGW/MSYS 动态库。
6. 产物复制到 `src-tauri/binaries/nmapffi.dll` + `binaries/nmap-data/`，
   经 `tauri.windows.conf.json` 的 resources 放安装根目录（nmap-data/
   随 DLL 平铺在安装根）。

## 运行时行为

- coreutils.exe 的 `nmap` 分发臂（硬链接短名 `tools\nmap.exe` 或子命令
  模式）按 exe 同目录 → 上级目录搜索 `nmapffi.dll`（与 yqffi/qalcffi 同
  一套约定），调用前置 `NMAPDIR`（若用户未设）指向安装根的 `nmap-data/`
  ——上游在 Windows 下默认找 **exe 同目录** 的数据文件，而 DLL 形态下
  `GetModuleFileName(GetModuleHandle(0))` 取到的是宿主 coreutils.exe 路径
  （tools\ 子目录），必须经 NMAPDIR 兜底；用户显式 `--datadir`/`NMAPDIR`
  优先级仍在上游搜索序的前面，不受影响。
- **Npcap 依赖是运行期的**：未装 Npcap 时 nmap 仍可用（`--version`、
  `-sT` connect 扫描、服务/版本探测），SYN/UDP/OS 指纹等原始包能力提示
  安装 Npcap（https://npcap.com）；装了 Npcap（含 WinPcap 兼容模式）则全
  功能可用。nmap 的进度输出直接写继承的 ConPTY stdio，无 TUI、无交互
  prompt，进程内直跑安全。
- NSE 用户级脚本目录 `%APPDATA%\nmap/scripts`、`--datadir` 覆盖等上游
  语义原样保留。安装目录（Program Files）通常只读，用户跑
  `--script-updatedb` 会因无法回写 script.db 报错——script.db 已在构建期
  预生成随包分发，正常使用不需要重算。

## 本地补丁（升级时需逐条复核重放）

升级 = 换 vendor 源码 + 改 `VELOTERM_VERSION` + 重放下列补丁 + 复核
build-tools.mjs 的 configure/link flag 仍然成立：

1. `mswin32/winfix.cc`（搜 `VeloTerm patch`，三处）：
   - 配置头分家：`<nmap_winconfig.h>` 仅 MSVC，MinGW 走 `config.h`；包含
     路径反斜杠改正斜杠（GCC）；
   - `PacketGetVersion()/pcap_lib_version()` 调用移入 `o.have_pcap` 门内，
     MinGW 下先 `LoadLibraryA("Packet.dll")` 探测（`#ifndef _MSC_VER`），
     缺 Npcap 直接降级、不触发 delayload 解析（详见补丁注释）。
2. `veloterm_ffi.cc`（本仓库新增文件）：`#define main` 改名包含 `main.cc`
   + 导出 `veloterm_nmap_main`。
3. `libpcap/CMakeLists.txt`（搜 `VeloTerm patch`，三处）：
   - flex/bison 探测与生成段整体跳过（`VELOTERM_SHIPPED_GENERATED`），
     scanner/grammar 消费点改指源码树内**预生成**文件——树内无 grammar.y
     （仅 grammar.y.in 模板），bison 生成路径无从工作；MSYS2 也没有
     mingw 版 flex/bison 包（win_flex/win_bison，packages.msys2.org 实查
     404）。与 NMAP_MODIFICATIONS 0001 在 autotools 侧「直接用发行文件、
     不重新生成」的立场一致；
   - `rpcapd`/`testprogs` 的 add_subdirectory 加存在性守卫（nmap 发行树
     裁掉了这两个目录，上游无条件引用会 configure 失败）。
4. **删除上游 `.gitignore`**：nmap 的 git 仓忽略规则面向他们自己的开发仓
   （那里 `/libpcap/bpf_filter.c`、`/libz/zconf.h`、`/libpcap/*.3pcap`、
   `/libpcap/grammar.y` 等是生成物/不入库文件），而 dist tarball 实际携带
   这些文件——vendor 入库时被静默吞掉 43 个（`bpf_filter.c` 是 CMake 必需
   源、`zconf.h` 是 bundled zlib 的公共头，缺了必挂）。删除该文件并从
   tarball 恢复全部被吞文件入库。
5. 删减：`zenmap/` 整目录（GUI，不构建不分发）；上游 `README.md` 挪为
   `README.upstream.md` 给本文件让位。

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
