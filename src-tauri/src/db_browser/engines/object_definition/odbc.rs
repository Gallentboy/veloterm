//! ODBC 的对象定义：通用 ODBC 无标准 Catalog API 可取函数/过程/序列的
//! 源码定义（各库方言不可知），统一拒绝并引导用查询框直接操作。

use super::super::{DbEngine, ObjectDefinition};

pub(crate) async fn definition(
    _engine: &mut DbEngine,
    _schema: &str,
    name: &str,
    kind: &str,
) -> Result<ObjectDefinition, String> {
    Err(format!(
        "通用 ODBC 引擎暂不支持 {kind} 对象定义（无标准目录 API；如目标库有 \\\\
         ALL_SOURCE 一类字典视图，可在查询框直接 SELECT「{name}」的源码）"
    ))
}
