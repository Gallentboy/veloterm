# vendor/libjxl — Windows 端 libjxl 动态库（随包分发）

JPEG XL 解码（图片预览的 `.jxl` 格式）通过 `jpegxl-rs` 绑定 libjxl。
macOS 走 Homebrew 安装的系统库（`brew install jpeg-xl`）；Windows 没有系统级
分发渠道，需要把 DLL 放进本目录随安装包分发。

## 需要放入的文件

把 libjxl 的 Windows 构建产物复制到本目录，**按架构命名**（与
`src-tauri/vendor/pdfium/` 的命名约定一致）：

| 目标平台 | 文件名 |
| --- | --- |
| Windows x64 | `jxl-x64.dll` |
| Windows ARM64 | `jxl-arm64.dll` |

> 实际文件名以 libjxl 官方 Windows 发布包里的 DLL 名为准（通常为 `jxl.dll`，
> 可能还依赖 `jxl_cms.dll` / `libjxl_threads.dll`）。如果发布包是多个 DLL，
> 全部按上面的架构后缀命名放进来即可，构建脚本会整目录拷贝。

## 打包方式

构建脚本（`scripts/build-tools.mjs`）按 target 把对应架构的 DLL 复制到
`src-tauri/binaries/libjxl/`，再由架构专用的 Tauri 配置映射进安装包
（`tauri.windows-x64.conf.json` / `tauri.windows-arm64.conf.json`）。

运行期由 `jpegxl-rs` 自己按 DLL 搜索路径加载——**DLL 需放在宿主 exe
（`coreutils.exe`）同目录或同目录的 `libjxl/` 子目录**，与 PDFium 的
`lib/`、`pdfium/` 布局约定一致。

## 版本约束

`jpegxl-rs 0.13.1` 绑定 **libjxl 0.11.2** 的 ABI。实测 macOS 上 libjxl 0.12
也能正常加载（ABI 兼容），但 Windows 建议使用 **0.11.x** 系列以降低风险。

## 当前状态

- macOS：已可用（依赖 `brew install jpeg-xl`，不走本目录）
- Windows：**待放入 DLL**。未放入时 `.jxl` 预览会报「初始化 JXL 解码器失败」
