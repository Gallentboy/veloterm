//go:build veloterm_ffi

// veloterm_ffi.go —— VeloTerm FFI 适配层（仅 `-tags veloterm_ffi` 构建时编译）。
//
// 上游 main.go 的 main() 失败时经本包 exit() 直接 os.Exit，会把宿主
// coreutils.exe 进程一起杀掉；本文件把同一段入口逻辑（参数解析 →
// Bash/Zsh/Fish/Nushell/Help/Version/Man 分支 → fzf.Run）以 C 可调用形式
// 导出，供 coreutils.exe 运行时加载 fzfffi.dll 调用：
//   - 返回进程退出码（0 / 1=NoMatch / 2=Error / 130=Interrupt），绝不
//     os.Exit；
//   - 参数经 C argv 传入（宿主组装，argv[0] 为 "fzf"），不复用进程
//     os.Args——DLL 里 os.Args 是宿主 coreutils 的参数；
//   - 调用期间临时替换 os.Args 为完整 argv：Run() 的 --popup（tmux/zellij）
//     与 winpty 路径会以 os.Args 再 exec 自身——替换后重入的是 tools\fzf.exe
//     硬链接（即再次进入本 FFI 路径），语义与独立 fzf 可执行文件一致；
//   - Bash/Zsh/Fish/Nushell 脚本与 man page 直接复用 main.go 的同包
//     //go:embed 变量与 printScript（同一 package main，无需复制）。
//
// 构建（由 scripts/build-tools.mjs 在 Windows 上执行；本包是合并 DLL 的宿主，
// 另有 veloterm_goffi.go 追加导出 yq 入口，双 tag 一并激活）：
//   CGO_ENABLED=1 GOAMD64=v3 go build -buildmode=c-shared \
//     -tags "veloterm_ffi veloterm_goffi" -trimpath -buildvcs=false \
//     -ldflags "-s -w -X github.com/mikefarah/yq/v4/cmd.GitDescribe=<ver> -X main.version=<VELOTERM_VERSION> -X main.revision=" \
//     -o veloterm_goffi.dll .
// 版本锁在仓库内本目录的 VELOTERM_VERSION（与 openssl-src 的双源纪律一致）。
// 升级复核：对照新版 main.go 核对本文件复刻的分支集合是否有增删
// （0.74.5 相对上游 master 曾多出 Nushell 分支，逐版本会有漂移）。

package main

/*
#include <stdlib.h>  // size_t（下方指针步进用到）
int veloterm_fzf_main(int argc, char** argv);
*/
import "C"

import (
	"fmt"
	"os"
	"os/exec"
	"unsafe"

	fzf "github.com/junegunn/fzf/src"
	"github.com/junegunn/fzf/src/protector"
)

//export veloterm_fzf_main
func veloterm_fzf_main(argc C.int, argv **C.char) C.int {
	// 按 C main 语义收取完整 argv（argv[0] 为程序名）
	full := make([]string, 0, int(argc))
	for i := 0; i < int(argc); i++ {
		p := *(**C.char)(unsafe.Pointer(
			uintptr(unsafe.Pointer(argv)) + uintptr(i)*unsafe.Sizeof(*argv)))
		full = append(full, C.GoString(p))
	}
	if len(full) == 0 {
		full = []string{"fzf"}
	}

	// Run() 的 popup/winpty 再 exec 路径读 os.Args（见文件头注释）
	savedArgs := os.Args
	os.Args = full
	defer func() { os.Args = savedArgs }()

	protector.Protect()

	options, err := fzf.ParseOptions(true, full[1:])
	if err != nil {
		fmt.Fprintln(os.Stderr, err.Error())
		return C.int(fzf.ExitError)
	}
	if options.Bash {
		printScript("key-bindings.bash", bashKeyBindings)
		printScript("completion.bash", bashCompletion)
		return 0
	}
	if options.Zsh {
		printScript("key-bindings.zsh", zshKeyBindings)
		printScript("completion.zsh", zshCompletion)
		return 0
	}
	if options.Fish {
		printScript("key-bindings.fish", fishKeyBindings)
		printScript("completion.fish", fishCompletion)
		return 0
	}
	if options.Nushell {
		printScript("key-bindings.nu", nushellKeyBindings)
		printScript("completion.nu", nushellCompletion)
		return 0
	}
	if options.Help {
		fmt.Print(fzf.Usage)
		return 0
	}
	if options.Version {
		if len(revision) > 0 {
			fmt.Printf("%s (%s)\n", version, revision)
		} else {
			fmt.Println(version)
		}
		return 0
	}
	if options.Man {
		file := fzf.WriteTemporaryFile([]string{string(manPage)}, "\n")
		if len(file) == 0 {
			fmt.Print(string(manPage))
			return 0
		}
		defer os.Remove(file)
		cmd := exec.Command("man", file)
		cmd.Stdin = os.Stdin
		cmd.Stdout = os.Stdout
		if err := cmd.Run(); err != nil {
			fmt.Print(string(manPage))
		}
		return 0
	}

	code, err := fzf.Run(options)
	if code == fzf.ExitError && err != nil {
		fmt.Fprintln(os.Stderr, err.Error())
	}
	return C.int(code)
}
