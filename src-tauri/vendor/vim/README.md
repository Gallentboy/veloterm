# vendor/vim — VeloTerm 内嵌的 Vim 源码

上游：https://github.com/vim/vim ，锁定 tag **v9.2.1202**（见 `VELOTERM_VERSION`）。
本目录是整树 vendor（构建资产），除本 README、`VELOTERM_VERSION` 外**零本地修补**——
与其他 vendor 树（jq 九处补丁、bc 的 strgen 复刻）不同，vim 走上游官方
`VIMDLL` 形态即可完成宿主接线，不需要改任何上游源码。

## 构建形态（官方 MSVC nmake 路径，见 src/INSTALLpc.txt）

```
nmake -f Make_mvc.mak VIMDLL=yes CPU=AMD64 vim.exe
```

- `VIMDLL=yes`（上游 8.2.2863 引入的官方拆分）：产出**瘦启动器 `vim.exe`**
  （os_w32exe.c 的 `wmain`，只做 `VimMain(0, NULL)`）+ **`vim64.dll`**
  （主体，`__declspec(dllexport) int VimMain(int argc, char **argv)`，见
  src/main.c）。默认 `FEATURES=HUGE`、静态 CRT（未定义 `USE_MSVCRT` 时 Make_mvc
  自动 `/MT`），依赖全系统库，零外部包。
- VeloTerm 分发：随包只带 `vim64.dll`（coreutils 的 `vim` 分发臂运行时
  LoadLibrary 直调 `VimMain`，见 `crates/coreutils/src/vim_impl.rs`）；启动器
  `vim.exe` 仅作 CI 冒烟 runner，不随包分发。
- GUI/控制台运行期选择：VIMDLL 形态下 DLL 同时编入 GUI 与控制台代码，
  `VimMain` 以 `mch_is_gui_executable()`（os_win32.c）读**宿主进程 exe** 的 PE
  subsystem 决定——宿主 coreutils.exe 是控制台程序，恒为控制台 vim。
- 参数获取：MSWIN 下 `VimMain` **无条件**用 `GetCommandLineW` 自解析进程命令行
  （main.c 调 `get_cmd_argsW`），入口的 argc/argv 被忽略。因此 vim 属 OWN_ARGV
  家族（同 rg/fd/awk/bat）：仅硬链接短名（`tools\vim.exe`）模式可用，
  `coreutils vim ...` 子命令模式给出提示。

## 随包数据文件（runtime）

vim 的运行文件树（syntax/ftplugin/doc 等）随包放安装根 `vim-runtime/`，
调用前由 vim_impl.rs 设 `VIMRUNTIME` 指过去（仅当用户未设；上游搜索序中
用户显式设置优先级更高）。`runtime/doc/tags` 上游仓库自带，无需构建期生成
`:helptags`。`vimrun.exe`（`:!` 命令的辅助器）**不随包分发**：上游按
「exe 同目录是否存在」探测（os_win32.c `s_dont_use_vimrun`），找不到即走
直调 shell 路径，行为完整。

## 裁剪清单（相对上游 tarball）

- `src/testdir/`（18M 测试套件；**仅保留 `Make_all.mak`**——Make_mvc.mak 第 699
  行无条件 `!INCLUDE .\testdir\Make_all.mak`（测试脚本名清单），其余 testdir
  引用都在 test/clean 目标内、构建不触及）、`src/po/`（10M 翻译源，官方
  Windows 包亦不带翻译）、`src/GvimExt/`（资源管理器外壳扩展，需 OLE 构建）
- 顶层 CI/仓库杂项：`.github/`、`ci/`、`READMEdir/`、`pixmaps/`、`nsis/`、
  `lang/`、`tools/`、`configure` 及各 dotfile/贡献指南
- runtime 内 GUI 专属：`bitmaps/`、`icons/`、`hi*.png`、`gvim.desktop`、
  `gvimrc_example.vim`（控制台 vim 不引用）
- 顶层 `README.md` 换为本文件（veloterm 的 vendor 说明；上游内容另有
  `README.txt` 原样保留）

其余全量保留（含 runtime/doc 帮助树、spell、tutor 与 `src/INSTALL/` ——
其中 `INSTALLpc.txt` 是上游 Windows 构建方式的权威文档）。

## 许可

Vim 以 **Vim License**（charityware，见顶层 `LICENSE` 与
`runtime/doc/uganda.txt`）分发：允许自由使用、复制与再分发，要求保留许可
文本（本树原样保留 LICENSE 与 doc/uganda.txt），并鼓励向乌干达的 ICCF 捐助。
该许可与 GPL 兼容（含 GPL 兼容性条款），随包分发无障碍。

## 升级流程

1. 下载新 tag tarball 解压替换本树，重放上方裁剪清单；
2. 更新 `VELOTERM_VERSION`；
3. 复核 `Make_mvc.mak` 的 VIMDLL 机制仍在（`VimMain` dllexport、
   `mch_is_gui_executable`、`get_cmd_argsW` 三处引用点）；
4. 推 `test_vim` tag 走 CI 车道（`.github/workflows/test-vim.yml`）。
