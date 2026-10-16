/* veloterm_ffi.cc -- VeloTerm 的 nmap FFI 壳（本仓库新增文件，非上游内容）。
 *
 * 与 qalc 的 -Dmain=qalc_main 改名直调同一手法，但 nmap 的 main() 本身就是
 * 薄壳（main.cc：locale 初始化 → set_program_name → NMAP_ARGS 环境变量拼接
 * → --resume 恢复 → nmap_main()），为原样保留全部入口语义（尤其
 * set_program_name——nmap_fetchfile_sub 断言其非空，数据文件搜索依赖它），
 * 这里以 `#define main` 改名后整文件包含 main.cc，再导出 C 入口供
 * coreutils.exe 运行时 LoadLibrary 调用（分发臂见 crates/coreutils 的
 * nmap_impl.rs）。
 *
 * Nmap 以 NPSL（Nmap Public Source License 0.95）分发，本文件的改动与
 * nmapffi.dll 的构建形态见本目录 README.md 的许可小节。
 */

#include <windows.h>

/* 改名 main.cc 的 main（ BusyBox 多呼叫二进制的入口改名手法；main.cc 内
 * 无其他名为 main 的符号，预处理替换安全）。必须在 include 之前定义。 */
#define main veloterm_nmap_main_impl
#include "main.cc"
#undef main

extern "C" __declspec(dllexport) int veloterm_nmap_main(int argc,
                                                        const char **argv) {
  /* main.cc 的入口逻辑（locale/set_program_name/NMAP_ARGS/--resume）全部
   * 经改名后的原函数执行；argv[0] 由调用方约定传 "nmap"。 */
  return veloterm_nmap_main_impl(argc, const_cast<char **>(argv));
}
