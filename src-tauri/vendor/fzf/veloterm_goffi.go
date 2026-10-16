//go:build veloterm_goffi

// veloterm_goffi.go —— yq + fzf 合并 DLL（veloterm_goffi.dll）的聚合层
// （仅 `-tags veloterm_goffi` 构建时编译，与 veloterm_ffi.go 同 package main）。
//
// 合并架构：宿主取 vendor/fzf 的 package main（自带 veloterm_fzf_main 导出与
// shell 资源 embed，fzf 上游逻辑在根 main 包、无法被外部 import），经
// vendor/go.work 跨模块引用 yq 的 cmd 包（可导入），本文件追加导出
// veloterm_yq_main——两工具共享同一份 Go runtime 与标准库（相比两个独立
// DLL 体积省约三成），Rust 侧按符号分发（crates/coreutils 的 goffi_impl.rs）。
// yq 入口逻辑与 vendor/yq/veloterm_ffi.go 完全同源（cobra SetArgs + Find
// 回落 eval + __complete 守卫），升级 yq 时两处需同步复核。
//
// GC 策略：DLL 随宿主 coreutils.exe 一进程一命令、随即退进程，常规按比例
// GC 无意义——init 里关闭（SetGCPercent(-1)），仅以 1GB 软上限兜底（巨型
// 输入文件防 OOM，到达上限时 Go 自动恢复回收）。
//
// 构建（由 scripts/build-tools.mjs 在 Windows 上执行，另设 GOAMD64=v3——
// 以 AVX2 为基线换取向量化性能，基线以下的老 CPU 会非法指令崩溃，回退只需
// 删掉该环境变量）：
//   CGO_ENABLED=1 GOAMD64=v3 go build -buildmode=c-shared \
//     -tags "veloterm_ffi veloterm_goffi" -trimpath -buildvcs=false \
//     -ldflags "-s -w -X github.com/mikefarah/yq/v4/cmd.GitDescribe=<ver> \
//               -X main.version=<ver> -X main.revision=" \
//     -o veloterm_goffi.dll .

package main

/*
#include <stdlib.h>  // size_t（下方指针步进用到）
int veloterm_yq_main(int argc, char** argv);
*/
import "C"

import (
	"runtime/debug"
	"unsafe"

	command "github.com/mikefarah/yq/v4/cmd"
)

func init() {
	debug.SetGCPercent(-1)
	debug.SetMemoryLimit(1024 * 1024 * 1024)
}

//export veloterm_yq_main
func veloterm_yq_main(argc C.int, argv **C.char) C.int {
	// 按 C main 语义收取完整 argv（argv[0] 为程序名），随后丢弃 argv[0]——
	// cobra 的 SetArgs 期望 os.Args[1:] 形态；不复用进程 os.Args（DLL 里
	// os.Args 是宿主 coreutils 的参数，直接给 cobra 会解析错）
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
	cmd.SetArgs(args)
	if err := cmd.Execute(); err != nil {
		return 1
	}
	return 0
}
