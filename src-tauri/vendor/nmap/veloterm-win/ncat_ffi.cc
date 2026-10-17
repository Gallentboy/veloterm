/* veloterm_ffi.cc -- VeloTerm 的 ncat FFI 壳（本仓库新增文件，非上游内容）。
 *
 * 与 nmap（vendor/nmap/veloterm_ffi.cc）、qalc（-Dmain=qalc_main）同一手法：
 * `#define main` 改名后整文件包含 ncat_main.c，再导出 C 入口供 coreutils.exe
 * 运行时 LoadLibrary 调用（分发臂见 crates/coreutils 的 ncat_impl.rs）。
 *
 * ncat 是**独立可执行程序**（不是库），其 main 里有大量 exit() 调用——对
 * coreutils 这种「单命令生命周期进程」而言等价于返回退出码（宿主进程随之
 * 结束），故无需像 eza/zoxide 那样把 process::exit 全改成返回值。
 *
 * 注意：不能在此 include <windows.h>（会先拉进旧 winsock.h，而 nbase_winunix.h
 * 需在 winsock2.h 之前 #define _WINSOCKAPI_ 抑制它；顺序颠倒会导致 ws2def.h 的
 * sockaddr 重定义，nmap 集成时 CI 实测过）。
 */

/* 改名 ncat_main.c 的 main（ncat 内仅此一处裸 main，预处理替换安全）。
 * 必须在 include 之前定义。 */
#define main veloterm_ncat_main_impl
#include "ncat_main.c"
#undef main

extern "C" __declspec(dllexport) int veloterm_ncat_main(int argc,
                                                        const char **argv) {
  /* ncat 的入口逻辑经改名后的原函数执行；argv[0] 由调用方约定传 "ncat"。 */
  return veloterm_ncat_main_impl(argc, const_cast<char **>(argv));
}
