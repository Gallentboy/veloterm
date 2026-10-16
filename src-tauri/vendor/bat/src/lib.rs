//! `bat` is a library to print syntax highlighted content.
//!
//! The main struct of this crate is `PrettyPrinter` which can be used to
//! configure and run the syntax highlighting.
//!
//! If you need more control, you can also use the structs in the submodules
//! (start with `controller::Controller`), but note that the API of these
//! internal modules is much more likely to change. Some or all of these
//! modules might be removed in the future.
//!
//! "Hello world" example:
//! ```
//! use bat::PrettyPrinter;
//!
//! PrettyPrinter::new()
//!     .input_from_bytes(b"<span style=\"color: #ff00cc\">Hello world!</span>\n")
//!     .language("html")
//!     .print()
//!     .unwrap();
//! ```

#![deny(unsafe_code)]

mod macros;

pub mod assets;
pub mod assets_metadata {
    pub use super::assets::assets_metadata::*;
}
pub mod config;
pub mod controller;
mod decorations;
mod diff;
pub mod error;
pub mod input;
mod less;
#[cfg(feature = "lessopen")]
mod lessopen;
pub mod line_range;
pub(crate) mod nonprintable_notation;
pub mod output;
#[cfg(feature = "paging")]
mod pager;
// veloterm vendor 修补：paging 模块是无依赖的纯枚举（PagingMode 三值），
// 但 bin 侧 CLI（bat-app 经 #[path] 编入）无条件 `use bat::PagingMode`——
// 上游从不编译无 paging 的 bin（minimal-application 恒含 paging），故此处的
// cfg 门控在我们的 feature 组合（不开 paging，避开 minus→crossterm 0.27
// 与工程 0.29 双版本）下会断。pager.rs（minus 集成）保持门控。
pub(crate) mod paging;
mod preprocessor;
mod pretty_printer;
pub(crate) mod printer;
pub mod style;
pub(crate) mod syntax_mapping;
mod terminal;
pub mod theme;
mod vscreen;
pub(crate) mod wrapping;

pub use nonprintable_notation::{BinaryBehavior, NonprintableNotation};
pub use preprocessor::StripAnsiMode;
pub use pretty_printer::{Input, PrettyPrinter, Syntax};
pub use syntax_mapping::{MappingTarget, SyntaxMapping};
pub use wrapping::WrappingMode;

// veloterm vendor 修补：同上，PagingMode 的导出随模块去掉 paging 门控
pub use paging::PagingMode;
