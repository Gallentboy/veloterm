# vendor/libjxl — Windows 端 libjxl 动态库（随包分发）

JPEG XL 解码（图片预览的 `.jxl` 格式）经树内 vendor 的 `jpegxl-rs` / `jpegxl-sys`
（运行期加载分支，见 `../jpegxl-sys/README.md`「本地修补」）绑定 libjxl。
macOS 走 Homebrew 系统库（`brew install jpeg-xl`），不随包分发、缺库只影响
`.jxl` 预览；Windows 无系统级分发渠道，DLL 需放进本目录随安装包分发。

## 产物来源

`test-libjxl.yml` 车道（tag `test_libjxl` 触发）用 vcpkg 在两种 Windows runner
上编译，产物打包为：

- `libjxl-x64.zip`（windows-latest / `x64-windows`）
- `libjxl-arm64.zip`（windows-11-arm / `arm64-windows`）
- 各附一份 `-manifest.txt` 清单（文件名 + 字节数）

## 放置约定

解压对应架构的 zip，把**全部 DLL 原样**放进本目录下的架构子目录（保留 vcpkg
产出的原始文件名，不要重命名——它们是 libjxl 及其依赖的真实模块名）：

```
vendor/libjxl/
├── x64/      ← libjxl-x64.zip 的全部内容（jxl/jxl_cms/jxl_threads/hwy/lcms2-2/brotli×3）
├── arm64/    ← libjxl-arm64.zip 的全部内容
└── README.md
```

**为什么必须全部 DLL**：`jxl.dll` 依赖 `hwy` / `brotli` / `lcms` 的运行时 DLL，
只拷 jxl.dll 会在加载时报缺依赖模块（`jxl_threads.dll` 严格说非必需——当前
解码路径未用 parallel runner，符号表按模块惰性解析，但同批产物一并保留省心）。

## 打包与加载

DLL 是预编译资产、无现场构建步骤（不占 build-tools 段）：架构专用的 Tauri
配置直接映射源码树（`tauri.windows-x64.conf.json` / `tauri.windows-arm64.conf.json`
的 `vendor/libjxl/{x64,arm64} → lib`），安装后与其他随包 DLL 一起落在安装根
`lib/` 子目录。

运行期由 app 侧 `pdf.rs` 的 `JXL_BIND` 按候选路径调
`jpegxl_sys::bind_to_library`：开发期命中本目录、发布期命中安装根 `lib/`。
Windows 侧用 `LOAD_WITH_ALTERED_SEARCH_PATH` 加载，`jxl.dll` 的依赖优先从
`jxl.dll` 自身目录解析——这就是 DLL 放子目录也能找到依赖的原因。

## 版本约束

`jpegxl-sys 0.12.1`（树内）绑定 **libjxl 0.11.2** 的 ABI；vcpkg port 与 brew
当前都是 **0.12.0**。macOS 上 0.12 已实测通过（含 4×4 样本端到端解码测试，
`src-tauri/testdata/sample.jxl`）；Windows 侧同版本产物，ABI 同理兼容，
`.jxl` 预览的最终确认依赖 Windows 真机/CI 回归（test-commands 车道）。

## 升级

换 `vendor/jpegxl-rs` + `vendor/jpegxl-sys` 源码并重放补丁（见 sys 树 README）
+ vcpkg port 升版时重跑 test-libjxl 车道换本目录 DLL + 复核 `testdata` 样本
仍能解码。
