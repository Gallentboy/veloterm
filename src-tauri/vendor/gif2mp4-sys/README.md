# gif2mp4-sys — libgif2mp4 的静态 Rust 绑定

ffmpeg 树内库 `vendor/ffmpeg/libgif2mp4/`（GIF→MP4 转码，C ABI）的
sys crate：**不编译任何 C 代码**——.a 族预编译好后由 build.rs 接线
（「库到时候编译好提供」，bc-app 同型的链接接线半边）。

## 构建前置

**无**——编译机不需要 configure/make ffmpeg：档案由
`vendor/ffmpeg-lib/<平台>/` 预提供（8 件齐全；产物命令见该目录
README.md）。

## build.rs 行为

- 校验 8 件档案存在且为 ar 归档（缺失时 panic 给出填充指引——早失败
  好过链接期满屏 UND）；
- 按依赖序发链接旗标：gif2mp4 → avfilter → swscale → avformat →
  avcodec → swresample → avutil → x264（压轴）；macOS 另附 pthread/m；
- 环境变量：`GIF2MP4_LIB_DIR` 覆盖整目录（8 件全从这取）；
- 平台目录映射：macos/aarch64 → mac-aarch64、macos/x86_64 →
  mac-x86_64、windows/x86_64 → win-x64、windows/aarch64 → win-arm64
  （Windows .lib 命名同形，待 CI 产出）。

## API（src/lib.rs 安全层）

```rust
gif2mp4_sys::install_log_bridge();                     // C 日志 → log crate（幂等）
gif2mp4_sys::convert(in, out, &ConvertParams { fps, crf, preset }, |p| {
    // p: &Progress { decoded_frames, total_frames, percent, out_time_sec, duration_sec }
    true // 返回 false 即取消
})?;                                                    // Err(Error { code, message })
```

- `Code` 枚举与 C 头文件错误码一一对应（Ok/Arg/Input/Decode/Encode/
  Output/Cancelled/Busy/Internal）；
- 阻塞调用（C 侧进程级单飞，并发第二路 Busy）；app 侧命令层在
  `src-tauri/src/gif2mp4.rs`（spawn_blocking + Channel 进度 + 取消）；
- 端到端 FFI 冒烟：app 测试 `convert_missing_input_maps_to_input_error`
  （链接真实 .a 跑通 C 侧 -2 路径）。

## ⚠️ 许可

x264（GPL-2+）+ ffmpeg `--enable-gpl` ⇒ 库按 **GPL** 分发；静态链入
app 二进制即整体负 GPL 分发义务——随包商用发布前需项目方确认
（libjxl/nmap 先例，已同步记录在 app Cargo.toml 依赖注释）。
