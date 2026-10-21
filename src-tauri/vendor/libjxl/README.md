# vendor/libjxl — Windows 端 libjxl 动态库（随包分发）

JPEG XL 解码（图片预览的 `.jxl` 格式）通过 `jpegxl-rs` 绑定 libjxl。
macOS 走 Homebrew 安装的系统库（`brew install jpeg-xl`）；Windows 没有系统级
分发渠道，需要把 DLL 放进本目录随安装包分发。

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
├── x64/      ← 解压 libjxl-x64.zip 的全部内容
├── arm64/    ← 解压 libjxl-arm64.zip 的全部内容
└── README.md
```

**为什么必须全部 DLL**：libjxl 本体依赖 `highway` / `brotli` / `lcms` 的运行时
DLL，只拷 `jxl.dll` 会导致宿主加载失败（缺依赖模块）。

## 打包方式

构建脚本（`scripts/build-tools.mjs`）按 target 把对应架构子目录的全部 DLL
复制到 `src-tauri/binaries/libjxl/`，再由架构专用的 Tauri 配置映射进安装包
（`tauri.windows-x64.conf.json` / `tauri.windows-arm64.conf.json`）。

运行期由 `jpegxl-rs` 自己按 DLL 搜索路径加载——**DLL 需放在宿主 exe
（`coreutils.exe`）同目录或同目录的 `libjxl/` 子目录**，与 PDFium 的
`lib/`、`pdfium/` 布局约定一致。

## 版本约束

`jpegxl-rs 0.13.1` 绑定 **libjxl 0.11.2** 的 ABI；vcpkg 当前 port 是 **0.12.0**。
实测 macOS 上 0.12 能正常加载（ABI 兼容），Windows 亦按此假设，车道跑通后
需在真机/CI 验证 `.jxl` 预览可用。

## 当前状态

- macOS：已可用（依赖 `brew install jpeg-xl`，不走本目录）
- Windows：**待放入 DLL**。未放入时 `.jxl` 预览会报「初始化 JXL 解码器失败」
