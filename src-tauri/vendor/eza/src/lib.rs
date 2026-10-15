// SPDX-FileCopyrightText: 2024 Christina Sørensen
// SPDX-License-Identifier: EUPL-1.2
//
// SPDX-FileCopyrightText: 2023-2024 Christina Sørensen, eza contributors
// SPDX-FileCopyrightText: 2014 Benjamin Sago
// SPDX-License-Identifier: MIT
#![warn(future_incompatible)]
#![warn(trivial_casts, trivial_numeric_casts)]
#![warn(clippy::all)]
#![allow(clippy::non_ascii_literal)]

/*!
The library entry point into eza.

VeloTerm vendor 新增：本文件在上游原有的 `pub mod` 声明基础上，把原
`src/main.rs` 的 main() 主逻辑搬到这里，使 eza 可以作为 `eza` / `tree`
命令内嵌进 VeloTerm `coreutils` 多呼叫二进制，通过 `eza::eza_main()` /
`eza::eza_main_from()` 调用。`main.rs` 保留为独立 eza 二进制的薄壳入口。
*/

use std::env;
use std::ffi::{OsStr, OsString};
use std::io::{self, ErrorKind, IsTerminal, Read, Write, stdin};
use std::path::{Component, PathBuf};

use nu_ansi_term::{AnsiStrings as ANSIStrings, Style};
use options::parser::get_command;

use crate::fs::feature::git::GitCache;
use crate::fs::filter::{FileFilterFlags::OnlyFiles, GitIgnore};
use crate::fs::{Dir, File};
use crate::options::stdin::FilesInput;
use crate::options::{Options, Vars, vars};
use crate::output::{Mode, View, code, details, escape, file_name, grid, grid_details, lines};
use crate::theme::Theme;
use log::*;

#[allow(unused)]
pub mod fs;
#[allow(unused)]
pub mod info;
#[allow(unused)]
pub mod loc;
#[allow(unused)]
pub mod logger;
#[allow(unused)]
pub mod options;
#[allow(unused)]
pub mod output;
#[allow(unused)]
pub mod theme;

/// VeloTerm coreutils 入口：以真实进程 argv 执行 eza 并返回进程退出码。
///
/// 注意：本函数直接读取真实进程 argv（argv[0] 作为程序名被 clap 跳过），
/// 适合「以 eza 短名（硬链接）启动」的场景；`coreutils eza ...` 子命令
/// 模式下请改用可注入的 [`eza_main_from`]。
pub fn eza_main() -> i32 {
    eza_main_from(env::args_os())
}

/// VeloTerm coreutils 可注入入口：以 `args`（首元素为程序名，将被 clap
/// 作为 bin name 跳过）执行 eza 并返回进程退出码。
///
/// `coreutils eza <args>...` 子命令模式下可传入
/// `std::iter::once(OsString::from("eza")).chain(rest_args)`；
/// `coreutils tree <args>...` 则注入 `["tree", "--tree"]` 前缀参数。
/// clap 自身的 usage 错误在此打印并返回退出码 2，`--help`/`--version`
/// 打印后返回 0（不会调用 `process::exit`，宿主进程不受影响）。
pub fn eza_main_from<I, T>(args: I) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    logger::configure(env::var_os(vars::EZA_DEBUG).or_else(|| env::var_os(vars::EXA_DEBUG)));

    // VeloTerm vendor 修改：原 `get_command().get_matches()` 改为
    // `try_get_matches_from(args)`，使参数可注入且错误不会被 clap 直接
    // `process::exit`（嵌入式场景不能杀死宿主进程）。
    let cli = match get_command().try_get_matches_from(args) {
        Ok(cli) => cli,
        Err(e) => {
            let _ = e.print();
            return i32::from(e.exit_code());
        }
    };

    let stdout_istty = io::stdout().is_terminal();
    let mut input = String::new();
    let mut input_paths: Vec<&OsStr> = match cli.get_many("FILE") {
        Some(x) => x.map(OsString::as_os_str).collect(),
        None => vec![],
    };
    match Options::deduce(&cli, &LiveVars) {
        Ok(options) => {
            if input_paths.is_empty() {
                match &options.stdin {
                    FilesInput::Args => {
                        input_paths = vec![OsStr::new(".")];
                    }
                    FilesInput::Stdin(separator) => {
                        stdin()
                            .read_to_string(&mut input)
                            .expect("Failed to read from stdin");
                        input_paths.extend(
                            input
                                .split(&separator.clone().into_string().unwrap_or("\n".to_string()))
                                .map(OsStr::new)
                                .filter(|s| !s.is_empty())
                                .collect::<Vec<_>>(),
                        );
                    }
                }
            }

            let git = git_options(&options, &input_paths);
            let writer = io::stdout();
            let git_repos = git_repos(&options, &input_paths);

            let console_width = options.view.width.actual_terminal_width();
            let theme = options.theme.to_theme(stdout_istty);
            let exa = Exa {
                options,
                writer,
                input_paths,
                theme,
                console_width,
                git,
                git_repos,
            };

            info!("matching on exa.run");
            match exa.run() {
                Ok(exit_status) => {
                    trace!("exa.run: exit Ok({exit_status})");
                    // VeloTerm vendor 修改：原 `exit(exit_status)` 改为直接
                    // 返回退出码（Exa::run 返回的已是 i32）。
                    exit_status
                }

                Err(e) if e.kind() == ErrorKind::BrokenPipe => {
                    warn!("Broken pipe error: {e}");
                    exits::SUCCESS
                }

                Err(e) => {
                    eprintln!("{e}");
                    trace!("exa.run: exit RUNTIME_ERROR");
                    exits::RUNTIME_ERROR
                }
            }
        }
        Err(error) => {
            eprintln!("eza: {error}");
            exits::OPTIONS_ERROR
        }
    }
}

/// The main program wrapper.
pub struct Exa<'args> {
    /// List of command-line options, having been successfully parsed.
    pub options: Options,

    /// The output handle that we write to.
    pub writer: io::Stdout,

    /// List of the free command-line arguments that should correspond to file
    /// names (anything that isn’t an option).
    pub input_paths: Vec<&'args OsStr>,

    /// The theme that has been configured from the command-line options and
    /// environment variables. If colours are disabled, this is a theme with
    /// every style set to the default.
    pub theme: Theme,

    /// The detected width of the console. This is used to determine which
    /// view to use.
    pub console_width: Option<usize>,

    /// A global Git cache, if the option was passed in.
    /// This has to last the lifetime of the program, because the user might
    /// want to list several directories in the same repository.
    pub git: Option<GitCache>,

    pub git_repos: bool,
}

/// The “real” environment variables type.
/// Instead of just calling `var_os` from within the options module,
/// the method of looking up environment variables has to be passed in.
struct LiveVars;
impl Vars for LiveVars {
    fn get(&self, name: &'static str) -> Option<OsString> {
        env::var_os(name)
    }
}

/// Create a Git cache populated with the arguments that are going to be
/// listed before they’re actually listed, if the options demand it.
fn git_options(options: &Options, args: &[&OsStr]) -> Option<GitCache> {
    if options.should_scan_for_git() {
        Some(args.iter().map(PathBuf::from).collect())
    } else {
        None
    }
}

#[cfg(not(feature = "git"))]
fn git_repos(_options: &Options, _args: &[&OsStr]) -> bool {
    return false;
}

#[cfg(feature = "git")]
fn get_files_in_dir(paths: &mut Vec<PathBuf>, path: PathBuf) {
    let temp_paths = if path.is_dir() {
        match path.read_dir() {
            Err(_) => {
                vec![path]
            }
            Ok(d) => d
                .filter_map(|entry| entry.ok().map(|e| e.path()))
                .collect::<Vec<PathBuf>>(),
        }
    } else {
        vec![path]
    };
    paths.extend(temp_paths);
}

#[cfg(feature = "git")]
fn git_repos(options: &Options, args: &[&OsStr]) -> bool {
    let option_enabled = match options.view.mode {
        Mode::Details(details::Options {
            table: Some(ref table),
            ..
        })
        | Mode::GridDetails(grid_details::Options {
            details:
                details::Options {
                    table: Some(ref table),
                    ..
                },
            ..
        }) => table.columns.subdir_git_repos || table.columns.subdir_git_repos_no_stat,
        _ => false,
    };
    if option_enabled {
        let paths: Vec<PathBuf> = args.iter().map(PathBuf::from).collect::<Vec<PathBuf>>();
        let mut files: Vec<PathBuf> = Vec::new();
        for path in paths {
            get_files_in_dir(&mut files, path);
        }
        let repos: Vec<bool> = files
            .iter()
            .map(git2::Repository::open)
            .map(|repo| repo.is_ok())
            .collect();
        repos.contains(&true)
    } else {
        false
    }
}

impl Exa<'_> {
    /// # Errors
    ///
    /// Will return `Err` if printing to stderr fails.
    pub fn run(mut self) -> io::Result<i32> {
        debug!("Running with options: {:#?}", self.options);

        // The `--code` summary doesn’t list files: it walks the given paths and
        // prints a per-language lines-of-code breakdown, so handle it up front.
        if let Mode::Code(opts) = &self.options.view.mode {
            let opts = *opts;
            let mut exit_status = 0;
            let mut roots = Vec::new();

            // Report paths that don’t exist, like the normal listing does,
            // and count the rest.
            for file_path in &self.input_paths {
                let path = PathBuf::from(file_path);
                if let Err(e) = std::fs::symlink_metadata(&path) {
                    exit_status = 2;
                    writeln!(io::stderr(), "{file_path:?}: {e}")?;
                } else {
                    roots.push(path);
                }
            }
            let file_style = &self.options.view.file_style;
            let show_icons = match file_style.show_icons {
                file_name::ShowIcons::Always(_) => true,
                file_name::ShowIcons::Automatic(_) => file_style.is_a_tty,
                file_name::ShowIcons::Never => false,
            };
            if roots.is_empty() {
                return Ok(exit_status);
            }

            let r = code::Render {
                theme: &self.theme,
                opts: &opts,
                roots,
                show_icons,
            };
            r.render(&mut self.writer)?;
            return Ok(exit_status);
        }

        let mut files = Vec::new();
        let mut dirs = Vec::new();
        let mut exit_status = 0;

        for file_path in &self.input_paths {
            let f = File::from_args(
                PathBuf::from(file_path),
                None,
                None,
                self.options.view.deref_links,
                self.options.view.total_size,
                None,
            );

            // We don't know whether this file exists, so we have to try to get
            // the metadata to verify.
            if let Err(e) = f.metadata() {
                exit_status = 2;
                writeln!(io::stderr(), "{file_path:?}: {e}")?;
                continue;
            }

            if f.points_to_directory() && !self.options.dir_action.treat_dirs_as_files() {
                trace!("matching on new Dir");
                dirs.push(f.to_dir());
            } else {
                files.push(f);
            }
        }

        // We want to print a directory’s name before we list it, *except* in
        // the case where it’s the only directory, *except* if there are any
        // files to print as well. (It’s a double negative)

        let no_files = files.is_empty();
        let is_only_dir = dirs.len() == 1 && no_files;

        self.options.filter.filter_argument_files(&mut files);
        self.print_files(None, files)?;

        self.print_dirs(dirs, no_files, is_only_dir, exit_status)
    }

    fn print_dirs(
        &mut self,
        dir_files: Vec<Dir>,
        mut first: bool,
        is_only_dir: bool,
        exit_status: i32,
    ) -> io::Result<i32> {
        let View {
            file_style: file_name::Options { quote_style, .. },
            ..
        } = self.options.view;

        let mut denied_dirs = vec![];

        for mut dir in dir_files {
            let dir = match dir.read() {
                Ok(dir) => dir,
                Err(e) => {
                    if e.kind() == ErrorKind::PermissionDenied {
                        eprintln!(
                            "Permission denied: {} - code: {}",
                            dir.path.display(),
                            exits::PERMISSION_DENIED
                        );
                        denied_dirs.push(dir.path);
                        continue;
                    }

                    eprintln!("{}: {}", dir.path.display(), e);
                    continue;
                }
            };

            // Put a gap between directories, or between the list of files and
            // the first directory.
            if first {
                first = false;
            } else {
                writeln!(&mut self.writer)?;
            }

            if !is_only_dir {
                let mut bits = Vec::new();
                escape(
                    dir.path.display().to_string(),
                    &mut bits,
                    Style::default(),
                    Style::default(),
                    quote_style,
                );
                writeln!(&mut self.writer, "{}:", ANSIStrings(&bits))?;
            }

            let mut children = Vec::new();
            let git_ignore = self.options.filter.git_ignore == GitIgnore::CheckAndIgnore;
            for file in dir.files(
                self.options.filter.dot_filter,
                self.git.as_ref(),
                git_ignore,
                self.options.view.deref_links,
                self.options.view.total_size,
            ) {
                children.push(file);
            }
            let recursing = self.options.dir_action.recurse_options().is_some();
            self.options
                .filter
                .filter_child_files(recursing, &mut children);
            self.options.filter.sort_files(&mut children);

            if let Some(recurse_opts) = self.options.dir_action.recurse_options() {
                let depth = dir
                    .path
                    .components()
                    .filter(|&c| c != Component::CurDir)
                    .count()
                    + 1;
                let follow_links = self.options.view.follow_links;
                if !recurse_opts.tree && !recurse_opts.is_too_deep(depth) {
                    let child_dirs = children
                        .iter()
                        .filter(|f| {
                            (if follow_links {
                                f.points_to_directory()
                            } else {
                                f.is_directory()
                            }) && !f.is_all_all
                        })
                        .map(fs::File::to_dir)
                        .collect::<Vec<Dir>>();

                    self.print_files(Some(dir), children)?;
                    match self.print_dirs(child_dirs, false, false, exit_status) {
                        Ok(_) => (),
                        Err(e) => return Err(e),
                    }
                    continue;
                }
            }

            self.print_files(Some(dir), children)?;
        }

        if !denied_dirs.is_empty() {
            eprintln!(
                "\nSkipped {} directories due to permission denied: ",
                denied_dirs.len()
            );
            for path in denied_dirs {
                eprintln!("  {}", path.display());
            }
        }

        Ok(exit_status)
    }

    /// Prints the list of files using whichever view is selected.
    fn print_files(&mut self, dir: Option<&Dir>, mut files: Vec<File<'_>>) -> io::Result<()> {
        if files.is_empty() {
            return Ok(());
        }
        let recursing = self.options.dir_action.recurse_options().is_some();
        let only_files = self.options.filter.flags.contains(&OnlyFiles);
        if recursing && only_files {
            files = files
                .into_iter()
                .filter(|f| !f.is_directory())
                .collect::<Vec<_>>();
        }
        let theme = &self.theme;
        let View {
            ref mode,
            ref file_style,
            ..
        } = self.options.view;

        match (mode, self.console_width) {
            (Mode::Grid(opts), Some(console_width)) => {
                let filter = &self.options.filter;
                let r = grid::Render {
                    files,
                    theme,
                    file_style,
                    opts,
                    console_width,
                    filter,
                };
                r.render(&mut self.writer)
            }

            (Mode::Grid(opts), None) => {
                let filter = &self.options.filter;
                let r = grid::Render {
                    files,
                    theme,
                    file_style,
                    opts,
                    console_width: 80,
                    filter,
                };
                r.render(&mut self.writer)
            }

            (Mode::Lines, _) => {
                let filter = &self.options.filter;
                let r = lines::Render {
                    files,
                    theme,
                    file_style,
                    filter,
                };
                r.render(&mut self.writer)
            }

            (Mode::Details(opts), _) => {
                let filter = &self.options.filter;
                let recurse = self.options.dir_action.recurse_options();

                let git_ignoring = self.options.filter.git_ignore == GitIgnore::CheckAndIgnore;
                let git = self.git.as_ref();
                let git_repos = self.git_repos;
                let r = details::Render {
                    dir,
                    files,
                    theme,
                    file_style,
                    opts,
                    recurse,
                    filter,
                    git_ignoring,
                    git,
                    git_repos,
                };
                r.render(&mut self.writer)
            }

            (Mode::GridDetails(opts), Some(console_width)) => {
                let details = &opts.details;
                let row_threshold = opts.row_threshold;

                let filter = &self.options.filter;
                let git_ignoring = self.options.filter.git_ignore == GitIgnore::CheckAndIgnore;
                let git = self.git.as_ref();
                let git_repos = self.git_repos;

                let r = grid_details::Render {
                    dir,
                    files,
                    theme,
                    file_style,
                    details,
                    filter,
                    row_threshold,
                    git_ignoring,
                    git,
                    console_width,
                    git_repos,
                };
                r.render(&mut self.writer)
            }

            (Mode::GridDetails(opts), None) => {
                let opts = &opts.to_details_options();
                let filter = &self.options.filter;
                let recurse = self.options.dir_action.recurse_options();
                let git_ignoring = self.options.filter.git_ignore == GitIgnore::CheckAndIgnore;
                let git = self.git.as_ref();
                let git_repos = self.git_repos;

                let r = details::Render {
                    dir,
                    files,
                    theme,
                    file_style,
                    opts,
                    recurse,
                    filter,
                    git_ignoring,
                    git,
                    git_repos,
                };
                r.render(&mut self.writer)
            }

            // The code summary never lists files; it’s handled up front in
            // `run` before we ever get here.
            (Mode::Code(_), _) => unreachable!("--code is handled in Exa::run"),
        }
    }
}

mod exits {

    /// Exit code for when exa runs OK.
    pub const SUCCESS: i32 = 0;

    /// Exit code for when there was at least one I/O error during execution.
    pub const RUNTIME_ERROR: i32 = 1;

    /// Exit code for when the command-line options are invalid.
    pub const OPTIONS_ERROR: i32 = 3;

    /// Exit code for missing file permissions
    pub const PERMISSION_DENIED: i32 = 13;
}
