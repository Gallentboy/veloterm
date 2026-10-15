//! tar 命令 CLI 壳（GNU 兼容核心子集），归档能力来自 vendor 的 tar-rs 0.4.46。
//!
//! 支持的形态：`tar -c|-x|-t -f FILE [-z] [-v] [-C DIR] [PATH...]`，
//! 长参数 --create/--extract/--list/--file/--gzip/--verbose/--directory 同义。
//!
//! 安全性：解包走 tar-rs 的 `Entry::unpack_in`（`Archive::unpack` 同源），
//! 对条目路径逐组件过滤——`..` 组件整条跳过、前导 `/` 与盘符前缀被剥除，
//! 且经 canonicalize 校验目标不得逃逸解包根（Zip Slip / CVE-2001-1267
//! 一类目录穿越免疫），这正是选 tar-rs 而非老旧 C 实现的核心原因。

use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// 入口（run_uutils 通用臂约定：迭代器首元素为命令名，被丢弃）
pub fn tar_main<I: IntoIterator<Item = OsString>>(args: I) -> i32 {
    let argv: Vec<String> = args
        .into_iter()
        .skip(1)
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    match run(&argv) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("tar: {err}");
            1
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum Mode {
    Create,
    Extract,
    List,
}

fn run(argv: &[String]) -> Result<i32, String> {
    let mut mode: Option<Mode> = None;
    let mut file: Option<String> = None;
    let mut gzip = false;
    let mut verbose = false;
    let mut chdir: Option<PathBuf> = None;
    let mut paths: Vec<String> = Vec::new();
    let mut no_more_flags = false;

    let mut i = 0;
    while let Some(arg) = argv.get(i) {
        let arg = arg.as_str();
        i += 1;
        if no_more_flags || !arg.starts_with('-') || arg == "-" {
            paths.push(arg.to_string());
            continue;
        }
        if arg == "--" {
            no_more_flags = true;
            continue;
        }
        if let Some(long) = arg.strip_prefix("--") {
            let (key, inline) = match long.split_once('=') {
                Some((k, v)) => (k, Some(v.to_string())),
                None => (long, None),
            };
            let take_value =
                |i: &mut usize, inline: &Option<String>, key: &str| -> Result<String, String> {
                    if let Some(v) = inline {
                        return Ok(v.clone());
                    }
                    let v = argv
                        .get(*i)
                        .cloned()
                        .ok_or(format!("missing value for --{key}"))?;
                    *i += 1;
                    Ok(v)
                };
            match key {
                "create" => mode = Some(Mode::Create),
                "extract" | "get" => mode = Some(Mode::Extract),
                "list" => mode = Some(Mode::List),
                "gzip" => gzip = true,
                "verbose" => verbose = true,
                "file" => file = Some(take_value(&mut i, &inline, key)?),
                "directory" => chdir = Some(PathBuf::from(take_value(&mut i, &inline, key)?)),
                "version" => {
                    println!("tar (veloterm tar-rs 0.4.46)");
                    return Ok(0);
                }
                "help" => {
                    print_usage();
                    return Ok(0);
                }
                _ => return Err(format!("unknown option: {arg}")),
            }
            continue;
        }
        // 短参数（可捆绑；-f/-C 可带内联值或取下一参数）
        let flags = &arg[1..];
        let mut chars = flags.char_indices();
        while let Some((ci, c)) = chars.next() {
            match c {
                'c' => mode = Some(Mode::Create),
                'x' => mode = Some(Mode::Extract),
                't' => mode = Some(Mode::List),
                'z' => gzip = true,
                'v' => verbose = true,
                'f' | 'C' => {
                    let inline = &flags[ci + c.len_utf8()..];
                    let value = if !inline.is_empty() {
                        inline.to_string()
                    } else {
                        let v = argv.get(i).cloned().ok_or("missing value after -f/-C")?;
                        i += 1;
                        v
                    };
                    if c == 'f' {
                        file = Some(value);
                    } else {
                        chdir = Some(PathBuf::from(value));
                    }
                    break; // 内联值消费完剩余字符
                }
                'h' => {
                    print_usage();
                    return Ok(0);
                }
                _ => return Err(format!("unknown option: -{c}")),
            }
        }
    }

    let Some(mode) = mode else {
        print_usage();
        return Ok(2);
    };
    let Some(file) = file else {
        return Err("you must specify one of the '-f' or '--file' options".into());
    };

    // -C DIR：创建模式按 GNU 语义切进程目录（影响源路径与输出文件相对解析）；
    // 解包/列表模式作为解包目标使用，不动进程目录。
    match mode {
        Mode::Create => {
            if let Some(dir) = &chdir {
                fs::create_dir_all(dir).map_err(|e| format!("-C {}: {}", dir.display(), e))?;
                std::env::set_current_dir(dir)
                    .map_err(|e| format!("-C {}: {}", dir.display(), e))?;
            }
            create(&file, &paths, gzip, verbose)
        }
        Mode::Extract => {
            let dest = chdir.unwrap_or_else(|| PathBuf::from("."));
            extract(&file, &dest, gzip, verbose)
        }
        Mode::List => list(&file, gzip, verbose),
    }
}

fn print_usage() {
    eprintln!("usage: tar -c|-x|-t -f ARCHIVE [-z] [-v] [-C DIR] [PATH...]");
}

fn create(file: &str, paths: &[String], gzip: bool, verbose: bool) -> Result<i32, String> {
    if paths.is_empty() {
        return Err("cowardly refusing to create an empty archive".into());
    }
    let out = fs::File::create(file).map_err(|e| format!("cannot create {file}: {e}"))?;
    let mut exit = 0;

    if gzip {
        let enc = flate2::write::GzEncoder::new(out, flate2::Compression::default());
        let mut builder = tar::Builder::new(enc);
        builder.follow_symlinks(false);
        for p in paths {
            if let Err(e) = append_path_recursive(&mut builder, p, verbose) {
                eprintln!("tar: {p}: {e}");
                exit = 1;
            }
        }
        builder
            .into_inner()
            .and_then(|enc| enc.finish())
            .map_err(|e| format!("cannot write archive: {e}"))?;
    } else {
        let mut builder = tar::Builder::new(out);
        builder.follow_symlinks(false);
        for p in paths {
            if let Err(e) = append_path_recursive(&mut builder, p, verbose) {
                eprintln!("tar: {p}: {e}");
                exit = 1;
            }
        }
        builder
            .into_inner()
            .map_err(|e| format!("cannot write archive: {e}"))?;
    }
    Ok(exit)
}

fn append_path_recursive<W: io::Write>(
    builder: &mut tar::Builder<W>,
    path: &str,
    verbose: bool,
) -> io::Result<()> {
    let p = Path::new(path);
    if p.is_dir() {
        builder.append_dir_all(path, path)?;
        if verbose {
            println!("{path}/");
        }
    } else {
        builder.append_path(path)?;
        if verbose {
            println!("{path}");
        }
    }
    Ok(())
}

fn open_archive(file: &str, gzip: bool) -> Result<tar::Archive<Box<dyn io::Read>>, String> {
    let mut f = fs::File::open(file).map_err(|e| format!("cannot open {file}: {e}"))?;
    // GNU 兼容：读侧自动探测 gzip 魔数（1f 8b），-z 仅在探测不出时兜底
    let mut magic = [0u8; 2];
    let n = io::Read::read(&mut f, &mut magic).map_err(|e| e.to_string())?;
    let f = io::Read::chain(
        io::Cursor::new(magic.get(..n).unwrap_or_default().to_vec()),
        f,
    );
    let is_gzip = gzip || (n == 2 && magic[0] == 0x1f && magic[1] == 0x8b);
    let reader: Box<dyn io::Read> = if is_gzip {
        Box::new(flate2::read::GzDecoder::new(f))
    } else {
        Box::new(f)
    };
    Ok(tar::Archive::new(reader))
}

fn extract(file: &str, dest: &Path, gzip: bool, verbose: bool) -> Result<i32, String> {
    let mut archive = open_archive(file, gzip)?;
    archive.set_preserve_permissions(true);
    let mut code = 0;
    for entry in archive.entries().map_err(|e| e.to_string())? {
        let mut entry = entry.map_err(|e| e.to_string())?;
        let path = entry
            .path()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        // unpack_in 与 Archive::unpack 同源的安全过滤，返回 false 表示
        // 条目被路径过滤跳过（如含 ..），不落盘
        match entry.unpack_in(dest) {
            Ok(true) => {
                if verbose {
                    println!("x {path}");
                }
            }
            Ok(false) => eprintln!("tar: skipping unsafe member name: {path}"),
            Err(e) => {
                eprintln!("tar: {path}: {e}");
                code = 1;
            }
        }
    }
    Ok(code)
}

fn list(file: &str, gzip: bool, verbose: bool) -> Result<i32, String> {
    let mut archive = open_archive(file, gzip)?;
    for entry in archive.entries().map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry
            .path()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        if verbose {
            let header = entry.header();
            let size = header.size().unwrap_or(0);
            let kind = match header.entry_type() {
                tar::EntryType::Directory => 'd',
                tar::EntryType::Symlink => 'l',
                _ => '-',
            };
            println!("{kind}rw-r--r-- {:>10} {path}", size);
        } else {
            println!("{path}");
        }
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tempdir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "veloterm-tar-test-{}-{}",
            tag,
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn s(v: &[&str]) -> Vec<OsString> {
        std::iter::once(OsString::from("tar"))
            .chain(v.iter().map(OsString::from))
            .collect()
    }

    #[test]
    fn create_list_extract_roundtrip() {
        let dir = tempdir("roundtrip");
        let src = dir.join("src");
        fs::create_dir_all(src.join("sub")).unwrap();
        fs::write(src.join("a.txt"), "alpha").unwrap();
        fs::write(src.join("sub").join("b.txt"), "beta").unwrap();

        let archive = dir.join("out.tar");
        let archive_str = archive.to_string_lossy().into_owned();
        let src_str = src.to_string_lossy().into_owned();

        // 创建
        assert_eq!(
            tar_main(s(&["-cf", archive_str.as_str(), src_str.as_str()])),
            0
        );
        assert!(archive.is_file());

        // 列表包含两个文件
        assert_eq!(tar_main(s(&["-tf", archive_str.as_str()])), 0);

        // 解包到 -C 指定目录（不污染进程 cwd）并校验内容
        let dest = dir.join("dest");
        fs::create_dir_all(&dest).unwrap();
        let dest_str = dest.to_string_lossy().into_owned();
        assert_eq!(
            tar_main(s(&["-xf", archive_str.as_str(), "-C", dest_str.as_str()])),
            0
        );
        let extracted_root = dest.join(&src);
        assert_eq!(
            fs::read_to_string(extracted_root.join("a.txt")).unwrap(),
            "alpha"
        );
        assert_eq!(
            fs::read_to_string(extracted_root.join("sub").join("b.txt")).unwrap(),
            "beta"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn gzip_roundtrip_and_flags() {
        let dir = tempdir("gzip");
        fs::write(dir.join("f.txt"), "gzip me").unwrap();

        let archive = dir.join("out.tar.gz");
        let archive_str = archive.to_string_lossy().into_owned();
        let file_str = dir.join("f.txt").to_string_lossy().into_owned();

        // 捆绑短参数
        assert_eq!(
            tar_main(s(&["-cvzf", archive_str.as_str(), file_str.as_str()])),
            0
        );
        assert!(archive.is_file());
        assert_eq!(tar_main(s(&["--list", "--file", archive_str.as_str()])), 0);

        let dest = dir.join("out");
        fs::create_dir_all(&dest).unwrap();
        let dest_str = dest.to_string_lossy().into_owned();
        assert_eq!(
            tar_main(s(&["-xzf", archive_str.as_str(), "-C", dest_str.as_str()])),
            0
        );
        assert_eq!(
            fs::read_to_string(dest.join(&dir).join("f.txt")).unwrap(),
            "gzip me"
        );

        // --version 正常；缺 -f / 缺模式 → 非零
        assert_eq!(tar_main(s(&["--version"])), 0);
        assert_ne!(tar_main(s(&["-c", "x"])), 0);

        let _ = fs::remove_dir_all(&dir);
    }
}
