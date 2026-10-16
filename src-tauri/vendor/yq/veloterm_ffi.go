//go:build veloterm_ffi

// veloterm_ffi.go —— VeloTerm FFI 适配层（仅 `-tags veloterm_ffi` 构建时编译）。
//
// 上游 yq.go 的 main() 把逻辑封在进程入口里且失败时 os.Exit(1)；本文件把
// 同一段逻辑以 C 可调用形式导出，供 coreutils.exe 运行时加载 yqffi.dll 调用：
//   - 返回进程退出码（0/1），绝不 os.Exit —— FFI 场景下 os.Exit 会连宿主
//     coreutils 进程一起杀掉；
//   - 参数经 C argv 传入（宿主组装，argv[0] 为 "yq"），不复用进程 os.Args
//     —— DLL 里 os.Args 是宿主 coreutils 的参数，直接给 cobra 会解析错；
//   - 与上游的差异仅一处防御：args 为空时跳过 Find 回落判断（上游
//     `args[0]` 在空参 + Find 出错的组合下会越界 panic，本文件补 len 判空）。
//
// 构建（由 scripts/build-tools.mjs 在 Windows 上执行）：
//   CGO_ENABLED=1 go build -buildmode=c-shared -tags veloterm_ffi \
//     -ldflags "-s -w -X github.com/mikefarah/yq/v4/cmd.GitDescribe=<VELOTERM_VERSION>" \
//     -o yqffi.dll .
// 版本锁在仓库内本目录的 VELOTERM_VERSION（与 openssl-src 的双源纪律一致）。

package main

/*
#include <stdlib.h>  // size_t（下方指针步进用到）
int veloterm_yq_main(int argc, char** argv);
*/
import "C"

import (
	"unsafe"

	command "github.com/mikefarah/yq/v4/cmd"
)

//export veloterm_yq_main
func veloterm_yq_main(argc C.int, argv **C.char) C.int {
	// 按 C main 语义收取完整 argv（argv[0] 为程序名），随后如上游 main 一样
	// 丢弃 argv[0] —— cobra 的 SetArgs 期望 os.Args[1:] 形态（不含程序名），
	// 多传一个位置参数会把表达式/子命令位整体顶错位
	full := make([]string, 0, int(argc))
	for i := 0; i < int(argc); i++ {
		p := *(**C.char)(unsafe.Pointer(
			uintptr(unsafe.Pointer(argv)) + uintptr(i)*unsafe.Sizeof(*argv)))
		full = append(full, C.GoString(p))
	}
	if len(full) == 0 {
		full = []string{"yq"}
	}
	args := full[1:]

	cmd := command.New()
	// 上游语义：无匹配子命令时回落到默认的 eval 子命令
	if _, _, err := cmd.Find(args); err != nil && len(args) > 0 && args[0] != "__complete" {
		args = append([]string{"eval"}, args...)
	}
	// 显式 SetArgs：不复用 os.Args（见文件头注释）
	cmd.SetArgs(args)
	if err := cmd.Execute(); err != nil {
		return 1
	}
	return 0
}
