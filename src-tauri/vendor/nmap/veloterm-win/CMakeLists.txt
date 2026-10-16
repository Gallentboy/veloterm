# VeloTerm: nmapffi.dll 的 MSVC x64 构建工程（本仓库新增，非上游内容）。
#
# # 为什么自建 CMake 而不是直接用官方 mswin32/nmap.sln
#
# 官方 Windows 构建（mswin32/Build.bat + nmap.sln）是上游唯一维护的 Windows
# 路径，但它只有 **Win32(x86)** 平台配置（sln 里 `Release|x64` 映射回
# `Release|Win32`，各 vcxproj 亦无 x64 项），产物是 32 位 nmap.exe——32 位
# 镜像无法载入 64 位宿主 coreutils.exe。本工程以 CMake 复刻官方 vcxproj 的
# 源清单 / 预处理宏 / include 路径 / 库清单，改用 `-A x64` 出 **x64 DLL**
# （C 入口 veloterm_nmap_main 由 veloterm_ffi.cc 提供）。
#
# # 关键：不编译 libpcap，直接链 Npcap SDK（与官方一致）
#
# 官方 vcxproj 链 `wpcap.lib`（提供 pcap_* 符号）+ `packet.lib`（Packet* 符号），
# 并以 `DelayLoadDLLs=packet.dll;wpcap.dll` 让二者延迟加载——没装 Npcap 的
# 机器上仍能启动，运行期由 winfix.cc 的 SEH 探测降级 connect 扫描。本工程
# 原样照搬。故 nmap 侧**完全不需要** bundled libpcap 的 CMake/flex/bison 构建。
#
# # 依赖（全部源码在 vendor/nmap 树内，用各自官方 CMakeLists 子目录构建）
#
# - libpcre (PCRE2 8-bit 静态) / libz (zlib 静态) / libdnet-stripped (静态，
#   需 Npcap SDK 提供 Packet) —— add_subdirectory 直接消费其 CMake target，
#   不猜库文件名。
# - Npcap SDK：-DNPCAP_SDK_DIR=<含 Include/ 与 Lib/x64/ 的根>
# - OpenSSL：-DOPENSSL_DIR=<含 include/ 与 lib/libssl.lib、libcrypto.lib 的根>
#   （CI 复用 vcpkg 的 x64-windows-static，与 SQLCipher / openssl-app 同源）
#
# # 与官方的差异
#
# - **libssh2 未集成**：官方链 libssh2.lib（需单独 CMake 构建 + OpenSSL 集成），
#   成本高于收益。nmap_winconfig.h 的 HAVE_LIBSSH2 / LIBSSH2_INCLUDED 已加
#   VeloTerm 门控去掉，nse_libssh2.cc 不编（nmap.cc 用 `#if HAVE_LIBSSH2`、
#   nse_main.cc 用 `#ifdef HAVE_LIBSSH2`，未定义即自动跳过，NSE 的 ssh 脚本
#   报 unavailable，其余不受影响）。
# - 入口源用 veloterm_ffi.cc（`#define main` 改名包含 main.cc）替代 main.cc。
# - 不定义 ZLIB_WINAPI：那是匹配「预编译 WINAPI 约定 zlibwapi.dll」用的；
#   我们自建静态 zlib（默认 cdecl），两边一致即可，无需该宏。

cmake_minimum_required(VERSION 3.20)
project(nmapffi LANGUAGES C CXX)

if(NOT MSVC)
    message(FATAL_ERROR "本工程仅支持 MSVC（上游 Windows 构建即 MSVC，无 MinGW 支持）")
endif()
foreach(required_var NPCAP_SDK_DIR OPENSSL_DIR)
    if(NOT ${required_var})
        message(FATAL_ERROR "缺少必需变量 -D${required_var}=<路径>")
    endif()
endforeach()

set(NMAP_ROOT "${CMAKE_CURRENT_SOURCE_DIR}/.." CACHE PATH "nmap 源码根（vendor/nmap）")
set(CMAKE_C_STANDARD 99)
set(CMAKE_CXX_STANDARD 14)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

# 静态 CRT（/MT）：与 vcpkg 的 x64-windows-static OpenSSL 一致（该 triplet 即
# VCPKG_CRT_LINKAGE=static）。DLL 自带 CRT 副本，不影响宿主进程。
set(CMAKE_MSVC_RUNTIME_LIBRARY "MultiThreaded$<$<CONFIG:Debug>:Debug>" CACHE STRING "" FORCE)

# ── 预处理宏（复刻官方 vcxproj 的 WIN32 集） ─────────────────────────────────
add_compile_definitions(
    WIN32
    _CONSOLE
    BPF_MAJOR_VERSION
    PCRE2_STATIC
    _CRT_SECURE_NO_WARNINGS
    _CRT_NONSTDC_NO_WARNINGS
)

# ── 子目录：pcre2 / zlib / libdnet（各自官方 CMakeLists，消费其 target） ─────
set(BUILD_SHARED_LIBS OFF CACHE BOOL "" FORCE)

set(PCRE2_BUILD_PCRE2_8 ON CACHE BOOL "" FORCE)
set(PCRE2_BUILD_PCRE2_16 OFF CACHE BOOL "" FORCE)
set(PCRE2_BUILD_PCRE2_32 OFF CACHE BOOL "" FORCE)
set(PCRE2_SUPPORT_JIT OFF CACHE BOOL "" FORCE)
set(BUILD_STATIC_LIBS ON CACHE BOOL "" FORCE)
set(PCRE2_BUILD_TESTS OFF CACHE BOOL "" FORCE)
add_subdirectory("${NMAP_ROOT}/libpcre" "${CMAKE_BINARY_DIR}/pcre2" EXCLUDE_FROM_ALL)

set(ZLIB_BUILD_SHARED OFF CACHE BOOL "" FORCE)
set(ZLIB_BUILD_STATIC ON CACHE BOOL "" FORCE)
set(ZLIB_BUILD_TESTING OFF CACHE BOOL "" FORCE)
set(ZLIB_INSTALL OFF CACHE BOOL "" FORCE)
add_subdirectory("${NMAP_ROOT}/libz" "${CMAKE_BINARY_DIR}/zlib" EXCLUDE_FROM_ALL)

# libdnet 的 FindPacket 从 Packet_ROOT 找 Npcap SDK（其 CMakeLists 设了
# CMAKE_MODULE_PATH 指向自带 cmake/Modules）
set(Packet_ROOT "${NPCAP_SDK_DIR}" CACHE PATH "" FORCE)
add_subdirectory("${NMAP_ROOT}/libdnet-stripped" "${CMAKE_BINARY_DIR}/dnet" EXCLUDE_FROM_ALL)

# ── include 路径（官方 vcxproj 的 AdditionalIncludeDirectories） ─────────────
# 注意 mswin32/：官方在那里放 NET/ 与 NETINET/ 两个兼容头目录（如
# NETINET/IN_SYSTM.H 充当 <netinet/in_systm.h>，MSVC 大小写不敏感匹配）——
# 缺它 libnetutil/netutil.cc 会 C1083 找不到 netinet/in_systm.h。
include_directories(
    "${NMAP_ROOT}"
    "${NMAP_ROOT}/mswin32"
    "${NMAP_ROOT}/liblua"
    "${NMAP_ROOT}/nbase"
    "${NMAP_ROOT}/nsock/include"
    "${NMAP_ROOT}/libdnet-stripped/include"
    "${NMAP_ROOT}/liblinear"
    # zlib 公共头（zlib.h/zconf.h 在源目录，官方 vcxproj 的 `..\libz`）
    "${NMAP_ROOT}/libz"
    # pcre2.h 是 CMake 从 src/pcre2.h.in 生成的（官方 vcxproj 的
    # `build-pcre2\interface`）；子目录构建落在 <build>/pcre2/interface
    "${CMAKE_BINARY_DIR}/pcre2/interface"
    "${NPCAP_SDK_DIR}/Include"
    "${OPENSSL_DIR}/include"
)

# ── 各静态库源码（官方各自 vcxproj 的源清单） ────────────────────────────────

# nbase：Windows 兼容层（snprintf.c 提供 asprintf/vasprintf 模拟，MSVC 下必需）
add_library(veloterm_nbase STATIC
    "${NMAP_ROOT}/nbase/getopt.c"
    "${NMAP_ROOT}/nbase/inet_ntop.c"
    "${NMAP_ROOT}/nbase/inet_pton.c"
    "${NMAP_ROOT}/nbase/nbase_addrset.c"
    "${NMAP_ROOT}/nbase/nbase_memalloc.c"
    "${NMAP_ROOT}/nbase/nbase_misc.c"
    "${NMAP_ROOT}/nbase/nbase_rnd.c"
    "${NMAP_ROOT}/nbase/nbase_str.c"
    "${NMAP_ROOT}/nbase/nbase_time.c"
    "${NMAP_ROOT}/nbase/nbase_winunix.c"
    "${NMAP_ROOT}/nbase/snprintf.c"
    "${NMAP_ROOT}/nbase/strcasecmp.c"
)

# nsock：异步 I/O 事件循环（engine_*.c 由 HAVE_* 宏门控，全量编入即可）。
# 官方 nsock.vcxproj 对该项目全部源设 CompileAsCpp（把 .c 当 C++ 编）：
# engine_iocp.c 的 `sin6_addr = IN6ADDR_ANY_INIT`（聚合初始化）在 C++ 下合法、
# C 下报 `error C2059: syntax error: '{'`（CI 实测）。故设 LANGUAGE CXX。
add_library(veloterm_nsock STATIC
    "${NMAP_ROOT}/nsock/src/engine_epoll.c"
    "${NMAP_ROOT}/nsock/src/engine_iocp.c"
    "${NMAP_ROOT}/nsock/src/engine_kqueue.c"
    "${NMAP_ROOT}/nsock/src/engine_poll.c"
    "${NMAP_ROOT}/nsock/src/engine_select.c"
    "${NMAP_ROOT}/nsock/src/error.c"
    "${NMAP_ROOT}/nsock/src/filespace.c"
    "${NMAP_ROOT}/nsock/src/gh_heap.c"
    "${NMAP_ROOT}/nsock/src/netutils.c"
    "${NMAP_ROOT}/nsock/src/nsock_connect.c"
    "${NMAP_ROOT}/nsock/src/nsock_core.c"
    "${NMAP_ROOT}/nsock/src/nsock_engines.c"
    "${NMAP_ROOT}/nsock/src/nsock_event.c"
    "${NMAP_ROOT}/nsock/src/nsock_iod.c"
    "${NMAP_ROOT}/nsock/src/nsock_log.c"
    "${NMAP_ROOT}/nsock/src/nsock_pcap.c"
    "${NMAP_ROOT}/nsock/src/nsock_pool.c"
    "${NMAP_ROOT}/nsock/src/nsock_proxy.c"
    "${NMAP_ROOT}/nsock/src/nsock_read.c"
    "${NMAP_ROOT}/nsock/src/nsock_ssl.c"
    "${NMAP_ROOT}/nsock/src/nsock_timers.c"
    "${NMAP_ROOT}/nsock/src/nsock_write.c"
    "${NMAP_ROOT}/nsock/src/proxy_http.c"
    "${NMAP_ROOT}/nsock/src/proxy_socks4.c"
)
set_source_files_properties(
    "${NMAP_ROOT}/nsock/src/engine_epoll.c"
    "${NMAP_ROOT}/nsock/src/engine_iocp.c"
    "${NMAP_ROOT}/nsock/src/engine_kqueue.c"
    "${NMAP_ROOT}/nsock/src/engine_poll.c"
    "${NMAP_ROOT}/nsock/src/engine_select.c"
    "${NMAP_ROOT}/nsock/src/error.c"
    "${NMAP_ROOT}/nsock/src/filespace.c"
    "${NMAP_ROOT}/nsock/src/gh_heap.c"
    "${NMAP_ROOT}/nsock/src/netutils.c"
    "${NMAP_ROOT}/nsock/src/nsock_connect.c"
    "${NMAP_ROOT}/nsock/src/nsock_core.c"
    "${NMAP_ROOT}/nsock/src/nsock_engines.c"
    "${NMAP_ROOT}/nsock/src/nsock_event.c"
    "${NMAP_ROOT}/nsock/src/nsock_iod.c"
    "${NMAP_ROOT}/nsock/src/nsock_log.c"
    "${NMAP_ROOT}/nsock/src/nsock_pcap.c"
    "${NMAP_ROOT}/nsock/src/nsock_pool.c"
    "${NMAP_ROOT}/nsock/src/nsock_proxy.c"
    "${NMAP_ROOT}/nsock/src/nsock_read.c"
    "${NMAP_ROOT}/nsock/src/nsock_ssl.c"
    "${NMAP_ROOT}/nsock/src/nsock_timers.c"
    "${NMAP_ROOT}/nsock/src/nsock_write.c"
    "${NMAP_ROOT}/nsock/src/proxy_http.c"
    "${NMAP_ROOT}/nsock/src/proxy_socks4.c"
    PROPERTIES LANGUAGE CXX
)

# libnetutil：报文头解析/构造
add_library(veloterm_netutil STATIC
    "${NMAP_ROOT}/libnetutil/ARPHeader.cc"
    "${NMAP_ROOT}/libnetutil/DestOptsHeader.cc"
    "${NMAP_ROOT}/libnetutil/EthernetHeader.cc"
    "${NMAP_ROOT}/libnetutil/FragmentHeader.cc"
    "${NMAP_ROOT}/libnetutil/HopByHopHeader.cc"
    "${NMAP_ROOT}/libnetutil/ICMPv4Header.cc"
    "${NMAP_ROOT}/libnetutil/ICMPv6Header.cc"
    "${NMAP_ROOT}/libnetutil/ICMPv6Option.cc"
    "${NMAP_ROOT}/libnetutil/ICMPv6RRBody.cc"
    "${NMAP_ROOT}/libnetutil/IPv4Header.cc"
    "${NMAP_ROOT}/libnetutil/IPv6Header.cc"
    "${NMAP_ROOT}/libnetutil/massdns.cc"
    "${NMAP_ROOT}/libnetutil/netutil.cc"
    "${NMAP_ROOT}/libnetutil/NetBlock.cc"
    "${NMAP_ROOT}/libnetutil/NetworkLayerElement.cc"
    "${NMAP_ROOT}/libnetutil/PacketElement.cc"
    "${NMAP_ROOT}/libnetutil/PacketParser.cc"
    "${NMAP_ROOT}/libnetutil/packettrace.cc"
    "${NMAP_ROOT}/libnetutil/RawData.cc"
    "${NMAP_ROOT}/libnetutil/RoutingHeader.cc"
    "${NMAP_ROOT}/libnetutil/TCPHeader.cc"
    "${NMAP_ROOT}/libnetutil/TransportLayerElement.cc"
    "${NMAP_ROOT}/libnetutil/UDPHeader.cc"
)

# liblua：NSE 的 Lua 5.4 运行时（lua.c/luac.c 是 standalone 解释器/编译器，
# 各含 main，nmap 不需要，排除）
add_library(veloterm_lua STATIC
    "${NMAP_ROOT}/liblua/lapi.c"
    "${NMAP_ROOT}/liblua/lauxlib.c"
    "${NMAP_ROOT}/liblua/lbaselib.c"
    "${NMAP_ROOT}/liblua/lcode.c"
    "${NMAP_ROOT}/liblua/lcorolib.c"
    "${NMAP_ROOT}/liblua/lctype.c"
    "${NMAP_ROOT}/liblua/ldblib.c"
    "${NMAP_ROOT}/liblua/ldebug.c"
    "${NMAP_ROOT}/liblua/ldo.c"
    "${NMAP_ROOT}/liblua/ldump.c"
    "${NMAP_ROOT}/liblua/lfunc.c"
    "${NMAP_ROOT}/liblua/lgc.c"
    "${NMAP_ROOT}/liblua/linit.c"
    "${NMAP_ROOT}/liblua/liolib.c"
    "${NMAP_ROOT}/liblua/llex.c"
    "${NMAP_ROOT}/liblua/lmathlib.c"
    "${NMAP_ROOT}/liblua/lmem.c"
    "${NMAP_ROOT}/liblua/loadlib.c"
    "${NMAP_ROOT}/liblua/lobject.c"
    "${NMAP_ROOT}/liblua/lopcodes.c"
    "${NMAP_ROOT}/liblua/loslib.c"
    "${NMAP_ROOT}/liblua/lparser.c"
    "${NMAP_ROOT}/liblua/lstate.c"
    "${NMAP_ROOT}/liblua/lstring.c"
    "${NMAP_ROOT}/liblua/lstrlib.c"
    "${NMAP_ROOT}/liblua/ltable.c"
    "${NMAP_ROOT}/liblua/ltablib.c"
    "${NMAP_ROOT}/liblua/ltm.c"
    "${NMAP_ROOT}/liblua/lundump.c"
    "${NMAP_ROOT}/liblua/lutf8lib.c"
    "${NMAP_ROOT}/liblua/lvm.c"
    "${NMAP_ROOT}/liblua/lzio.c"
)

# liblinear：OS 指纹的机器学习分类器
add_library(veloterm_linear STATIC
    "${NMAP_ROOT}/liblinear/blas/daxpy.c"
    "${NMAP_ROOT}/liblinear/blas/ddot.c"
    "${NMAP_ROOT}/liblinear/blas/dnrm2.c"
    "${NMAP_ROOT}/liblinear/blas/dscal.c"
    "${NMAP_ROOT}/liblinear/linear.cpp"
    "${NMAP_ROOT}/liblinear/newton.cpp"
)

# ── nmap 主体（官方 nmap.vcxproj 的源清单；入口单独，见下） ──────────────────
# 抽成 OBJECT 库：nmapffi.dll 与 nmap-runner.exe 共享同一批目标文件，仅入口
# 不同（DLL 用 veloterm_ffi.cc 的改名入口，runner 用上游 main.cc 的原生 main）。
add_library(veloterm_nmap_core OBJECT
    "${NMAP_ROOT}/charpool.cc"
    "${NMAP_ROOT}/FingerPrintResults.cc"
    "${NMAP_ROOT}/FPEngine.cc"
    "${NMAP_ROOT}/FPModel.cc"
    "${NMAP_ROOT}/idle_scan.cc"
    "${NMAP_ROOT}/MACLookup.cc"
    "${NMAP_ROOT}/NewTargets.cc"
    "${NMAP_ROOT}/nmap.cc"
    "${NMAP_ROOT}/nmap_dns.cc"
    "${NMAP_ROOT}/nmap_error.cc"
    "${NMAP_ROOT}/nmap_ftp.cc"
    "${NMAP_ROOT}/nmap_tty.cc"
    "${NMAP_ROOT}/NmapOps.cc"
    "${NMAP_ROOT}/NmapOutputTable.cc"
    "${NMAP_ROOT}/osscan.cc"
    "${NMAP_ROOT}/osscan2.cc"
    "${NMAP_ROOT}/output.cc"
    "${NMAP_ROOT}/payload.cc"
    "${NMAP_ROOT}/portlist.cc"
    "${NMAP_ROOT}/portreasons.cc"
    "${NMAP_ROOT}/protocols.cc"
    "${NMAP_ROOT}/scan_engine.cc"
    "${NMAP_ROOT}/scan_engine_connect.cc"
    "${NMAP_ROOT}/scan_engine_raw.cc"
    "${NMAP_ROOT}/scan_lists.cc"
    "${NMAP_ROOT}/service_scan.cc"
    "${NMAP_ROOT}/services.cc"
    "${NMAP_ROOT}/string_pool.cc"
    "${NMAP_ROOT}/Target.cc"
    "${NMAP_ROOT}/TargetGroup.cc"
    "${NMAP_ROOT}/targets.cc"
    "${NMAP_ROOT}/tcpip.cc"
    "${NMAP_ROOT}/timing.cc"
    "${NMAP_ROOT}/traceroute.cc"
    "${NMAP_ROOT}/utils.cc"
    "${NMAP_ROOT}/xml.cc"
    # NSE（nse_libssh2.cc 未集成，见文件头说明）
    "${NMAP_ROOT}/nse_db.cc"
    "${NMAP_ROOT}/nse_debug.cc"
    "${NMAP_ROOT}/nse_dnet.cc"
    "${NMAP_ROOT}/nse_fs.cc"
    "${NMAP_ROOT}/nse_lpeg.cc"
    "${NMAP_ROOT}/nse_main.cc"
    "${NMAP_ROOT}/nse_nmaplib.cc"
    "${NMAP_ROOT}/nse_nsock.cc"
    "${NMAP_ROOT}/nse_openssl.cc"
    "${NMAP_ROOT}/nse_ssl_cert.cc"
    "${NMAP_ROOT}/nse_utility.cc"
    "${NMAP_ROOT}/nse_zlib.cc"
    # Windows 专用初始化（官方在 nmap.vcxproj 里单列 mswin32/winfix.cc）
    "${NMAP_ROOT}/mswin32/winfix.cc"
)

# 入口文件 #include "main.cc"（相对路径），需 nmap 根为 include 基准
target_include_directories(veloterm_nmap_core PRIVATE "${NMAP_ROOT}")

# 共享链接依赖（DLL 与 runner 同源）
set(VELOTERM_NMAP_LIBS
    veloterm_nbase
    veloterm_nsock
    veloterm_netutil
    veloterm_lua
    veloterm_linear
    pcre2-8-static
    zlibstatic
    dnet
    # Npcap SDK 导入库（pcap_* 来自 wpcap，Packet* 来自 packet）
    "${NPCAP_SDK_DIR}/Lib/x64/wpcap.lib"
    "${NPCAP_SDK_DIR}/Lib/x64/Packet.lib"
    # OpenSSL（vcpkg 静态，与 SQLCipher / openssl-app 共用）
    "${OPENSSL_DIR}/lib/libssl.lib"
    "${OPENSSL_DIR}/lib/libcrypto.lib"
    # Windows 系统库（官方 AdditionalDependencies 的 sys 部分）
    ws2_32 iphlpapi advapi32 shell32 user32 gdi32 crypt32
    # 延迟加载辅助（/DELAYLOAD 运行时依赖 __delayLoadHelper2）
    delayimp
)

# ── nmapffi.dll：veloterm_nmap_core + FFI 入口 ──────────────────────────────
add_library(nmapffi SHARED
    "${CMAKE_CURRENT_SOURCE_DIR}/../veloterm_ffi.cc"
    $<TARGET_OBJECTS:veloterm_nmap_core>
)
target_link_libraries(nmapffi PRIVATE ${VELOTERM_NMAP_LIBS})

# 延迟加载 packet.dll / wpcap.dll：没装 Npcap 时 DLL 仍可加载，运行期由
# winfix.cc 的 SEH 探测把 have_pcap 置 false 并降级 connect 扫描（官方行为）
target_link_options(nmapffi PRIVATE
    "/DELAYLOAD:packet.dll"
    "/DELAYLOAD:wpcap.dll"
)
set_target_properties(nmapffi PROPERTIES
    OUTPUT_NAME "nmapffi"
    PREFIX ""
    RUNTIME_OUTPUT_DIRECTORY "${CMAKE_BINARY_DIR}/out"
    RUNTIME_OUTPUT_DIRECTORY_RELEASE "${CMAKE_BINARY_DIR}/out"
)

# ── runner nmap.exe（构建期工具，不随包分发） ────────────────────────────────
# 上游 main.cc 原样编成控制台 exe：用于 ① 跑 --script-updatedb 生成
# scripts/script.db（NSE 运行期索引，nmap-data/ 的一部分）；② 无 Npcap 环境的
# 构建期冒烟（--version 与 -sT connect 扫描，正好回归 /DELAYLOAD 降级路径）。
# 与 nmapffi 共享 veloterm_nmap_core，仅入口不同。
add_executable(nmap-runner
    "${NMAP_ROOT}/main.cc"
    $<TARGET_OBJECTS:veloterm_nmap_core>
)
target_link_libraries(nmap-runner PRIVATE ${VELOTERM_NMAP_LIBS})
target_link_options(nmap-runner PRIVATE
    "/DELAYLOAD:packet.dll"
    "/DELAYLOAD:wpcap.dll"
)
set_target_properties(nmap-runner PROPERTIES
    OUTPUT_NAME "nmap"
    RUNTIME_OUTPUT_DIRECTORY "${CMAKE_BINARY_DIR}/out"
    RUNTIME_OUTPUT_DIRECTORY_RELEASE "${CMAKE_BINARY_DIR}/out"
)
