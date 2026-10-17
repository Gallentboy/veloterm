//! FFI DLL 的定位约定（跨平台编译，macOS 单测覆盖）。
//!
//! 三个 DLL 例外形态（Go 的 veloterm_goffi.dll、libqalculate 的 qalcffi.dll、
//! nmap 的 nmapffi.dll）都随安装包分发在安装根的 `lib/` 子目录下，运行期由
//! 各自的 `*_impl.rs` 经本模块解析候选路径后 LoadLibrary。
//! 附带 `flat_dir_candidates`：平铺在安装根的数据目录（nmap-data/、
//! vim-runtime/ 等，不随 DLL 进 lib/）的候选生成——与 DLL 候选同一套
//! 双启动形态推导，供 *_impl.rs 的数据目录兑底使用。
//!
//! # 为什么是 lib/ 而不是安装根
//!
//! 安装根只保留运行必需的可执行体（`VeloTerm.exe`、`coreutils.exe`）与
//! `tools/` 硬链接目录，几十 MB 的第三方 DLL 集中到 `lib/` 便于识别与清理。
//! 打包侧映射见 `src-tauri/tauri.windows.conf.json` 的 `bundle.resources`。
//!
//! # 候选顺序与两种启动形态
//!
//! 安装后的目录形状：
//!
//! ```text
//! <安装根>\
//!   VeloTerm.exe
//!   coreutils.exe
//!   lib\veloterm_goffi.dll / qalcffi.dll / nmapffi.dll
//!   nmap-data\
//!   tools\yq.exe …（coreutils.exe 的 NTFS 硬链接短名）
//! ```
//!
//! 两种启动形态的 exe 目录不同，故需要两条 `lib/` 候选：
//! - `tools\yq.exe` 启动 → exe 目录是 `tools\`，DLL 在 **上一级** 的 `lib\`
//!   （`dir/../lib`）；
//! - `coreutils.exe yq …` 直跑 → exe 目录就是安装根，DLL 在 **本级** 的 `lib\`
//!   （`dir/lib`）。
//!
//! 候选按「先 lib/、后平铺」排列，理由是**防止旧版残留遮蔽新版**：从旧版本
//! 升级而来的机器上，安装根可能还留着上一版平铺的 `x.dll`（NSIS 卸载清单已
//! 不含它，不会自动清理）。若把平铺候选排在前面，新装的 `lib/x.dll` 会被那
//! 个陈旧文件永久遮蔽。
//!
//! 平铺候选（`dir/x.dll`）作为末位兜底保留，服务开发机手工放置 DLL 的场景
//! （`target/debug/` 旁直接放一份即可调试），不影响安装后的解析优先级。

use std::path::{Path, PathBuf};

/// 生成某个 DLL 名的候选路径（不含存在性判断，纯函数）。
///
/// 顺序即优先级（调用方逐个尝试 LoadLibrary）：
/// 1. `<exe_dir>/lib/<name>`    —— 安装根直跑（coreutils.exe 子命令模式）
/// 2. `<exe_dir>/../lib/<name>` —— tools\ 短名硬链接启动
/// 3. `<exe_dir>/<name>`        —— 开发机手工放置 / 平铺兜底
///
/// `exe` 为当前进程的可执行文件路径；不可得（`None`）或无父目录时返回空候选
/// （调用方据此给出「无法定位当前进程路径」的诊断）。
// 调用方全部在 cfg(windows) 的 *_impl 里，非 Windows 构建中仅测试消费——
// 允许 dead_code（flat_dir_candidates 同）。
#[cfg_attr(not(windows), allow(dead_code))]
pub fn dll_candidates(exe: Option<&Path>, dll_name: &str) -> Vec<PathBuf> {
    let Some(dir) = exe.and_then(|p| p.parent()) else {
        return Vec::new();
    };
    let mut v = vec![dir.join("lib").join(dll_name)];
    if let Some(parent) = dir.parent() {
        v.push(parent.join("lib").join(dll_name));
    }
    v.push(dir.join(dll_name));
    v
}

/// 生成平铺数据目录名的候选路径（不含存在性判断，纯函数）。
///
/// 数据目录（nmap-data/、vim-runtime/）不随 DLL 进 `lib/`，仍平铺在安装根，
/// 故候选不含 lib/ 变体：
/// 1. `<exe_dir>/<name>`    —— coreutils.exe 直跑（exe 目录即安装根）
/// 2. `<exe_dir>/../<name>` —— tools\ 短名硬链接启动（上级即安装根）
#[cfg_attr(not(windows), allow(dead_code))]
pub fn flat_dir_candidates(exe: Option<&Path>, dir_name: &str) -> Vec<PathBuf> {
    let Some(dir) = exe.and_then(|p| p.parent()) else {
        return Vec::new();
    };
    let mut v = vec![dir.join(dir_name)];
    if let Some(parent) = dir.parent() {
        v.push(parent.join(dir_name));
    }
    v
}

#[cfg(test)]
mod tests {
    use super::{dll_candidates, flat_dir_candidates};
    use std::path::{Path, PathBuf};

    // 本模块用平台原生的 `Path::parent()`/`join()` 解析，故跨平台用例一律用
    // Unix 风格路径（`/` 分隔）——它们在 macOS/Windows 上语义一致，能被本机
    // `cargo test` 真正执行（Windows 车道不跑 cargo test，若只写 Windows 盘符
    // 形态，这些断言在任何 CI 上都不会运行）。盘符路径另有一条
    // `#[cfg(windows)]` 用例记录形态，供真机调试对照。

    /// 安装根直跑：本级 `lib/` 命中，且排在平铺候选之前
    #[test]
    fn install_root_prefers_own_lib_subdir() {
        let exe = Path::new("/opt/veloterm/coreutils.exe");
        let c = dll_candidates(Some(exe), "qalcffi.dll");
        assert_eq!(
            c,
            vec![
                PathBuf::from("/opt/veloterm/lib/qalcffi.dll"),
                PathBuf::from("/opt/lib/qalcffi.dll"),
                PathBuf::from("/opt/veloterm/qalcffi.dll"),
            ]
        );
    }

    /// tools\ 短名启动：上一级 `lib/` 是第二个候选（首个本级 lib/ 不存在）
    #[test]
    fn shortname_layout_reaches_parent_lib() {
        let exe = Path::new("/opt/veloterm/tools/yq.exe");
        let c = dll_candidates(Some(exe), "veloterm_goffi.dll");
        assert_eq!(c[0], PathBuf::from("/opt/veloterm/tools/lib/veloterm_goffi.dll"));
        assert_eq!(c[1], PathBuf::from("/opt/veloterm/lib/veloterm_goffi.dll"));
        assert_eq!(c[2], PathBuf::from("/opt/veloterm/tools/veloterm_goffi.dll"));
    }

    /// 升级残留防护（本模块的关键不变量）：旧版平铺在安装根的 DLL 必须排在
    /// `lib/` 之后，否则陈旧文件会永久遮蔽新装的 DLL。
    #[test]
    fn flat_legacy_copy_never_shadows_lib() {
        let exe = Path::new("/opt/veloterm/coreutils.exe");
        let c = dll_candidates(Some(exe), "veloterm_goffi.dll");
        let lib_pos = c
            .iter()
            .position(|p| p == Path::new("/opt/veloterm/lib/veloterm_goffi.dll"))
            .expect("lib/ candidate must exist");
        let flat_pos = c
            .iter()
            .position(|p| p == Path::new("/opt/veloterm/veloterm_goffi.dll"))
            .expect("flat fallback candidate must exist");
        assert!(lib_pos < flat_pos, "lib/ must be tried before the flat legacy copy");
    }

    /// 每个 DLL 名独立生效（三个 impl 共用本函数，名不同候选不同）
    #[test]
    fn candidates_use_the_given_dll_name() {
        let exe = Path::new("/opt/veloterm/tools/nmap.exe");
        let c = dll_candidates(Some(exe), "nmapffi.dll");
        assert!(c.iter().all(|p| p.ends_with("nmapffi.dll")));
    }

    /// 进程路径不可得时不 panic，返回空候选（调用方给出可诊断提示）
    #[test]
    fn empty_without_exe() {
        assert!(dll_candidates(None, "qalcffi.dll").is_empty());
    }

    /// 裸文件名（无目录分量）：`parent()` 给空串，只剩同目录候选。
    /// 真实调用不会走到（`current_exe()` 恒为绝对路径），此用例锁定
    /// 「不 panic、不臆造路径」的行为。
    #[test]
    fn bare_filename_yields_only_the_sibling_candidate() {
        let c = dll_candidates(Some(Path::new("qalc.exe")), "qalcffi.dll");
        assert_eq!(c, vec![PathBuf::from("lib/qalcffi.dll"), PathBuf::from("qalcffi.dll")]);
    }

    /// 平铺数据目录：coreutils.exe 直跑 → 本级命中；tools\ 短名 → 上级命中
    #[test]
    fn flat_dir_candidates_cover_both_launch_shapes() {
        let exe = Path::new("/opt/veloterm/coreutils.exe");
        assert_eq!(
            flat_dir_candidates(Some(exe), "vim-runtime"),
            vec![
                PathBuf::from("/opt/veloterm/vim-runtime"),
                PathBuf::from("/opt/vim-runtime"),
            ]
        );
        let short = Path::new("/opt/veloterm/tools/vim.exe");
        assert_eq!(
            flat_dir_candidates(Some(short), "nmap-data"),
            vec![
                PathBuf::from("/opt/veloterm/tools/nmap-data"),
                PathBuf::from("/opt/veloterm/nmap-data"),
            ]
        );
    }

    /// 平铺数据目录：进程路径不可得时不 panic，返回空候选
    #[test]
    fn flat_dir_candidates_empty_without_exe() {
        assert!(flat_dir_candidates(None, "vim-runtime").is_empty());
    }

    /// Windows 盘符路径形态（含反斜杠）按同样顺序展开；该平台不跑 cargo test，
    /// 此用例作为形态记录与真机调试时的对照。
    #[test]
    #[cfg(windows)]
    fn windows_drive_paths_resolve() {
        let exe = Path::new(r"C:\app\tools\yq.exe");
        let c = dll_candidates(Some(exe), "veloterm_goffi.dll");
        assert_eq!(
            c,
            vec![
                PathBuf::from(r"C:\app\tools\lib\veloterm_goffi.dll"),
                PathBuf::from(r"C:\app\lib\veloterm_goffi.dll"),
                PathBuf::from(r"C:\app\tools\veloterm_goffi.dll"),
            ]
        );
    }
}
