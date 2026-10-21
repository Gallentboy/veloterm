# `jpegxl-sys`

> ## veloterm 本地修补（runtime-loading 分支）
>
> 上游版本：`jpegxl-sys 0.12.1+libjxl-0.11.2`（crates.io，inflation/jpegxl-rs）。
> 本树在上游基础上改为**运行期动态加载**，pdfium-render `bind_to_library` 同型。
> 动机：上游 build.rs 用 pkg-config 做链接期动态链接——构建机没有 libjxl（或
> 没有 pkg-config，如 Windows runner）时 build script 直接 panic，Windows/CI
> 全不可编；且链接期依赖意味着进程启动时必须找到 jxl.dll/libjxl.dylib，用户
> 机器缺库会拖死整个宿主程序（终端都打不开）。运行期加载把缺库的影响面收敛
> 到「.jxl 预览报可诊断错误」。
>
> 补丁清单（升级 = 换上游源码 + 重放以下各项）：
>
> 1. **`Cargo.toml`**：删 build.rs/`links = "jxl"`/pkg-config/jpegxl-src
>    build-deps（不再有链接期依赖），加 `libloading = "0.8"`；删 dev-deps
>    与 `[lints]`（上游测试不随本工程跑）。
> 2. **`build.rs` 删除**（build-dependencies 归零）。
> 3. **`src/lib.rs`**：新增 `bind_to_library(path)`（进程内一次、`Box::leak`
>    永不卸载）与 `jxl_ffi!` 宏（见下）；删除原 `#[cfg(test)] mod test`
>    （依赖已移除的 dev-deps 与发布包里不存在的 `samples/` 目录）。
>    Windows 加载用 `LOAD_WITH_ALTERED_SEARCH_PATH`：`jxl.dll` 依赖同目录的
>    hwy/brotli/lcms 运行时 DLL，普通搜索序不含被加载 DLL 自身目录。
> 4. **8 个 extern 块头一行改**（块内 120 条 FFI 声明原文零改动）：
>    `extern "C-unwind" {` → `crate::jxl_ffi! {`，位于 decode.rs、
>    encoder/encode.rs、encoder/stats.rs、color/cms.rs、
>    threads/{thread,resizable}_parallel_runner.rs、metadata/{gain_map,compressed_icc}.rs。
>    宏为每条声明生成「函数指针字段（模块级符号表，惰性解析）+ 同名同签名的
>    `extern "C-unwind"` 包装函数」——包装保持 C ABI 是因为上游把部分函数
>    （如 `JxlThreadParallelRunner`）当函数指针值传参；符号表按模块惰性解析，
>    未用到的模块不触发解析（只带 jxl.dll、不带 jxl_threads.dll 也能解码）。
>
> 许可证：上游 GPL-3.0-or-later，随树保留 COPYING/LICENSE；该传染性义务与
> crates.io 依赖形态相同（本项目自 d34456f9 起引入本依赖时即已成立）。

`jpegxl-sys` provides a wrapper for the `libjxl` library.
For more details, refer to the [original library](https://github.com/libjxl/libjxl).

## Building

If you wish to specify a custom library path, set the `DEP_JXL_LIB` environment variable.

To build `libjxl` and statically link it, use the `vendored` feature.

## Usage

Check out testing units in `src/lib.rs` for some examples.

### Multi-thread

Because `libjxl_threads` uses `std::thread`, if you build and statically link `libjxl`, you need to
dynamically link to `libc++` or `libstdc++`.

Using the dynamic library doesn't need this step.
