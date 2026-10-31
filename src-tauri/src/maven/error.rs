use std::fmt;

/// 带 Maven 语义的错误类型；其余传输/IO 错误由 anyhow 透传。
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// 所有仓库均未取到该 POM
    #[error("POM not found: {coord}\n  tried:\n{tried}")]
    PomNotFound {
        coord: String,
        tried: String,
    },

    /// 依赖缺 version 且无 dependencyManagement 可补
    #[error("missing version for {dep} (and no dependencyManagement entry)")]
    MissingVersion { dep: String },

    #[error("bad XML in {context}: {source}")]
    BadXml {
        context: String,
        source: roxmltree::Error,
    },

    /// settings.xml 中出现 {xxx} 加密密码（需要 settings-security，不支持）
    #[error("encrypted password for server `{id}` is not supported; provide a plaintext password in settings.xml")]
    EncryptedPassword { id: String },

    #[error("parent cycle detected at {coord}")]
    ParentCycle { coord: String },

    #[error("BOM import cycle detected at {coord}")]
    BomCycle { coord: String },

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// 记录一次下载在多仓库间的尝试结果，用于错误信息与失败清单。
#[derive(Debug, Default)]
pub struct AttemptLog {
    pub urls: Vec<String>,
}

impl AttemptLog {
    pub fn push(&mut self, url: &str, status: &dyn fmt::Display) {
        self.urls.push(format!("  {url} -> {status}"));
    }
}

impl fmt::Display for AttemptLog {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.urls.join("\n"))
    }
}
