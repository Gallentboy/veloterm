# vendor/dm-pure —— 达梦 DM8 纯 Rust 驱动（树内 vendored）

## 来源

- **crates.io**：`dm-pure` 1.2.4（`https://crates.io/crates/dm-pure`，MIT）
- **源码包**：`https://static.crates.io/crates/dm-pure/dm-pure-1.2.4.crate` 解包
- **上游仓库**：`https://github.com/zhangtao103239/dm8_pure_rs` —— **入库时（2026-10）该地址 404 不可访问**，
  crates.io 的 `.crate` 源码包是唯一可靠来源。上游随时可能失联，故整树入 vendor（而非仅依赖
  crates.io 版本号），以便自主打补丁与长期维护。

## 形态

纯 Rust 实现的 DM8 网络协议驱动：握手 / 登录（DH 密钥交换 + DES-CFB 登录加密）/ 执行 /
参数绑定 / LOB / 事务 / 分批 fetch。**零外部数据库依赖、零 C 库、零链接期动态库**——无需达梦
官方 Instant Client 或 libdmdpi，四平台免配置编译，符合本工程「缺库不得拖累构建与启动」铁律。

依赖仅 8 个纯 Rust crate：`num-bigint` / `num-traits` / `rand` / `md5` / `des` / `aes` /
`cipher` / `encoding_rs`。**同步驱动**（`std::net`，无 tokio），接入 app 侧经
`spawn_blocking` 包裹。

## 树内补丁清单

后续补丁按「文件 / 位置 / 改动 / 依据」四段追加到本节，升级时逐条重放。

| # | 文件 | 改动 | 依据 |
| - | ---- | ---- | ---- |
| 1 | `src/client.rs`（`execute` 读帧循环）、`src/frame.rs`（新增 `CMD_MORE_RESULTS`） | 读帧终止条件由「等到 `cmd == 187` 结果帧」改为官方的「`cmd != 269` 即停」；无 187 时若存在终态帧（`cmd == 0`）按 `ExecuteResult::Affected(0)` 成功返回，不再报「执行未产生有效结果帧」 | **官方 JDBC 驱动反编译实证**（`DmJdbcDriver8.jar` 的 `dm.jdbc.a.a.e(ad)`）：读帧循环为 `read header(64) → len=buf[4..6] → cmd=buf[6..10] → if(len>0) read body → if(cmd != 269) break`，269(0x10D) 是续帧标志。原实现等 187，对 `SET SCHEMA` / `SET NAMES` / `ALTER SESSION` 这类**只回 cmd=0 终态帧**的会话命令会永久阻塞 → 60s socket 超时 → `EAGAIN` → `[CODE:-6852] 网络异常`（实测：达梦工作台切换当前库卡死转圈）。官方 JDBC 对这些语句同样返回 updateCount=0 的成功。回归测试：`tests/mock_server.rs::e2e_session_command_no_result_frame` |

## 相对上游删除的文件

仅删包元数据，无代码改动：

- `.cargo_vcs_info.json`（git 提交信息，vendor 树不需要）
- `Cargo.toml.orig`（cargo 打包时的原始清单副本）
- `Cargo.lock`（workspace 统一锁版本，vendor 已被根 `[workspace] exclude`）
- `.github/`（上游 CI 工作流，树内不适用）
- `.gitignore`（上游仓库忽略规则，树内不需要）

`tests/`、`examples/`、`scripts/`（Python 参考向量生成器）**全部保留**：编解码层有与 Python
参考实现（dmpy）的随机向量交叉验证，`tests/mock_server.rs` + `mock_responses.rs` 提供无需真实
达梦服务器的端到端测试——打补丁后本机即可回归。

## 接入方式

`src-tauri/Cargo.toml`：

```toml
dm-pure = { path = "vendor/dm-pure" }
```

包内 `Cargo.toml` 保持上游版本号 `1.2.4` 原样；树内自有演进时在版本号后加 `+veloterm-N`
（semver build metadata，不影响解析）并在本节记录。

## 兼容模式（MySQL / Oracle）说明

驱动**不感知**达梦的兼容模式：它只做协议与编解码，SQL 语义完全由服务器解释。兼容模式是
实例级配置（`dm.ini` 的 `COMPATIBLE_MODE`：2=Oracle，4=MySQL），客户端侧无法会话级切换。

因此本工程的对象树 / 表详情 / 定义查询**统一走达梦的数据字典**
（schema 清单取元数据基表 `SYSOBJECTS WHERE TYPE$ = 'SCH'`；表/视图/函数/过程/序列
与列、源码、索引分别走 `ALL_TABLES` / `ALL_VIEWS` / `ALL_OBJECTS` / `ALL_SEQUENCES` /
`ALL_TAB_COLUMNS` / `ALL_SOURCE` / `ALL_INDEXES`）——这些视图在两种兼容模式下都存在
（兼容模式只增不删），一套查询覆盖两模式。

**schema 清单不能用 `ALL_USERS`**：达梦的用户与模式可分离（`CREATE SCHEMA xxx
AUTHORIZATION user` 可建与用户名不同的模式，一个用户也可挂多个模式），`ALL_USERS`
只列用户、会漏掉独立命名的业务模式。

连接表单里的「兼容模式」选项**只用于客户端侧方言选择**（SQL 编辑器高亮与语句分割方言、
树右键生成语句的取行语法 `ROWNUM` vs `LIMIT`），不改变服务器行为。

## 升级流程

1. 拉新 `.crate` 解包替换树内 `src/`（保留本 README 与补丁清单）；
2. 逐条重放「树内补丁清单」；
3. 同步包内 `Cargo.toml` 版本号与本 README 的「来源」节；
4. 跑 `cargo test -p dm-pure`（mock 端到端 + 向量交叉验证，无需真实服务器）。
