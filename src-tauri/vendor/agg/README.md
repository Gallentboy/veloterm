# agg - asciinema gif generator

> ## veloterm 集成说明（本地裁剪，零代码补丁）
>
> 上游版本：**v1.9.0**（tag，commit 26ca84c）。本工程以 **path 依赖引 lib 目标**
> （`src/lib.rs` 的 `pub fn run(input, output, Config)`），不走 CLI：终端录制的
> 「导出 GIF」入口（`src-tauri/src/terminal_recording.rs::export_recording_gif`）
> 把 .cast 文本喂给 `agg::run`，主题串（bg,fg,c0..c15 十六进制三元组）与字体
> 回退链（含 CJK 系统字体）由前端随命令传入，与回放器观感对齐。
>
> **树体裁剪清单**（相对上游 tarball；升级 = 换树 + 重放）：
> - 删 `tests/`、`demo.cast`、`demo.gif`、`.github/`、`Dockerfile`、
>   `flake.nix`/`flake.lock`、`CONTRIBUTING.md`；
> - `fonts/` 只留编译期 `include_bytes!` 硬依赖的 `NotoEmoji-Regular.ttf` +
>   `SymbolsNerdFont-Regular.ttf`（及各自许可 txt）；其余 JetBrainsMono /
>   NotoColorEmoji / NotoSansCJKjp 等仅供上游 `#[cfg(test)]` 与 demo 使用
>   ——测试目标不随本工程编译（vendor 被 workspace exclude），一并裁去。
>
> **本地代码补丁（2 处，升级 = 换树 + 重放）**：
>
> 1. `src/lib.rs`：gifski `Settings { fast: true }` → `false`。上游 CLI 为速度
>    写死 fast（libimagequant 量化 speed 强制 10 = 最低质量档，非首帧全部
>    打折；bg-keep 路径也无条件丢弃背景像素重要性），嵌入产品侧用户显式
>    导出、秒级预算内，质量优先改回 gifski 缺省（quality 本就是缺省 100）。
>    实测：同帧唯一色数 344 → 376，量化 banding 更少。
> 2. `src/lib.rs` + `src/main.rs`：`Config` 新增 `progress:
>    Option<Box<dyn Fn(u64, u64) + Send>>`（每帧渲染完成回调 (已完成帧数,
>    总帧数)，CLI 恒为 None）。嵌入方接 Tauri Channel 做 UI 进度条；
>    main.rs 的穷举构造同步补 `progress: None`。
>
> **许可证注意：上游 GPL-3.0-or-later，且其 GIF 编码内核 gifski 为
> AGPL-3.0-or-later（libimagequant 同族）——静态链入 app 后整个组合按
> 传染条款处理，商用发布前需项目方确认（同 libjxl GPL-3.0、nmap NPSL、
> nginx-language-server 数据的先例）。**
>
> 升级提示：上游 main 的 Cargo.toml 已新增 ureq 依赖（v1.9.0 尚无引用），
> 换更新版本时留意依赖面变化。

__agg__ is a command-line tool for generating animated GIF files from terminal
session recordings.

It supports conversion from [asciicast
v2](https://github.com/asciinema/asciinema/blob/master/doc/asciicast-v2.md)
files produced by [asciinema recorder](https://github.com/asciinema/asciinema).
It uses Kornel Lesiński's excellent
[gifski](https://github.com/ImageOptim/gifski) library to produce optimized,
high quality GIF output with accurate frame timing.

Example GIF file generated with agg:

![Example GIF file generated with agg](demo.gif)

Notable features:

- conversion of [asciicast](https://docs.asciinema.org/manual/asciicast/v3/)
  recordings (v1, v2, v3) to animated GIF files,
- input from local files, stdin, or HTTP(S) URLs (e.g.
  [asciinema.org](https://asciinema.org) recording links),
- high-quality, optimized GIF output with accurate frame timing via the
  [gifski](https://github.com/ImageOptim/gifski) encoder,
- multiple built-in color themes (asciinema, dracula, monokai, github-dark,
  github-light, kanagawa, nord, solarized-dark, solarized-light, gruvbox-dark,
  and more),
- custom ad-hoc themes specified as hex color triplets,
- automatic use of the recording's embedded theme when present,
- configurable [font families](https://docs.asciinema.org/manual/agg/usage/#fonts)
  with sensible cross-platform defaults and implicit fallbacks for symbols,
  including automatic Nerd Font symbols rendering,
- configurable font size and line height,
- additional font directory support via `--font-dir` for fonts outside standard
  system locations,
- color emoji rendering with support for Apple Color Emoji, Noto Color Emoji,
  and other common emoji fonts,
- two selectable rendering backends: `swash` (default) and `resvg`,
- adjustable playback speed,
- idle time limiting to skip periods of inactivity,
- frame selection by time ranges, discrete positions, markers, percentages, and
  event indexes,
- looped or single-pass playback,
- configurable FPS cap and last-frame duration,
- terminal size override (cols/rows) for re-rendering at a different geometry.

Check out the [agg docs](https://docs.asciinema.org/manual/agg/) for
installation and usage overview.

agg is a successor to
[asciicast2gif](https://github.com/asciinema/asciicast2gif).

## Building

Building from source requires [Rust](https://www.rust-lang.org/) compiler
(1.85.0 or later) and [Cargo package
manager](https://doc.rust-lang.org/cargo/). You can install both with
[rustup](https://rustup.rs/).

To download source code, build agg binary and install it in `$HOME/.cargo/bin`
run:

```bash
cargo install --git https://github.com/asciinema/agg
```

You need to ensure `$HOME/.cargo/bin` is in your shell's `$PATH`.

Alternatively, you can manually download source code and build agg binary with:

```bash
git clone https://github.com/asciinema/agg
cd agg
cargo build --release
```

This produces an executable file in _release mode_ (`--release`) at
`target/release/agg`. There are no other build artifacts so you can copy the
binary to a directory in your `$PATH`.

### Building with Docker

Alternatively, if you have Docker, Podman or another Docker-compatible tool
installed you can use it to build agg container image. This doesn't require Rust
toolchain installed on your machine.

Build the image with the following command:

```sh
docker build -t agg .
```

Then run agg like this:

```sh
docker run --rm -it -u $(id -u):$(id -g) -v $PWD:/data agg demo.cast demo.gif
```

If you use Podman in root-less mode:

```sh
podman run --rm -it -v $PWD:/data agg demo.cast demo.gif
```

## Consulting

If you're interested in customization of agg or any other asciinema component to
for your corporate needs, check [asciinema consulting
services](https://docs.asciinema.org/consulting/).

## License

© 2022 Marcin Kulik.


All code is licensed under the GPL, v3 or later. See [LICENSE](./LICENSE) file
for details.
