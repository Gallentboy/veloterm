/* veloterm_ffi.cc -- VeloTerm 的 nping FFI 壳（本仓库新增文件，非上游内容）。
 *
 * 与 nmap（vendor/nmap/veloterm_ffi.cc）、ncat（同目录 ncat_ffi.cc）同一手法：
 * `#define main` 改名后整文件包含 nping.cc，再导出 C 入口供 coreutils.exe
 * 运行时 LoadLibrary 调用（分发臂见 crates/coreutils 的 nping_impl.rs）。
 *
 * nping 是**独立可执行程序**，其 main 与 output.cc 里有 exit() 调用——对
 * coreutils 这种「单命令生命周期进程」而言等价于返回退出码。
 *
 * 注意：不能在此 include <windows.h>（同 ncat/nmap 的原因：nbase_winunix.h 需
 * 在 winsock2.h 之前 #define _WINSOCKAPI_，顺序颠倒会致 sockaddr 重定义）。
 */

/* 改名 nping.cc 的 main（nping 内仅此一处裸 main，预处理替换安全）。
 * 必须在 include 之前定义。 */
#define main veloterm_nping_main_impl
#include "nping.cc"
#undef main

extern "C" __declspec(dllexport) int veloterm_nping_main(int argc,
                                                         const char **argv) {
  /* nping 的入口逻辑经改名后的原函数执行；argv[0] 由调用方约定传 "nping"。 */
  return veloterm_nping_main_impl(argc, const_cast<char **>(argv));
}
