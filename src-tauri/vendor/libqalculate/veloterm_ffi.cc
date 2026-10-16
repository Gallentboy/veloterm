// VeloTerm FFI 入口：把 qalc 的 main 暴露为 C ABI 导出，供 coreutils.exe
// 运行时加载（见 crates/coreutils 的 qalc_impl.rs，与 yqffi.dll 同一形态）。
//
// qalc 的 CLI 主体在上游 src/qalc.cc 的 main()——构建脚本对该翻译单元
// 追加 `-Dmain=qalc_main` 改名后单独编译（BusyBox 多呼叫二进制的经典
// 手法），本文件只做转发。veloterm_qalc_main 以显式 __declspec(dllexport)
// 导出——注意：存在显式导出时 GNU ld 不再自动导出其余符号（删掉此标注
// 会静默丢符号，直到运行期 qalc_impl 报"缺少导出符号"）；extern "C"
// 保证符号名不被 C++ name mangling 破坏。

// 改名后的 qalc 入口（构建时 -Dmain=qalc_main 编译 src/qalc.cc 产出）
extern int qalc_main(int argc, char *argv[]);

extern "C" __declspec(dllexport) int veloterm_qalc_main(int argc,
                                                        const char *const argv[]) {
	// coreutils 侧已按 C 约定在 argv[argc] 补 NULL 哨兵；qalc 只按 argc
	// 读取，此处仅调整顶层 const 语义转发指针
	return qalc_main(argc, const_cast<char **>(argv));
}
