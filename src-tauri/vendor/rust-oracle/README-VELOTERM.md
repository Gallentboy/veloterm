# rust-oracle（veloterm vendor 树）

> ## veloterm 集成说明
>
> 上游：https://github.com/kubo/rust-oracle （`oracle` crate，Oracle 官方
> ODPI-C 绑定的 Rust 封装）。**锁 tag v0.6.3**（4593547f；master 为
> 0.7.0-dev 不跟）。
>
> **许可证：UPL-1.0 / Apache-2.0 双许可（Cargo.toml SPDX 声明；上游仓库未
> 附许可证全文，全文见 ODPI-C 与上游仓库）——宽松许可无 GPL 负担。**
>
> **为什么换掉 stiang/oracle-rs**：纯 Rust TNS 驱动在 Oracle 11.2 服务器
> 实测不可用（协议补丁后仍失败），放弃；本树经 ODPI-C（Oracle 官方 C 驱
> 动层，编译期由 crates.io 的 odpic-sys 经 cc 静态编入，**编译期无需 Oracle
> SDK**）覆盖 11.2–23ai 全版本服务器。代价：**运行期需要 Oracle Instant
> Client**（macOS libclntsh.dylib / Windows oci.dll，由用户自行安装），
> 连接错误含 DPI-1047 时 app 侧追加安装提示（engines.rs）。
>
> **同步 API**：rust-oracle 无 async——app 侧经 `DbEngine::with_oracle`
> （spawn_blocking + 连接取出/归还）包裹每次调用；行流在阻塞线程内迭代，
> 取消粒度为语句间（mod.rs 会话层 AtomicBool），行流中不可中断。
>
> **树体裁剪**（相对上游；升级 = 换树 + 重放）：删 `tests/`、`examples/`、
> `docs/`、空 `odpi/`、`clippy.toml`、oracle_procmacro 的 `docs/`；
> `oracle_procmacro`（path 子成员，workspace 自带）保留 Cargo.toml + src。
>
> **本地代码补丁：无**（src/ 与上游逐字节一致）。唯一本地改动在
> Cargo.toml：`[lints.rust] mismatched_lifetime_syntaxes = "allow"`——
> 上游 17 处寿命省略风格触发 rustc 该风格告警（非缺陷），crate 内允许，
> 升级换树时重放这一段。
>
> **上游能力要点**：EZConnect 连接串、流式 ResultSet（逐行 fetch）、
> SqlValue Display 字符串化（NULL 内置）、位置/命名绑定、事务、PL/SQL
> 执行。0.6.3 无隐式结果集（DBMS_SQL.RETURN_RESULT）支持——工作台 PL/SQL
> 输出以 affected 行数为准（sqlsplit 整段化语义不变）。
