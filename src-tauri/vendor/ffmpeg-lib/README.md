# ffmpeg-lib — libgif2mp4 的预编译静态库（分平台）

`vendor/ffmpeg/libgif2mp4`（ffmpeg 树内库）各平台构建产物的**归档位**：
`cargo build` 经 `vendor/gif2mp4-sys/build.rs` 从这里的对应平台目录取档
静态链入 app——**编译机不需要 configure/make ffmpeg**。

## 目录布局

```
ffmpeg-lib/
  mac-aarch64/   libgif2mp4.a + 五个 libav*/libsw*.a + libx264.a（8 件）
  mac-x86_64/    （待产）
  win-x64/       （待产：--toolchain=msvc 出 .lib，CI 车道）
  win-arm64/     （待产：同上）
```

## 产物方式（macOS）

在 `vendor/ffmpeg` 按其 `libgif2mp4/README.md` 的裁剪命令构建后，
把 7 个 .a 与 brew 的 `libx264.a` 拷进对应平台目录：

```sh
cd vendor/ffmpeg && make -j
cp libgif2mp4/libgif2mp4.a libavfilter/libavfilter.a \
   libswscale/libswscale.a libavformat/libavformat.a \
   libavcodec/libavcodec.a libswresample/libswresample.a \
   libavutil/libavutil.a \
   /opt/homebrew/opt/x264/lib/libx264.a \
   ../ffmpeg-lib/mac-aarch64/
```

mac x86_64：在 Intel 机器（CI macos-26-intel 腿同源）重复同命令。
Windows：CI 车道 `test_gif2mp4` 产物（--toolchain=msvc + vcpkg x264
静态链入；改名约定 `libavcodec.lib → avcodec.lib`——build.rs 期望无
lib 前缀）。推 tag `test_gif2mp4` 触发，产物 zip 下载解压到
win-x64/ / win-arm64/ 即可（车道详见 .github/workflows/test-gif2mp4.yml）。

## build.rs 取档序

1. 环境变量 `GIF2MP4_LIB_DIR`（整目录覆盖，8 件全从这取）；
2. 本目录的平台子目录（按 `CARGO_CFG_TARGET_OS`/`ARCH` 映射：
   macos+aarch64 → mac-aarch64 等）。

不做 ffmpeg 树内兜底——预编译档案是唯一来源（8 件齐全才可用）。
档案缺失时 build.rs panic 并打印指引（早失败好过链接期满屏 UND）。

## ⚠️ 许可

x264（GPL-2+）+ ffmpeg `--enable-gpl` ⇒ 本目录全部产物按 **GPL** 分发；
静态链入 app 即整体负 GPL 分发义务——**随包商用发布前需项目方确认**
（libjxl/nmap 先例；源码树与 gif2mp4-sys 的 README 有同步记录）。
