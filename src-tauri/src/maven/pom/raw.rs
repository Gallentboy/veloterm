//! raw POM：roxmltree → 中间模型，只做结构提取不做语义（继承/插值/合并都在 effective.rs）。
//! 所有元素按 local name 匹配（现实 POM 命名空间不统一，不能按 qualified name 匹配）。

use crate::maven::error::{Error, Result};
use crate::maven::model::{DepDecl, Exclusion};

/// `<relativePath>` 三态：缺省 = `../pom.xml`；空元素 = 禁用本地查找；显式路径。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RelativePath {
    Default,
    Disabled,
    Explicit(String),
}

#[derive(Clone, Debug)]
pub struct ParentDecl {
    pub group: String,
    pub artifact: String,
    pub version: String,
}

/// POM 中声明的 `<repository>`（未应用 mirror）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawRepo {
    pub id: String,
    pub url: String,
}

/// `<build><plugins>` 或 `<pluginManagement><plugins>` 中声明的插件
#[derive(Clone, Debug)]
pub struct PluginDecl {
    pub group: Option<String>,
    pub artifact: String,
    pub version: Option<String>,
    pub dependencies: Vec<DepDecl>,
}

/// 未加工的 POM 模型（保留声明顺序）。
#[derive(Clone, Debug)]
pub struct RawPom {
    pub group: Option<String>,
    pub artifact: String,
    pub version: Option<String>,
    /// 默认 `jar`
    pub packaging: String,
    pub parent: Option<ParentDecl>,
    /// 保序；同 key 后者覆盖前者
    pub properties: Vec<(String, String)>,
    pub dependencies: Vec<DepDecl>,
    /// `<dependencyManagement><dependencies>` 条目（含 scope=import）
    pub dep_mgmt: Vec<DepDecl>,
    /// `<build><plugins>` 声明的构建插件
    pub plugins: Vec<PluginDecl>,
    /// `<build><pluginManagement><plugins>` 插件版本管理
    pub plugin_mgmt: Vec<PluginDecl>,
    pub repositories: Vec<RawRepo>,
    pub relative_path: RelativePath,
    /// 全部 `<profile>`（激活判定在 effective 侧：activeByDefault + jdk/os 静态条件）
    pub profiles: Vec<RawProfile>,
    /// `<distributionManagement><relocation>`：坐标已迁移，应改拉新坐标
    pub relocation: Option<ParentDecl>,
    /// `<modules><module>xxx</module></modules>` 子模块相对路径（多模块工程）
    pub modules: Vec<String>,
    /// 本地文件来源绝对路径（如有）
    pub file_path: Option<std::path::PathBuf>,
}

/// POM 内 `<profile>`（含激活条件，静态可判定部分）
#[derive(Clone, Debug)]
pub struct RawProfile {
    pub properties: Vec<(String, String)>,
    pub repositories: Vec<RawRepo>,
    /// profile 内声明的 <dependencies>（激活时并入有效依赖）
    pub dependencies: Vec<crate::maven::model::DepDecl>,
    /// profile 内声明的 <dependencyManagement>（激活时参与合并，import 条目同样支持）
    pub dep_mgmt: Vec<crate::maven::model::DepDecl>,
    pub plugins: Vec<PluginDecl>,
    pub plugin_mgmt: Vec<PluginDecl>,
    pub active_by_default: bool,
    /// `<activation><jdk>`（如 "1.8"、"[1.8,1.9)"）
    pub jdk: Option<String>,
    /// `<activation><os><name/arch/family>`
    pub os_name: Option<String>,
    pub os_arch: Option<String>,
    pub os_family: Option<String>,
    /// `<activation><property><name/value>`
    pub property_name: Option<String>,
    pub property_value: Option<String>,
}

impl RawPom {
    pub fn parse(xml: &str) -> Result<Self> {
        let sanitized = sanitize_pom_xml(xml);
        let doc = roxmltree::Document::parse(&sanitized)
            .map_err(|e| Error::BadXml { context: "pom".to_string(), source: e })?;
        let root = doc.root_element();
        let child = |name: &str| -> Option<roxmltree::Node<'_, '_>> {
            root.children()
                .filter(|n| n.is_element() && n.tag_name().name() == name)
                .next()
        };
        let text = |name: &str| -> Option<String> {
            child(name)
                .and_then(|n| n.text())
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
        };

        // relativePath 是 <parent> 的子元素；无 parent 时取默认值
        let relative_path = match child("parent") {
            None => RelativePath::Default,
            Some(p) => {
                match p
                    .children()
                    .find(|n| n.is_element() && n.tag_name().name() == "relativePath")
                {
                    None => RelativePath::Default,
                    // 缺省 ../pom.xml；空元素 = 禁用本地查找
                    Some(n) => match n.text().map(|t| t.trim()) {
                        None | Some("") => RelativePath::Disabled,
                        Some(p) => RelativePath::Explicit(p.to_string()),
                    },
                }
            }
        };

        let parent = child("parent").map(|p| {
            let pt = |name: &str| -> String {
                p.children()
                    .find(|n| n.is_element() && n.tag_name().name() == name)
                    .and_then(|n| n.text())
                    .map(|t| t.trim().to_string())
                    .unwrap_or_default()
            };
            ParentDecl {
                group: pt("groupId"),
                artifact: pt("artifactId"),
                version: pt("version"),
            }
        });

        let properties = child("properties")
            .map(|props| {
                props
                    .children()
                    .filter(|n| n.is_element())
                    .filter_map(|n| {
                        let k = n.tag_name().name().to_string();
                        let v = n.text().unwrap_or("").trim().to_string();
                        Some((k, v))
                    })
                    .collect()
            })
            .unwrap_or_default();

        let dependencies = child("dependencies")
            .map(|d| parse_dep_list(&d))
            .unwrap_or_default();
        let dep_mgmt = child("dependencyManagement")
            .and_then(|dm| dm.children().find(|n| n.is_element() && n.tag_name().name() == "dependencies"))
            .map(|d| parse_dep_list(&d))
            .unwrap_or_default();

        let repositories = child("repositories")
            .map(|repos| {
                repos
                    .children()
                    .filter(|n| n.is_element() && n.tag_name().name() == "repository")
                    .filter_map(|r| {
                        let t = |name: &str| -> Option<String> {
                            r.children()
                                .find(|n| n.is_element() && n.tag_name().name() == name)
                                .and_then(|n| n.text())
                                .map(|t| t.trim().to_string())
                                .filter(|t| !t.is_empty())
                        };
                        let id = t("id")?;
                        let url = t("url")?;
                        Some(RawRepo { id, url })
                    })
                    .collect()
            })
            .unwrap_or_default();

        let profiles = child("profiles")
            .map(|ps| {
                ps.children()
                    .filter(|n| n.is_element() && n.tag_name().name() == "profile")
                    .map(|p| {
                        let dependencies = p
                            .children()
                            .find(|n| n.is_element() && n.tag_name().name() == "dependencies")
                            .map(|d| parse_dep_list(&d))
                            .unwrap_or_default();
                        let dep_mgmt = p
                            .children()
                            .find(|n| n.is_element() && n.tag_name().name() == "dependencyManagement")
                            .and_then(|dm| {
                                dm.children().find(|n| n.is_element() && n.tag_name().name() == "dependencies")
                            })
                            .map(|d| parse_dep_list(&d))
                            .unwrap_or_default();
                        let properties = p
                            .children()
                            .find(|n| n.is_element() && n.tag_name().name() == "properties")
                            .map(|props| {
                                props
                                    .children()
                                    .filter(|n| n.is_element())
                                    .map(|n| {
                                        (n.tag_name().name().to_string(), n.text().unwrap_or("").trim().to_string())
                                    })
                                    .collect()
                            })
                            .unwrap_or_default();
                        let repositories = p
                            .children()
                            .find(|n| n.is_element() && n.tag_name().name() == "repositories")
                            .map(|rs| {
                                rs.children()
                                    .filter(|n| n.is_element() && n.tag_name().name() == "repository")
                                    .filter_map(|r| {
                                        let rt = |name: &str| -> Option<String> {
                                            r.children()
                                                .find(|n| n.is_element() && n.tag_name().name() == name)
                                                .and_then(|n| n.text())
                                                .map(|t| t.trim().to_string())
                                                .filter(|t| !t.is_empty())
                                        };
                                        Some(RawRepo { id: rt("id")?, url: rt("url")? })
                                    })
                                    .collect()
                            })
                            .unwrap_or_default();
                        let at_os = |p: &roxmltree::Node<'_, '_>| -> (Option<String>, Option<String>, Option<String>) {
                            p.children()
                                .find(|n| n.is_element() && n.tag_name().name() == "activation")
                                .and_then(|a| a.children().find(|n| n.is_element() && n.tag_name().name() == "os"))
                                .map(|o| {
                                    let ot = |name: &str| -> Option<String> {
                                        o.children()
                                            .find(|n| n.is_element() && n.tag_name().name() == name)
                                            .and_then(|n| n.text())
                                            .map(|t| t.trim().to_string())
                                            .filter(|t| !t.is_empty())
                                    };
                                    (ot("name"), ot("arch"), ot("family"))
                                })
                                .unwrap_or((None, None, None))
                        };
                        let act = p
                            .children()
                            .find(|n| n.is_element() && n.tag_name().name() == "activation");
                        let at = |name: &str| -> Option<String> {
                            act.and_then(|a| {
                                a.children()
                                    .find(|n| n.is_element() && n.tag_name().name() == name)
                                    .and_then(|n| n.text())
                                    .map(|t| t.trim().to_string())
                                    .filter(|t| !t.is_empty())
                            })
                        };
                        let act_prop = act.and_then(|a| a.children().find(|n| n.is_element() && n.tag_name().name() == "property"));
                        let prop_name = act_prop.and_then(|p| {
                            p.children()
                                .find(|n| n.is_element() && n.tag_name().name() == "name")
                                .and_then(|n| n.text())
                                .map(|t| t.trim().to_string())
                                .filter(|t| !t.is_empty())
                        });
                        let prop_val = act_prop.and_then(|p| {
                            p.children()
                                .find(|n| n.is_element() && n.tag_name().name() == "value")
                                .and_then(|n| n.text())
                                .map(|t| t.trim().to_string())
                        });
                        let os = at_os(&p);
                        let (prof_plugins, prof_plugin_mgmt) = parse_build_plugins(&p);
                        RawProfile {
                            properties,
                            dependencies,
                            dep_mgmt,
                            plugins: prof_plugins,
                            plugin_mgmt: prof_plugin_mgmt,
                            repositories,
                            active_by_default: at("activeByDefault").map(|t| t == "true").unwrap_or(false),
                            jdk: at("jdk"),
                            os_name: os.0,
                            os_arch: os.1,
                            os_family: os.2,
                            property_name: prop_name,
                            property_value: prop_val,
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        // relocation（distributionManagement）
        let relocation = child("distributionManagement")
            .and_then(|dm| dm.children().find(|n| n.is_element() && n.tag_name().name() == "relocation"))
            .map(|r| {
                let rt = |name: &str| -> String {
                    r.children()
                        .find(|n| n.is_element() && n.tag_name().name() == name)
                        .and_then(|n| n.text())
                        .map(|t| t.trim().to_string())
                        .unwrap_or_default()
                };
                ParentDecl { group: rt("groupId"), artifact: rt("artifactId"), version: rt("version") }
            })
            .filter(|r| !r.group.is_empty() && !r.artifact.is_empty());

        let modules = child("modules")
            .map(|m| {
                m.children()
                    .filter(|n| n.is_element() && n.tag_name().name() == "module")
                    .filter_map(|n| n.text().map(|t| t.trim().to_string()))
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default();

        let (plugins, plugin_mgmt) = parse_build_plugins(&root);

        Ok(Self {
            group: text("groupId"),
            artifact: text("artifactId").unwrap_or_default(),
            version: text("version"),
            packaging: text("packaging").unwrap_or_else(|| "jar".to_string()),
            parent,
            properties,
            dependencies,
            dep_mgmt,
            plugins,
            plugin_mgmt,
            repositories,
            relative_path,
            profiles,
            relocation,
            modules,
            file_path: None,
        })
    }

    /// 从本地文件解析，自动记录其绝对路径 file_path
    pub fn from_file(path: &std::path::Path) -> Result<Self> {
        let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let xml = std::fs::read_to_string(&canonical)
            .map_err(|e| Error::Other(format!("read pom file `{}`: {e}", canonical.display())))?;
        let mut pom = Self::parse(&xml)?;
        pom.file_path = Some(canonical);
        Ok(pom)
    }
}

/// 净化 POM XML：宽容处理「严格 XML 拒绝、但 Maven/libxml2 放行或位于我们不读的
/// 字段里」的内容，使其可通过 roxmltree 解析。三步各自跳过 CDATA：
/// 1. 剥离注释（规范禁止注释内 `--`；注释内容不影响依赖解析）
/// 2. 实体修复：未声明实体（&nbsp; 等 HTML 实体，常在 description/license）与
///    裸 `&`（URL 未转义）替换为转义形态
/// 3. 非法名修复：元素/属性名的本地部分以非法字符开头（hadoop 的
///    `<Xlint:-unchecked/>` 把 javac 开关当元素名），`:`/首字符替换为 `_`
fn sanitize_pom_xml(xml: &str) -> String {
    fix_tag_names(&fix_entities(&strip_comments(xml)))
}

/// 跳过 CDATA 区段，返回 Some(end_after_cdata)：调用方在 [i, end) 区间原样透传
fn cdata_end(xml: &str, i: usize) -> Option<usize> {
    xml[i..].starts_with("<![CDATA[").then(|| {
        match xml[i + 9..].find("]]>") {
            Some(p) => i + 9 + p + 3,
            None => xml.len(),
        }
    })
}

fn strip_comments(xml: &str) -> String {
    let mut out = String::with_capacity(xml.len());
    let mut i = 0;
    while i < xml.len() {
        if let Some(end) = cdata_end(xml, i) {
            out.push_str(&xml[i..end]);
            i = end;
            continue;
        }
        if xml[i..].starts_with("<!--") {
            match xml[i + 4..].find("-->") {
                Some(p) => i = i + 4 + p + 3,
                None => i = xml.len(),
            }
            continue;
        }
        // 防御性取首字符：循环条件保证 i < len，但绝不 panic（空串跳过）
        let Some(c) = xml[i..].chars().next() else {
            break;
        };
        out.push(c);
        i += c.len_utf8();
    }
    out
}

const PREDEFINED: [&str; 5] = ["amp", "lt", "gt", "apos", "quot"];

fn fix_entities(xml: &str) -> String {
    let mut out = String::with_capacity(xml.len());
    let mut i = 0;
    while i < xml.len() {
        if let Some(end) = cdata_end(xml, i) {
            out.push_str(&xml[i..end]);
            i = end;
            continue;
        }
        if xml[i..].starts_with('&') {
            // 合法实体形态 &name; / &#123; / &#x1F;
            let rest = &xml[i + 1..];
            let semi = rest.find(';').filter(|&p| p <= 10);
            if let Some(p) = semi {
                let name = &rest[..p];
                let known = PREDEFINED.contains(&name)
                    || name.strip_prefix('#').is_some_and(|n| {
                        n.strip_prefix('x').is_some_and(|h| !h.is_empty() && h.chars().all(|c| c.is_ascii_hexdigit()))
                            || (!n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
                    });
                if known {
                    out.push_str(&xml[i..i + 1 + p + 1]);
                    i += 1 + p + 1;
                    continue;
                }
            }
            // 未声明实体或裸 &：转义，后续字符（如 nbsp;）按文本处理
            out.push_str("&amp;");
            i += 1;
            continue;
        }
        let Some(c) = xml[i..].chars().next() else {
            break;
        };
        out.push(c);
        i += c.len_utf8();
    }
    out
}

fn fix_tag_names(xml: &str) -> String {
    let bytes = xml.as_bytes();
    let is_name_char = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || b == b'-' || b == b'.' || b == b':';
    let mut out = String::with_capacity(xml.len());
    let mut i = 0usize;
    while i < bytes.len() {
        if let Some(end) = cdata_end(xml, i) {
            out.push_str(&xml[i..end]);
            i = end;
            continue;
        }
        let Some(c) = xml[i..].chars().next() else {
            break;
        };
        // 名字可能的开头（含非法首字符如 `-`）；`<!`/`<?`（注释/CDATA/PI/doctype）透传
        if c == '<' && i + 1 < bytes.len() && !(bytes[i + 1] == b'!' || bytes[i + 1] == b'?') {
            let start = if bytes[i + 1] == b'/' { i + 2 } else { i + 1 };
            let mut j = start;
            while j < bytes.len() && is_name_char(bytes[j]) {
                j += 1;
            }
            let name = &xml[start..j.min(xml.len())];
            let fixed = fix_one_name(name);
            out.push('<');
            if bytes[i + 1] == b'/' {
                out.push('/');
            }
            out.push_str(&fixed);
            i = j;
            continue;
        }
        out.push(c);
        i += c.len_utf8();
    }
    out
}

/// 单个名字 token（元素名或属性名）的净化：
/// `ns:local` 中 local 首字符非法 → `:` 换 `_`；无冒号但首字符非法 → 首字符换 `_`
fn fix_one_name(name: &str) -> String {
    if let Some(pos) = name.find(':') {
        let local = &name[pos + 1..];
        let local_ok = local.chars().next().map(|c| c.is_alphabetic() || c == '_').unwrap_or(false);
        if !local_ok {
            return format!("{}{}_{}", "", &name[..pos], local);
        }
        return name.to_string();
    }
    match name.chars().next() {
        Some(c) if c.is_alphabetic() || c == '_' => name.to_string(),
        Some(_) => format!("_{}", name),
        None => "_".to_string(),
    }
}

/// `<dependencies>` 下的 `<dependency>` 列表。
fn parse_dep_list(deps: &roxmltree::Node<'_, '_>) -> Vec<DepDecl> {
    deps.children()
        .filter(|n| n.is_element() && n.tag_name().name() == "dependency")
        .map(|d| {
            let t = |name: &str| -> Option<String> {
                d.children()
                    .find(|n| n.is_element() && n.tag_name().name() == name)
                    .and_then(|n| n.text())
                    .map(|t| t.trim().to_string())
                    .filter(|t| !t.is_empty())
            };
            let exclusions = d
                .children()
                .find(|n| n.is_element() && n.tag_name().name() == "exclusions")
                .map(|ex| {
                    ex.children()
                        .filter(|n| n.is_element() && n.tag_name().name() == "exclusion")
                        .filter_map(|e| {
                            let et = |name: &str| -> String {
                                e.children()
                                    .find(|n| n.is_element() && n.tag_name().name() == name)
                                    .and_then(|n| n.text())
                                    .map(|t| t.trim().to_string())
                                    .unwrap_or_default()
                            };
                            Some(Exclusion { group: et("groupId"), artifact: et("artifactId") })
                                .filter(|x| !x.group.is_empty() && !x.artifact.is_empty())
                        })
                        .collect()
                })
                .unwrap_or_default();
            let optional = t("optional").map(|o| o == "true").unwrap_or(false);
            DepDecl {
                group: t("groupId").unwrap_or_default(),
                artifact: t("artifactId").unwrap_or_default(),
                version: t("version"),
                scope: t("scope"),
                optional,
                type_: t("type"),
                classifier: t("classifier"),
                exclusions,
            }
        })
        .filter(|d| !d.group.is_empty() && !d.artifact.is_empty())
        .collect()
}

/// `<plugins>` 节点下的 `<plugin>` 列表。
fn parse_plugin_list(plugins_node: &roxmltree::Node<'_, '_>) -> Vec<PluginDecl> {
    plugins_node
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == "plugin")
        .filter_map(|p| {
            let t = |name: &str| -> Option<String> {
                p.children()
                    .find(|n| n.is_element() && n.tag_name().name() == name)
                    .and_then(|n| n.text())
                    .map(|t| t.trim().to_string())
                    .filter(|t| !t.is_empty())
            };
            let artifact = t("artifactId")?;
            let group = t("groupId");
            let version = t("version");
            let dependencies = p
                .children()
                .find(|n| n.is_element() && n.tag_name().name() == "dependencies")
                .map(|d| parse_dep_list(&d))
                .unwrap_or_default();
            Some(PluginDecl {
                group,
                artifact,
                version,
                dependencies,
            })
        })
        .collect()
}

/// 解析任意容器节点（如 root 或 profile）下的 `<build><plugins>` 与 `<build><pluginManagement><plugins>`。
fn parse_build_plugins(parent_node: &roxmltree::Node<'_, '_>) -> (Vec<PluginDecl>, Vec<PluginDecl>) {
    let build = parent_node
        .children()
        .find(|n| n.is_element() && n.tag_name().name() == "build");
    let plugins = build
        .as_ref()
        .and_then(|b| b.children().find(|n| n.is_element() && n.tag_name().name() == "plugins"))
        .map(|p| parse_plugin_list(&p))
        .unwrap_or_default();
    let plugin_mgmt = build
        .as_ref()
        .and_then(|b| b.children().find(|n| n.is_element() && n.tag_name().name() == "pluginManagement"))
        .and_then(|pm| pm.children().find(|n| n.is_element() && n.tag_name().name() == "plugins"))
        .map(|p| parse_plugin_list(&p))
        .unwrap_or_default();
    (plugins, plugin_mgmt)
}

/// 异步 POM 来源抽象（依赖倒置：pom 层不依赖 repo 层）。
pub type BoxFut<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;

pub trait PomSource: Send + Sync {
    /// 按 GAV 获取 raw POM（实现自带缓存/落盘）。
    fn fetch_raw_pom<'a>(&'a self, g: &'a str, a: &'a str, v: &'a str) -> BoxFut<'a, Result<RawPom>>;

    /// 解析版本表达式（范围/LATEST/RELEASE）为具体版本；默认实现不支持。
    fn resolve_version<'a>(&'a self, _g: &'a str, _a: &'a str, expr: &'a str) -> BoxFut<'a, std::result::Result<String, String>> {
        Box::pin(async move { Err(format!("version expression `{expr}` not supported by this source")) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const POM: &str = r#"
<project xmlns="http://maven.apache.org/POM/4.0.0">
  <parent>
    <groupId>org.springframework.boot</groupId>
    <artifactId>spring-boot-starter-parent</artifactId>
    <version>3.2.0</version>
    <relativePath/>
  </parent>
  <artifactId>demo</artifactId>
  <packaging>jar</packaging>
  <properties>
    <spring.version>6.1.0</spring.version>
  </properties>
  <dependencies>
    <dependency>
      <groupId>org.springframework</groupId>
      <artifactId>spring-core</artifactId>
      <version>${spring.version}</version>
      <exclusions>
        <exclusion>
          <groupId>commons-logging</groupId>
          <artifactId>*</artifactId>
        </exclusion>
      </exclusions>
    </dependency>
  </dependencies>
  <dependencyManagement>
    <dependencies>
      <dependency>
        <groupId>org.junit</groupId>
        <artifactId>junit-bom</artifactId>
        <version>5.10.1</version>
        <type>pom</type>
        <scope>import</scope>
      </dependency>
    </dependencies>
  </dependencyManagement>
  <repositories>
    <repository>
      <id>jboss</id>
      <url>https://repository.jboss.org/nexus/content/groups/public/</url>
    </repository>
  </repositories>
</project>
"#;

    #[test]
    fn parse_basic_structure() {
        let pom = RawPom::parse(POM).unwrap();
        // groupId/version 未声明 → 由 parent 继承（effective 层处理）
        assert_eq!(pom.group, None);
        assert_eq!(pom.artifact, "demo");
        assert_eq!(pom.version, None);
        assert_eq!(pom.packaging, "jar");

        let parent = pom.parent.unwrap();
        assert_eq!(parent.artifact, "spring-boot-starter-parent");
        assert_eq!(parent.version, "3.2.0");
        // 空元素 = 禁用本地查找
        assert_eq!(pom.relative_path, RelativePath::Disabled);

        assert_eq!(pom.properties, vec![("spring.version".to_string(), "6.1.0".to_string())]);

        assert_eq!(pom.dependencies.len(), 1);
        let dep = &pom.dependencies[0];
        assert_eq!(dep.version.as_deref(), Some("${spring.version}"));
        assert_eq!(dep.exclusions.len(), 1);
        assert_eq!(dep.exclusions[0].artifact, "*");

        assert_eq!(pom.dep_mgmt.len(), 1);
        assert_eq!(pom.dep_mgmt[0].scope.as_deref(), Some("import"));
        assert_eq!(pom.dep_mgmt[0].type_.as_deref(), Some("pom"));

        assert_eq!(pom.repositories.len(), 1);
        assert_eq!(pom.repositories[0].id, "jboss");
    }

    #[test]
    fn sanitize_illegal_tag_names() {
        // hadoop-project 真实形态：javac 开关当元素名（本地名以 - 开头，XML 非法）
        let xml = r#"<project>
          <build><plugins><plugin><configuration>
            <Xlint:-unchecked/>
            <-Werror/>
            <Xmaxwarns>9999</Xmaxwarns>
          </configuration></plugin></plugins></build>
          <dependencies><dependency><groupId>g</groupId><artifactId>a</artifactId><version>1</version></dependency></dependencies>
        </project>"#;
        let pom = RawPom::parse(xml).expect("净化后应可解析");
        assert_eq!(pom.dependencies.len(), 1);
        assert_eq!(pom.dependencies[0].artifact, "a");
        // 无非法标签的正常 XML 不受影响
        let normal = r#"<project><groupId>g</groupId><artifactId>a</artifactId><version>1</version></project>"#;
        assert_eq!(sanitize_pom_xml(normal), normal);
        // 命名空间合法形态不改动
        let ns = r#"<project xmlns="http://maven.apache.org/POM/4.0.0"><a:b x="1"/></project>"#;
        assert_eq!(sanitize_pom_xml(ns), ns);
    }

    #[test]
    fn sanitize_comments_entities_and_bare_amp() {
        // 注释内含 --（规范非法）与杂乱内容：剥离后可解析
        let xml = r#"<project><!-- license -- see LICENSE -- <weird/> -->
          <groupId>g</groupId><artifactId>a</artifactId><version>1</version>
          <description>A &amp; B &nbsp; C &copy;</description>
          <url>http://x/?a=1&b=2</url>
          <dependencies><dependency><groupId>g</groupId><artifactId>d</artifactId><version>2</version></dependency></dependencies>
        </project>"#;
        let pom = RawPom::parse(xml).expect("净化后应可解析");
        assert_eq!(pom.dependencies.len(), 1);
        // CDATA 内的 & 与 < 原样保留
        let cdata = r#"<project><description><![CDATA[a & b < c]]></description><groupId>g</groupId><artifactId>a</artifactId><version>1</version></project>"#;
        assert!(RawPom::parse(cdata).is_ok());
        // 已是合法转义的 &amp; 不被二次转义
        assert!(sanitize_pom_xml("<a>&amp;&#65;&#x41;</a>").contains("&amp;&#65;&#x41;"));
    }

    #[test]
    fn parse_no_namespace_pom() {
        let pom = RawPom::parse("<project><groupId>g</groupId><artifactId>a</artifactId><version>1</version></project>").unwrap();
        assert_eq!(pom.group.as_deref(), Some("g"));
        assert_eq!(pom.version.as_deref(), Some("1"));
        assert_eq!(pom.relative_path, RelativePath::Default);
        assert!(pom.dependencies.is_empty());
    }

    #[test]
    fn optional_and_defaults() {
        let xml = r#"
<project>
  <dependencies>
    <dependency>
      <groupId>g</groupId>
      <artifactId>a</artifactId>
      <version>1.0</version>
      <optional>true</optional>
      <scope>runtime</scope>
    </dependency>
  </dependencies>
</project>"#;
        let pom = RawPom::parse(xml).unwrap();
        assert!(pom.dependencies[0].optional);
        assert_eq!(pom.dependencies[0].scope.as_deref(), Some("runtime"));
        assert!(pom.dependencies[0].type_.is_none());
    }

    #[test]
    fn parse_modules_support() {
        let xml = r#"
<project>
  <groupId>com.example</groupId>
  <artifactId>root</artifactId>
  <version>1.0.0</version>
  <packaging>pom</packaging>
  <modules>
    <module>core</module>
    <module>web</module>
    <module>common/utils</module>
  </modules>
</project>"#;
        let pom = RawPom::parse(xml).unwrap();
        assert_eq!(pom.packaging, "pom");
        assert_eq!(pom.modules, vec!["core", "web", "common/utils"]);
    }

    #[test]
    fn parse_plugins_and_plugin_management() {
        let xml = r#"
<project>
  <groupId>com.example</groupId>
  <artifactId>demo</artifactId>
  <version>1.0.0</version>
  <build>
    <plugins>
      <plugin>
        <artifactId>maven-compiler-plugin</artifactId>
        <version>3.11.0</version>
      </plugin>
      <plugin>
        <groupId>org.springframework.boot</groupId>
        <artifactId>spring-boot-maven-plugin</artifactId>
        <dependencies>
          <dependency>
            <groupId>org.projectlombok</groupId>
            <artifactId>lombok</artifactId>
            <version>1.18.30</version>
          </dependency>
        </dependencies>
      </plugin>
    </plugins>
    <pluginManagement>
      <plugins>
        <plugin>
          <groupId>org.apache.maven.plugins</groupId>
          <artifactId>maven-surefire-plugin</artifactId>
          <version>3.2.5</version>
        </plugin>
      </plugins>
    </pluginManagement>
  </build>
</project>"#;
        let pom = RawPom::parse(xml).unwrap();
        assert_eq!(pom.plugins.len(), 2);
        assert_eq!(pom.plugins[0].artifact, "maven-compiler-plugin");
        assert_eq!(pom.plugins[0].group, None);
        assert_eq!(pom.plugins[0].version.as_deref(), Some("3.11.0"));

        assert_eq!(pom.plugins[1].artifact, "spring-boot-maven-plugin");
        assert_eq!(pom.plugins[1].group.as_deref(), Some("org.springframework.boot"));
        assert_eq!(pom.plugins[1].dependencies.len(), 1);
        assert_eq!(pom.plugins[1].dependencies[0].artifact, "lombok");

        assert_eq!(pom.plugin_mgmt.len(), 1);
        assert_eq!(pom.plugin_mgmt[0].artifact, "maven-surefire-plugin");
        assert_eq!(pom.plugin_mgmt[0].version.as_deref(), Some("3.2.5"));
    }

    #[test]
    fn parse_profile_property_activation() {
        let xml = r#"
<project>
  <groupId>g</groupId><artifactId>a</artifactId><version>1</version>
  <profiles>
    <profile>
      <id>hadoop-2.0</id>
      <activation>
        <property>
          <name>!hadoop.profile</name>
        </property>
      </activation>
      <properties>
        <hadoop.version>2.7.4</hadoop.version>
      </properties>
    </profile>
  </profiles>
</project>"#;
        let pom = RawPom::parse(xml).unwrap();
        assert_eq!(pom.profiles.len(), 1);
        let prof = &pom.profiles[0];
        assert_eq!(prof.property_name.as_deref(), Some("!hadoop.profile"));
        assert!(prof.property_value.is_none());
        assert_eq!(prof.properties, vec![("hadoop.version".to_string(), "2.7.4".to_string())]);
    }
}
