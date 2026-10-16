//! This build script gets run during every build. Its purpose is to put
//! together the files used for the `--help` and `--version`, which need to
//! come in both coloured and non-coloured variants. The main usage text is
//! contained in `src/usage.txt`; to make it easier to edit, backslashes (\)
//! are used instead of the beginning of ANSI escape codes.
//!
//! VeloTerm vendor 修改：上游对 debug / pre-release 构建会在版本串里嵌入
//! `git rev-parse --short HEAD` 的哈希与编译日期（经 build-dependency
//! `datetime`）——作为路径依赖进 veloterm 工作区后，这既破坏密封构建
//! （git 缺失时 `unwrap()` 直接令构建失败，取到的也是外层 veloterm 仓库
//! 的哈希而非 dog 自己的），对内嵌分发也无意义。已移除 datetime 依赖与
//! git/日期分支（连同 [build-dependencies]），所有构建统一走上游 release
//! 形态的版本串（版本号 + 非默认特性集）；usage 文本的处理保持原样。

use std::env;
use std::fs::File;
use std::io::{self, Write};
use std::path::PathBuf;


/// The build script entry point.
fn main() -> io::Result<()> {
    #![allow(clippy::write_with_newline)]

    let usage   = include_str!("src/usage.txt");
    let tagline = "dog \\1;32m●\\0m command-line DNS client";
    let url     = "https://dns.lookup.dog/";

    let ver = format!("{}\nv{}\n\\1;4;34m{}\\0m", tagline, version_string(), url);

    // We need to create these files in the Cargo output directory.
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());

    // Pretty version text
    let mut f = File::create(&out.join("version.pretty.txt"))?;
    writeln!(f, "{}", convert_codes(&ver))?;

    // Bland version text
    let mut f = File::create(&out.join("version.bland.txt"))?;
    writeln!(f, "{}", strip_codes(&ver))?;

    // Pretty usage text
    let mut f = File::create(&out.join("usage.pretty.txt"))?;
    writeln!(f, "{}", convert_codes(&tagline))?;
    writeln!(f)?;
    write!(f, "{}", convert_codes(&usage))?;

    // Bland usage text
    let mut f = File::create(&out.join("usage.bland.txt"))?;
    writeln!(f, "{}", strip_codes(&tagline))?;
    writeln!(f)?;
    write!(f, "{}", strip_codes(&usage))?;

    Ok(())
}

/// Converts the escape codes in ‘usage.txt’ to ANSI escape codes.
fn convert_codes(input: &str) -> String {
    input.replace("\\", "\x1B[")
}

/// Removes escape codes from ‘usage.txt’.
fn strip_codes(input: &str) -> String {
    input.replace("\\0m", "")
         .replace("\\1m", "")
         .replace("\\4m", "")
         .replace("\\32m", "")
         .replace("\\33m", "")
         .replace("\\1;31m", "")
         .replace("\\1;32m", "")
         .replace("\\1;33m", "")
         .replace("\\1;4;34", "")
}

/// Retrieves the [package] version in Cargo.toml as a string.
fn cargo_version() -> String {
    env::var("CARGO_PKG_VERSION").unwrap()
}

/// Returns the version and build parameters string.
fn version_string() -> String {
    let mut ver = cargo_version();

    let feats = nonstandard_features_string();
    if ! feats.is_empty() {
        ver.push_str(&format!(" [{}]", &feats));
    }

    ver
}

/// Finds whether a feature is enabled by examining the Cargo variable.
fn feature_enabled(name: &str) -> bool {
    env::var(&format!("CARGO_FEATURE_{}", name))
        .map(|e| ! e.is_empty())
        .unwrap_or(false)
}

/// A comma-separated list of non-standard feature choices.
fn nonstandard_features_string() -> String {
    let mut s = Vec::new();

    if ! feature_enabled("WITH_IDNA") {
        s.push("-idna");
    }

    if ! feature_enabled("WITH_TLS") {
        s.push("-tls");
    }

    if ! feature_enabled("WITH_HTTPS") {
        s.push("-https");
    }

    s.join(", ")
}
