# libgif2mp4 — GIF → MP4 (H.264) 转码库（ffmpeg 树内第七库）

veloterm vendor 补丁模块：在 ffmpeg 源码树内新增一个子库，把「GIF 转
MP4」封装成一个纯 C ABI 函数（libavfilter 管线：gif demux → decode →
`[fps=N,]scale=trunc(iw/2)*2:trunc(ih/2)*2,format=yuv420p` → libx264
(crf/preset) → mp4 mux(+faststart)）。

## 文件

| 文件 | 说明 |
| --- | --- |
| `gif2mp4.h` | 对外 C ABI：`gif2mp4_run`（选项结构体 + 进度回调/取消）、`gif2mp4_set_log_callback`（日志回调，av_log 重定向）、错误码、TLS 错误串、`gif2mp4()` 位置式糖。线程模型与回调安全边界（NULL 守卫/可重入/禁止事项）见头文件注释 |
| `gif2mp4.c` | 实现：两遍法（第一遍 demux 数包得精确总帧数）、CFR/VFR 双模式、进度节流（首帧/每 16 帧/末帧）、临时文件 + 原子替换（Windows MoveFileExW）、进程级单飞（CAS，-7 busy）、绝不 exit() |
| `version.h` / `version.c` | 树内库惯例的版本三件套 |
| `Makefile` | ffbuild 子库清单（NAME/HEADERS/OBJS） |

## 构建接线补丁（ffmpeg 升级时重放）

1. `configure`：`LIBRARY_LIST` 头部加 `gif2mp4`（获得 `CONFIG_GIF2MP4`
   与 `--disable-gif2mp4` 开关）；
2. `configure`：`*_deps` 段加
   `gif2mp4_deps="avcodec avfilter avformat swscale"`（shared 构建时生成
   `GIF2MP4_FFLIBS` 依赖）；
3. `Makefile`：`ALLFFLIBS` 头部加 `gif2mp4`；
4. `Makefile`：`FFLIBS-$(CONFIG_GIF2MP4) += gif2mp4`（依赖者在前——
   gif2mp4 依赖全部兄弟库）。

## 构建命令（裁剪配置；产物 .a / MSVC 下 .lib）

> 裁剪集只收 gif 解封装/解码/**重组 parser**、mp4 封装、fps/scale/format
> 三滤镜与 libx264。**`--enable-parser=gif` 不可省**：gif demuxer 的
> read_packet 是固定字节块盲读（不看帧边界），块→帧重组完全靠解码侧
> 的 gif parser——缺了它首块之后全盘错位（LZW decode failed /
> Invalid image width 风暴），与全量构建的可用性差异即源于此。

```sh
./configure --disable-everything --disable-shared --enable-static \
  --disable-doc --disable-network --disable-autodetect \
  --enable-demuxer=gif --enable-decoder=gif --enable-parser=gif \
  --enable-muxer=mp4 \
  --enable-encoder=libx264 --enable-libx264 --enable-gpl \
  --enable-protocol=file --enable-filter=fps,scale,format,null,pad,crop \
  --enable-pic \
  --extra-libs=/opt/homebrew/Cellar/x264/r3222/lib/libx264.a \
  --prefix="$PWD/build"
make -j                  # 或 make libgif2mp4/libgif2mp4.a 只出本库
```

- macOS：x264 用 brew 的**静态库**（`libx264.a`）自包含链入；brew 只有
  动态 .dylib 时需先 `brew install x264` 确认 cellar 内 .a 存在。
- Windows：`--toolchain=msvc` 出 `.lib`（或 MSYS2 MinGW 出 `.a`，qalc
  先例）；x264 静态库由 vcpkg/MSYS2 提供。
- `gif2mp4` 库默认启用；`--disable-gif2mp4` 可整体裁掉。

## ⚠️ 许可（商用发布前需项目方确认）

x264（GPL-2+）+ `--enable-gpl` ⇒ 本库整体按 **GPL** 分发。静态链入宿主
二进制即宿主整体负 GPL 分发义务（比独立 DLL 运行期加载更紧）；veloterm
随包分发形态的商用确认同 libjxl/nmap 先例，由项目方拍板。

## API 一览（详见 gif2mp4.h）

```c
int  gif2mp4_run(const gif2mp4_options *opt, const char *in, const char *out);
int  gif2mp4(const char *in, const char *out, int fps, int crf,
             const char *preset, int loop);
void gif2mp4_set_log_callback(gif2mp4_log_cb cb, void *ud,
                              gif2mp4_log_level min_level);
const char *gif2mp4_last_error(void);
int  gif2mp4_abi_version(void);
```

参数语义：`fps ≤0` 跟随 GIF 时基（VFR），`>0` 固定 CFR（1-240）；`crf`
缺省 23（0-51）；`preset` 缺省 medium（白名单十档）；`loop` 为 no-op
保留位（MP4 无通用循环标志）。返回码 0 / -1..-8（见头文件错误码表）。

未注册任何回调时：日志回落 ffmpeg 默认回调（stderr），无进度即无取消，
全部路径不崩溃（内部所有回调发射先判空）。
