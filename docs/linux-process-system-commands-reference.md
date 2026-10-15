# Linux 进程与系统监控命令参考

> 覆盖 `ps / top / htop / kill / killall / pkill / jobs-fg-bg / nohup / systemctl / journalctl / free / uptime / uname / lscpu / lsblk / df / du` 共 16 条命令。
>
> **来源**：procps-ng 4.x（ps/top/free/uptime/pgrep-pkill）、psmisc（killall）、util-linux 2.40（kill/lscpu/lsblk）、GNU coreutils 9.11（nohup/uname/df/du）、systemd 257（systemctl/journalctl）、htop 3.x、GNU Bash 5.x（jobs/fg/bg）。信号编号取自 **Linux 内核源码 `include/uapi/asm-generic/signal.h`**。
>
> **BSD/macOS 差异在每节末尾单列**，因为本机是 macOS、而实际运维目标通常是 Linux。
>
> 不确定的地方一律查了手册，未凭记忆推断。

## 速查表

| 命令 | 作用 | 危险级别 | 本参考中的参数模板 |
|---|---|---|---|
| `ps` | 查看进程快照 | safe | `ps {{options}}` |
| `top` / `htop` | 实时进程监控 | safe | `top {{options}}` |
| `kill` | 向进程发信号 | **destructive** | `kill {{options}} {{pid}}` |
| `killall` | 按进程名发信号 | **destructive** | `killall {{options}} {{name}}` |
| `pkill` | 按模式发信号 | **destructive** | `pkill {{options}} {{pattern}}` |
| `jobs` / `fg` / `bg` | 任务控制（shell 内建） | safe | `jobs {{options}}` |
| `nohup` | 后台不挂断运行 | safe | `nohup {{command}}` |
| `systemctl` | systemd 服务管理 | **confirm** | `systemctl {{options}} {{unit}}` |
| `journalctl` | 查看 systemd 日志 | safe | `journalctl {{options}} {{unit}}` |
| `free` | 内存使用 | safe | `free {{options}}` |
| `uptime` | 运行时长 / 负载 | safe | `uptime {{options}}` |
| `uname` | 系统 / 内核信息 | safe | `uname {{options}}` |
| `lscpu` | CPU 信息 | safe | `lscpu {{options}}` |
| `lsblk` | 块设备列表 | safe | `lsblk {{options}}` |
| `df` | 文件系统空间 | safe | `df {{options}}` |
| `du` | 目录占用 | safe | `du {{options}} {{path}}` |

> `kill` / `killall` / `pkill` 之所以是 **destructive**：`-9`(SIGKILL) 不给进程任何清理机会，会丢失未落盘数据、留下孤儿进程与脏锁文件。
> `systemctl` 是 **confirm**：`stop`/`restart`/`disable` 会影响线上服务；而 `halt`/`poweroff`/`reboot` 更是直接关机，属 destructive 级，见该节末尾。

---

## 1. ps — 查看进程快照

**语法**：`ps [options]`

`ps` 的输出**列**由你选的选项决定，不是固定的一堆列。三种风格混用要注意：

| 风格 | 写法 | 特点 |
|---|---|---|
| UNIX | `ps -ef` | 带 `-`，选项可分组 |
| BSD | `ps aux` | **不带 `-`**，这是最常用的一种 |
| GNU 长选项 | `ps --sort=-%cpu` | 双横线 |

### `ps aux` 的 11 列（BSD 风格）

| 列 | 含义 |
|---|---|
| `USER` | 进程的有效用户（EUID） |
| `PID` | 进程 ID |
| `%CPU` | CPU 使用率（自进程启动以来的累计占比，**不是瞬时值**） |
| `%MEM` | 常驻内存占物理内存的百分比 |
| `VSZ` | 虚拟内存大小（KiB，含未实际分配的） |
| `RSS` | 常驻集大小（KiB，实际占用的物理内存） |
| `TTY` | 控制终端（`?` 表示无终端，通常是守护进程） |
| `STAT` | 进程状态（见下表） |
| `START` | 启动时间（24 小时内显示时刻，超过显示日期） |
| `TIME` | 累计 CPU 时间（user + system） |
| `COMMAND` | 命令与参数（`ps aux` 会截断，用 `ps auxww` 不截断） |

### `ps -ef` 的 8 列（UNIX 风格）

`UID PID PPID C STIME TTY TIME CMD` —— 比 `aux` 多了 `PPID`（父进程 ID）和 `C`（处理器利用率）。

### STAT 状态码（`s`/`stat`/`state`）

| 码 | 含义 |
|---|---|
| `D` | 不可中断睡眠（通常是 I/O） |
| `I` | 空闲内核线程 |
| `R` | 运行中或可运行（在运行队列上） |
| `S` | 可中断睡眠（等待事件完成） |
| `T` | 被作业控制信号停止 |
| `t` | 被调试器在跟踪期间停止 |
| `W` | 换页（Linux 2.6 起已无效） |
| `X` | 已死（不应出现） |
| `Z` | 僵尸进程（已终止但父进程未回收） |

BSD 格式或使用 `stat` 关键字时还会附加字符：

| 字符 | 含义 |
|---|---|
| `<` | 高优先级（对其他用户不 nice） |
| `N` | 低优先级（对其他用户 nice） |
| `L` | 有页面被锁进内存（实时/自定义 I/O） |
| `s` | 是会话首进程 |
| `l` | 多线程（使用了 `CLONE_THREAD`） |
| `+` | 位于前台进程组 |

### 进程选择（Selection）

| 选项 | 含义 |
|---|---|
| `-A` / `-e` | 选择所有进程 |
| `-a` | 选择所有进程，但排除会话首进程与无终端的进程 |
| `-d` | 选择所有进程，但排除会话首进程 |
| `-N` / `--deselect` | 取反：排除满足条件的进程 |
| `-C cmdlist` | 按可执行名选择（**注意不是命令行**） |
| `-p pidlist` / `--pid` | 按 PID 选择 |
| `-q pidlist` / `--quick-pid` | 按 PID 选择（快速模式，不额外过滤，保持给定顺序） |
| `--ppid pidlist` | 按父进程 ID 选择 |
| `-s sesslist` / `--sid` | 按会话 ID 选择 |
| `-t ttylist` / `--tty` | 按终端选择（`-` 表示无终端） |
| `-U userlist` / `--User` | 按**真实**用户 ID 选择 |
| `-u userlist` / `--user` | 按**有效**用户 ID 选择 |
| `-G grplist` / `--Group` | 按**真实**组 ID 选择 |
| `-g grplist` / `--group` | 按会话或有效组名选择 |
| `-123` | 按进程组 ID（PGID）选择 |
| `-T` | 显示线程（带 SPID 列） |
| `-L` | 显示线程（带 LWP 与 NLWP 列） |
| `-m` | 在线程后显示进程 |

### 输出格式

| 选项 | 含义 |
|---|---|
| `-f` | 完整格式（含 UID、PID、PPID、C、STIME、TTY、TIME、CMD） |
| `-F` | 超完整格式（额外含 SZ、RSS、PSR） |
| `-l` | 长格式 |
| `-j` | 作业格式（PID、PGID、SID、TTY、TIME、CMD） |
| `-o format` / `--format` | 自定义列（如 `-o pid,user,%cpu,args`） |
| `-O format` | 同 `-o`，但预置若干默认列 |
| `-M` / `Z` | 增加安全数据列（SELinux 上下文） |
| `-c` | 显示不同的调度器信息（配合 `-l`） |
| `-y` | 不显示 flags，用 rss 替代 addr（仅配合 `-l`） |
| `-P` | 增加 psr（CPU 编号）列 |
| `-w` | 宽输出（写两次则不限宽度） |
| `--no-headers` / `--no-heading` | 不打印表头 |
| `--headers` | 每页重复表头 |
| `--forest` / `-H` | ASCII 进程树 |
| `--cols n` / `--columns n` / `--width n` | 设置屏宽 |
| `--rows n` / `--lines n` | 设置屏高 |
| `--sort spec` / `k spec` | 排序，如 `--sort=-%cpu,-%mem` |
| `--delimiter delim` | 列间分隔符（单字符） |
| `-D format` / `--date-format` | `lstart` 字段的日期格式（strftime） |
| `--signames` | 用缩写信号名显示信号掩码 |
| `--cumulative` | 把已死子进程数据计入父进程 |
| `--context` | 显示安全上下文格式 |
| `--info` | 打印调试信息 |
| `--help [section]` | 帮助（section 可为 simple/list/output/threads/misc/all） |
| `-V` / `--version` | 版本 |

### 常用格式关键字（`-o`）

`pid` `ppid` `pgid` `sid` `uid` `user` `ruser` `group` `tty` `stat` `%cpu`(`pcpu`) `%mem`(`pmem`) `vsz` `rss` `ni` `pri` `c` `f`(`flags`) `wchan` `lstart` `etime` `time`(`bsdtime`) `nlwp` `comm` `args`(`command`) `psr`

### 示例

```bash
ps aux                          # 最常用：全量进程
ps auxww                        # 同上，命令行不截断
ps -ef                          # UNIX 风格，含 PPID
ps -ef --forest                 # 进程树
ps -u nginx                     # 只看某用户
ps -C nginx -o pid,etime,cmd    # 按可执行名 + 自定义列
ps aux --sort=-%mem | head -n 11   # 内存 TOP 10
ps aux --sort=-%cpu | head -n 11   # CPU TOP 10
ps -eo pid,ppid,%cpu,%mem,stat,args --sort=-%cpu | head
ps -p 1234 -o pid,user,%cpu,%mem,etime,args   # 看单个进程
ps -eLf | grep java             # 看线程（LWP/NLWP）
ps -eo stat,pid,args | grep '^Z'   # 找僵尸进程
ps -eo user | sort | uniq -c | sort -rn   # 按用户统计进程数
ps --ppid 1 -o pid,cmd          # 找某进程的子进程
```

### 易踩坑

- **`ps aux` 的 `%CPU` 不是瞬时值**，是自进程启动以来的累计占比 —— 想看瞬时用 `top`。
- `ps aux` 默认**截断命令行**，宽终端下要用 `ps auxww`。
- `ps -C name` 匹配的是**可执行文件名**，不是命令行参数；按参数匹配用 `pgrep -f`。
- `-u` 是**有效**用户、`-U` 是**真实**用户，二者不同（sudo 场景差异明显）。
- 状态 `Z`（僵尸）**杀不掉** —— 必须等父进程回收或杀掉父进程。

**BSD/macOS**：macOS 的 `ps` 默认就是 BSD 风格，`ps aux` 可用，但**没有 `--forest`/`-H`/`-eLf` 的完整 GNU 支持**，`-o` 支持的关键字也少（无 `nlwp`、`psr` 等）；`ps -M` 在 macOS 上是“显示线程”而非 SELinux 上下文。

---

## 2. top / htop — 实时进程监控

**语法**：`top [option ...]` / `htop [option ...]`

### top 的屏幕三区

1. **Summary Area（汇总区）**：`uptime`+负载 → 任务与 CPU 状态 → 内存/交换
2. **Fields/Columns Header**：列标题
3. **Task Area（任务区）**：进程列表

汇总区内存行的 `/proc/meminfo` 来源（手册明确）：

| 显示 | 来源 |
|---|---|
| `total` | MemTotal |
| `free` | MemFree |
| `used` | MemTotal − MemAvailable |
| `buff/cache` | Buffers + Cached + SReclaimable |
| 交换行 `avail` | MemAvailable（属于物理内存） |

### top 命令行选项

| 选项 | 含义 |
|---|---|
| `-b, --batch` | 批处理模式（不接受输入，配合 `-n` 跑完即退，**写脚本/重定向必用**） |
| `-n, --iterations=NUMBER` | 最多刷新多少帧后结束 |
| `-d, --delay=SECS[.TENTHS]` | 刷新间隔（支持小数，不允许负数） |
| `-p, --pid=PIDLIST` | 只监控指定 PID（最多 20 个，可逗号分隔或多次给） |
| `-U, --filter-any-user=USER` | 只显示匹配该用户的进程（匹配任意 uid：real/effective/saved/fs）；`!` 前缀取反 |
| `-u, --filter-only-euser=USER` | 只显示有效用户匹配的进程；`!` 前缀取反 |
| `-o, --sort-override=FIELDNAME` | 指定排序列（前置 `+` 从高到低、`-` 从低到高） |
| `-O, --list-fields` | 列出所有可用字段名后退出 |
| `-H, --threads-show` | 显示单个线程（而非按进程汇总） |
| `-i, --idle-toggle` | 反转上次记住的 `i` 状态（隐藏空闲任务） |
| `-c, --cmdline-toggle` | 反转上次记住的 `c` 状态（命令行 / 程序名） |
| `-S, --accum-time-toggle` | 反转累计时间模式 |
| `-E, --scale-summary-mem=k\|m\|g\|t\|p\|e` | 强制汇总区内存单位 |
| `-e, --scale-task-mem=k\|m\|g\|t\|p` | 强制任务区内存单位 |
| `-1, --single-cpu-toggle` | 反转 CPU 显示（单行 / 每核一行） |
| `-s, --secure-mode` | 强制安全模式（禁用部分交互命令） |
| `-A, --apply-defaults` | 只用编译默认值，忽略除 `/etc/toprc` 外所有配置（**必须是唯一选项**） |
| `-w, --width[=COLUMNS]` | 输出宽度（批处理模式下不带参数时用 `COLUMNS=`/`LINES=` 环境变量，否则固定 512） |
| `-h, --help` | 帮助 |
| `-V, --version` | 版本 |

### top 交互按键

**全局**

| 键 | 作用 |
|---|---|
| `q` | 退出 |
| `?` / `h` | 帮助 |
| `<Enter>` / `<Space>` | 刷新显示 |
| `d` / `s` | 修改刷新间隔 |
| `k` | 杀进程（会提示输入 PID 与信号） |
| `r` | renice 进程 |
| `W` | 把当前配置写入 `~/.toprc` |
| `Z` | 修改配色 |
| `A` | 切换备用显示模式 |
| `B` | 切换加粗 |
| `0` | 零值抑制开关 |
| `=` | 清除所有显示限制 |
| `L` | 定位字符串 |
| `&` | 定位下一个匹配 |
| `Y` | 检视其他输出 |
| `^O` | 显示当前生效的过滤器 |

**汇总区**

| 键 | 作用 |
|---|---|
| `l` | 负载/运行时长开关 |
| `t` | 任务/CPU 状态开关 |
| `m` | 内存/交换开关 |
| `1` | 单核 / 每核 CPU 状态开关 |
| `2` | NUMA 节点 / CPU 汇总开关 |
| `3` | 展开 NUMA 节点 |
| `4` | 多元素相邻显示开关 |
| `5` | 显示 P 核与 E 核开关 |
| `!` | 合并 CPU 模式开关 |
| `^` | 核心 vs CPU 显示开关 |
| `C` | 显示滚动坐标开关 |

**任务区**

| 键 | 作用 |
|---|---|
| `f` | 字段管理（**也是改排序列的正规入口**） |
| `R` | 反转排序方向 |
| `<` / `>` | 把排序列左移 / 右移 |
| `M` / `N` / `P` / `T` | 旧版排序快捷键：`%MEM` / `PID` / `%CPU` / `TIME+`（兼容保留，不显示在帮助里；`A` 排序启动时间**已不支持**） |
| `c` | 命令行 / 程序名开关 |
| `i` | 空闲进程开关 |
| `n` / `#` | 设置最大显示任务数 |
| `u` / `U` | 只显示指定用户 |
| `o` / `O` | 过滤器 |
| `V` | 森林视图（进程树）开关 |
| `v` | 折叠/展开子进程 |
| `F` | 保持父进程焦点 |
| `S` | 累计时间模式开关 |
| `x` | 列高亮开关（**配合排序时建议打开**） |
| `y` | 行高亮开关 |
| `z` | 彩色 / 单色开关 |
| `b` | 加粗/反显开关 |
| `j` / `J` | 字符列 / 数字列对齐开关 |
| `^A` `^G` `^K` `^N` `^P` `^U` `^L` | 显示 capabilities / cgroup / 命令行 / 环境变量 / 命名空间 / 补充组 / 已记录消息 |

### htop 命令行选项

| 选项 | 含义 |
|---|---|
| `-d, --delay=DELAY` | 刷新间隔（**单位是十分之一秒**；<1 提到 1，>100 降到 100） |
| `-C, --no-color` / `--no-colour` | 单色模式启动 |
| `-F, --filter=FILTER` | 按命令过滤进程（**大小写不敏感、按固定字符串匹配**，可用 `\|` 分隔多个词） |
| `-p, --pid=PID,PID...` | 只显示指定 PID |
| `-s, --sort-key=COLUMN` | 按此列排序（`--sort-key help` 列出可选列）；会强制列表视图，除非同时给 `-t` |
| `-u, --user[=USERNAME\|UID]` | 只显示指定用户的进程（省略则为自己） |
| `-t, --tree[=MODE]` | 树状视图（MODE：`0`/`classic`、`1`/`soft`/`stable`、`2`/`hard`/`STABLE`） |
| `-H, --highlight-changes=DELAY` | 高亮新老进程 |
| `-U, --no-unicode` | 用 ASCII 而非 Unicode 画图形表 |
| `-M, --no-mouse` | 禁用鼠标 |
| `--no-function-bar` | 隐藏功能键栏 |
| `--no-meters` | 隐藏图形表 |
| `--readonly` | 禁用一切修改系统的功能 |
| `--drop-capabilities[=off\|basic\|strict]` | 丢弃不需要的 Linux capabilities（需编译期开启 + libcap） |
| `-h, --help` | 帮助 |
| `-V, --version` | 版本 |

### 示例

```bash
top                             # 默认
top -b -n 1                     # 批处理抓一帧（脚本用）
top -b -n 1 -o %MEM | head -20  # 按内存排序抓一帧
top -p 1234                     # 只看某 PID
top -u nginx                    # 只看某用户
top -H                          # 显示线程
top -d 1                        # 1 秒刷新
top -o %CPU                     # 按 CPU 排序
htop                            # 更友好的交互界面
htop -u nginx -t                # 指定用户 + 树状
htop -p 1234,5678               # 只看两个 PID
```

### 易踩坑

- `top` 在**管道里**必须加 `-b`，否则输出一堆终端控制字符。
- `top -b -n 1` 的第一帧 CPU 百分比**不可靠**（没有前一帧做差），至少要 `-n 2` 并取最后一帧。
- `top` 的 `%CPU` 在非线程模式下**可以超过 100%**（多线程进程累加）；按 `H` 切到线程模式后不会。
- `htop -d` 的单位是**十分之一秒**，`htop -d 10` = 1 秒；而 `top -d` 的单位是**秒**。两者不同，别搞混。
- `q` 退出，`Ctrl-C` 也可。

**BSD/macOS**：macOS 自带 `top` 但选项与 Linux 差异很大（`-l` 表示采样次数、`-n` 表示显示行数），且**没有 `htop`**（需 `brew install htop`）。

---

## 3. kill — 向进程发信号

**语法**：`kill [options] <pid> [...]`

默认发送 **SIGTERM(15)**。

### PID 的写法

| 写法 | 含义 |
|---|---|
| `n`（n>0） | 向 PID 为 n 的进程发信号 |
| `0` | 向**调用进程所在进程组**的所有进程发信号 |
| `-1` | 向所有 PID > 1 的进程发信号（**极其危险**） |
| `-n`（n>1） | 向进程组 n 的所有进程发信号 |
| `-n` 且 n<0 | 向进程组 |n| 的所有进程发信号 |

### 选项

| 选项 | 含义 |
|---|---|
| `-s, --signal signal` | 指定要发送的信号（名字或编号） |
| `-l, --list [number\|0xsigmask]` | 列出信号名；把信号编号转成名字；把信号掩码转成名字 |
| `-L, --table` | 类似 `-l`，但打印信号名与对应编号 |
| `-a, --all` | 不把“命令名→PID”的转换限制为同 UID 的进程 |
| `-p, --pid` | 只打印指定进程的 PID，**不发信号** |
| `-r, --require-handler` | 若目标进程未在用户态捕获该信号，则不发 |
| `-q, --queue value` | 用 `sigqueue(3)` 而非 `kill(2)`，把整数 value 随信号一起发送 |
| `--timeout milliseconds signal` | 先发一个信号，延迟后再发一个跟随信号（基于 PID fd，保证目标存活） |
| `--verbose` | 打印将要被 signal 的 PID 与信号 |
| `-d, --show-process-state pid` | 解码 `/proc/pid/status` 中的信号相关字段 |
| `-h, --help` | 帮助 |
| `-V, --version` | 版本 |

### 常用信号（编号经内核源码核实）

| 信号 | 编号 | 默认动作 | 说明 |
|---|---|---|---|
| `SIGHUP` | 1 | Term | 终端挂断 / 控制进程死亡（**很多守护进程用它来重载配置**） |
| `SIGINT` | 2 | Term | 键盘中断（`Ctrl-C`） |
| `SIGQUIT` | 3 | Core | 键盘退出（`Ctrl-\`），产生 core |
| `SIGILL` | 4 | Core | 非法指令 |
| `SIGTRAP` | 5 | Core | 跟踪/断点陷阱 |
| `SIGABRT` | 6 | Core | `abort(3)` 中止 |
| `SIGBUS` | 7 | Core | 总线错误 |
| `SIGFPE` | 8 | Core | 算术异常 |
| `SIGKILL` | 9 | Term | **强制杀死，不可捕获/阻塞/忽略** |
| `SIGUSR1` | 10 | Term | 用户自定义 1 |
| `SIGSEGV` | 11 | Core | 无效内存引用 |
| `SIGUSR2` | 12 | Term | 用户自定义 2 |
| `SIGPIPE` | 13 | Term | 管道破裂 |
| `SIGALRM` | 14 | Term | `alarm(2)` 定时器 |
| `SIGTERM` | 15 | Term | **终止（kill 默认）** |
| `SIGSTKFLT` | 16 | Term | 协处理器栈错误（未用） |
| `SIGCHLD` | 17 | Ign | 子进程停止/终止/继续 |
| `SIGCONT` | 18 | Cont | 继续（若已停止） |
| `SIGSTOP` | 19 | Stop | **停止，不可捕获/阻塞/忽略** |
| `SIGTSTP` | 20 | Stop | 终端停止（`Ctrl-Z`） |
| `SIGTTIN` | 21 | Stop | 后台进程读终端 |
| `SIGTTOU` | 22 | Stop | 后台进程写终端 |
| `SIGURG` | 23 | Ign | socket 紧急条件 |
| `SIGXCPU` | 24 | Core | 超 CPU 时间限制 |
| `SIGXFSZ` | 25 | Core | 超文件大小限制 |
| `SIGVTALRM` | 26 | Term | 虚拟时钟 |
| `SIGPROF` | 27 | Term | 性能分析时钟 |
| `SIGWINCH` | 28 | Ign | 窗口大小改变 |
| `SIGIO` | 29 | Term | I/O 就绪 |
| `SIGPWR` | 30 | Term | 电源故障 |
| `SIGSYS` | 31 | Core | 非法系统调用 |
| `SIGRTMIN`..`SIGRTMAX` | 32..64 | Term | 实时信号 |

> **实时信号编号的坑**：内核支持 33 个实时信号，编号 **32–64**；但 glibc 的 NPTL 内部占用两个（LinuxThreads 占用三个），因此把 `SIGRTMIN` 调整为 **34（或 35）**。所以程序**不应硬编码实时信号编号**，一律用宏。

### 示例

```bash
kill 1234                  # 发 SIGTERM（默认），请求优雅退出
kill -15 1234              # 同上
kill -9 1234               # 强制杀死（最后手段）
kill -HUP 1234             # 让守护进程重载配置
kill -USR1 1234            # 用户自定义信号（nginx 用它重开日志）
kill -STOP 1234            # 暂停进程
kill -CONT 1234            # 恢复进程
kill -l                    # 列出所有信号名
kill -l 9                  # 编号 9 对应的信号名 → KILL
kill -l TERM               # 信号名对应的编号 → 15
kill -0 1234               # 探测进程是否存在（不发信号，仅检查权限）
kill -9 $(pgrep -f 'java.*MyApp')   # 组合：按模式找 PID 再杀
kill -- -1234              # 向进程组 1234 发信号
```

### 易踩坑

- **先 `-15`，确认无效再 `-9`**。`-9` 无法被捕获，进程来不及 flush、释放锁、删除 pid 文件。
- `kill -9` 杀不掉 `D` 状态（不可中断睡眠）的进程 —— 它在等 I/O，只能等或重启。
- 僵尸进程（`Z`）`kill` 无效，必须处理其父进程。
- `kill -9 1`（PID 1）在现代内核上**被拒绝**，别试。
- `kill` 是 **shell 内建**，也是 `/bin/kill` 外部命令，行为略有差异（内建支持 `%1` 这类 jobspec）。

**BSD/macOS**：macOS 无 `--timeout`、`--require-handler`、`-q/--queue`、`-d`；信号编号在 macOS 上**不同**（例如 SIGUSR1=30、SIGUSR2=31、SIGSTOP=17、SIGTSTP=18），**跨平台脚本不要硬编码编号**。

---

## 4. killall — 按进程名发信号

**语法**：`killall [options] [--] name ...`

按**进程名**（不是命令行）匹配并发信号，默认 SIGTERM。

### 选项

| 选项 | 含义 |
|---|---|
| `-e, --exact` | 要求完全匹配（名字超过 15 字符时，若无法取得全名会被跳过；配合 `-v` 会打印每个被跳过的条目） |
| `-I, --ignore-case` | 进程名匹配忽略大小写 |
| `-g, --process-group` | 杀进程组（**每组只发一次信号**，即使匹配到多个进程） |
| `-i, --interactive` | 杀之前逐个交互确认 |
| `-l, --list` | 列出所有已知信号名 |
| `-n, --ns PID` | 匹配指定 PID 所在命名空间（默认匹配所有命名空间） |
| `-o, --older-than TIME` | 只匹配比 TIME 更老（更早启动）的进程；TIME 为浮点数+单位（`s m h d w M y`） |
| `-y, --younger-than TIME` | 只匹配比 TIME 更年轻（更晚启动）的进程 |
| `-q, --quiet` | 没有杀到任何进程时不抱怨 |
| `-r, --regexp` | 进程名模式按 POSIX 扩展正则解释 |
| `-s, --signal SIG` / `-SIG` | 发送指定信号（替代 SIGTERM） |
| `-u, --user USER` | 只杀该用户拥有的进程（此时进程名可省略） |
| `-v, --verbose` | 报告信号是否发送成功 |
| `-w, --wait` | 等所有被杀进程真正退出（每秒检查一次）；**若信号被忽略或进程变僵尸，可能永远等待** |
| `-Z, --context PATTERN` | 按安全上下文（SELinux）匹配；**必须放在其他参数之前** |
| `-V, --version` | 版本 |

### 示例

```bash
killall nginx                    # 杀掉所有名为 nginx 的进程
killall -9 nginx                 # 强制杀
killall -HUP nginx               # 让 nginx 重载配置
killall -i -v nginx              # 逐个确认并报告
killall -u deploy                # 杀掉 deploy 用户的所有进程
killall -r 'nginx.*worker'       # 正则匹配
killall -o 1h nginx              # 只杀启动超过 1 小时的 nginx
killall -w nginx                 # 等它彻底退出
killall -g myproc                # 杀整个进程组
```

### 易踩坑

- `killall` 匹配的是**进程名**（`/proc/pid/comm`，通常 ≤15 字符），不是完整命令行 —— 想按命令行匹配用 `pkill -f`。
- 名字超过 15 字符时可能匹配不全，`-e` 可要求精确匹配（不匹配则跳过）。
- `killall` 在 **Solaris 上语义完全不同**（会杀掉所有进程），跨平台脚本要警惕。
- `-w` 有**永久阻塞**风险，脚本里慎用。

**BSD/macOS**：macOS 的 `killall` 参数集小得多，无 `-r`、`-o`、`-y`、`-w`、`-Z`、`-n`。

---

## 5. pkill — 按模式发信号

**语法**：`pkill [options] pattern`

按**模式**（默认匹配进程名，`-f` 时匹配完整命令行）匹配。与 `pgrep` 同一套选项（`pgrep` 只列出、不发信号）。

### 选项

| 选项 | 含义 |
|---|---|
| `-signal` / `--signal signal` | 指定发送的信号（数字或符号名） |
| `-f, --full` | 用**完整命令行**匹配（否则只匹配进程名） |
| `-x, --exact` | 名字（或 `-f` 时的命令行）必须**完全匹配** |
| `-i, --ignore-case` | 忽略大小写 |
| `-c, --count` | 不输出内容，只打印匹配数量（**pkill 的计数是匹配数，不是成功信号数**） |
| `-e, --echo` | 显示被杀进程的名字与 PID（仅 pkill） |
| `-a, --list-full` | 同时列出完整命令行与 PID（仅 pgrep） |
| `-l, --list-name` | 同时列出进程名与 PID（仅 pgrep） |
| `-d, --delimiter delimiter` | 输出 PID 之间的分隔符（默认换行，仅 pgrep） |
| `-Q, --shell-quote` | 命令行以 shell 引用形式输出（仅 pgrep） |
| `--quiet` | 不向标准输出写任何内容（仅 pgrep） |
| `-F, --pidfile file` | 从文件读 PID（`-` 表示 stdin）；与 `--pid` 冲突 |
| `-L, --logpidfile` | 若 pidfile 未加锁则失败 |
| `-A, --ignore-ancestors` | 忽略 pgrep/pkill 自身的所有祖先进程（sudo 提权时有用） |
| `-H, --require-handler` | 只匹配在用户态装了该信号处理器的进程 |
| `-m, --mrelease` | 发完信号后调用 `process_mrelease()` 立即释放目标内存 |
| `-n, --newest` | 只选匹配中最新的（最晚启动） |
| `-o, --oldest` | 只选匹配中最老的（最早启动） |
| `-O, --older secs` | 只选启动超过 secs 秒的 |
| `-p, --pid pid,...` | 只匹配指定 PID；与 `--pidfile` 冲突 |
| `-P, --parent ppid,...` | 只匹配指定父进程的子进程 |
| `-g, --pgroup pgrp,...` | 只匹配指定进程组（0 表示自身进程组） |
| `-G, --group gid,...` | 只匹配真实组 ID |
| `-u, --euid euid,...` | 只匹配**有效**用户 ID |
| `-U, --uid uid,...` | 只匹配**真实**用户 ID |
| `-s, --session sid,...` | 只匹配指定会话 ID（0 表示自身会话） |
| `-t, --terminal term,...` | 只匹配指定控制终端（**不带 `/dev/` 前缀**） |
| `-r, --runstates D,R,S,Z,...` | 只匹配指定进程状态 |
| `-v, --inverse` | 取反匹配（**在 pkill 下短选项被禁用**，避免误用） |
| `-w, --lightweight` | 显示所有线程 ID 而非 PID（pkill 下被禁用） |
| `-q, --queue value` | 随信号发送整数 value（`pidfd_send_signal` 或 `sigqueue`） |
| `--cgroup name,...` | 按 cgroup v2 名匹配 |
| `--env name[=value],...` | 按环境变量匹配 |
| `--ns pid` | 匹配属于同一命名空间的进程（匹配其他用户需 root） |
| `--nslist name,...` | 限定要匹配的命名空间：`ipc mnt net pid user uts` |
| `-V, --version` | 版本 |
| `-h, --help` | 帮助 |

### 示例

```bash
pkill -f 'java.*MyApp'            # 按完整命令行匹配（最常用）
pkill -9 -f 'python.*train.py'    # 强制杀
pkill -u deploy                   # 杀掉 deploy 用户的所有进程
pkill -x nginx                    # 进程名必须完全等于 nginx
pkill -HUP nginx                  # 发 HUP 重载
pkill -o -f 'java'                # 只杀最老的那个 java
pkill -n -f 'java'                # 只杀最新的那个 java
pkill -e -f 'java'                # 显示被杀的名字与 PID
pkill -t pts/2                    # 杀掉某终端上的进程
pkill -f 'node .*server.js'       # 正则（POSIX ERE）
pgrep -af java                    # 先看看会匹配到什么（-a 显示完整命令行）
pgrep -u root -l sshd             # 只列 PID 与名字
pkill --older 3600 -f myworker    # 只杀启动超过 1 小时的
```

### 易踩坑

- **`pkill -f` 会匹配到自己的命令行**！例如 `pkill -f 'python'` 在某些情况下会误伤执行它的脚本。`-A` 可忽略祖先进程。
- 匹配模式是 **POSIX 扩展正则**，`-f 'java.*'` 里的 `.` 是通配 —— 想精确用 `-x`。
- **先 `pgrep -af` 预览再 `pkill`**，这是最稳的习惯。
- `pkill` 默认只匹配进程名（≤15 字符），长命令行必须 `-f`。
- `-v` 在 pkill 下短选项被禁用，只能用 `--inverse`。

**BSD/macOS**：macOS 的 `pkill` 无 `-A`、`-H`、`-m`、`-q`、`--cgroup`、`--env`、`--ns`；`-F` 存在但行为略有差异。

---

## 6. jobs / fg / bg — 任务控制（shell 内建）

**语法**：`jobs [-lnprs] [jobspec ...]` / `jobs -x command [args ...]` / `fg [jobspec]` / `bg [jobspec ...]`

这是 **shell 内建**，不是外部命令。核心概念：shell 把每个**管道**当作一个作业，作业号从 1 开始，`jobs` 用方括号显示。

### 三个命令

| 命令 | 作用 |
|---|---|
| `jobs` | 列出活动作业 |
| `fg [jobspec]` | 把作业**调回前台**并成为当前作业；返回该命令的退出状态 |
| `bg [jobspec ...]` | 让挂起的作业在**后台继续**（如同用 `&` 启动）；无参数时作用于当前作业 |

### jobs 的选项

| 选项 | 含义 |
|---|---|
| `-l` | 除常规信息外还列出进程 ID |
| `-n` | 只显示自上次通知后**状态发生变化**的作业 |
| `-p` | 只列出作业进程组首进程的 PID |
| `-r` | 只显示**运行中**的作业 |
| `-s` | 只显示**已停止**的作业 |
| `-x command [args]` | 把 command/args 中的 jobspec 替换为对应进程组 ID 后执行，返回其退出状态 |

### jobspec（作业标识）语法

| 形式 | 含义 |
|---|---|
| `%n` | 作业号 n |
| `%%` / `%+` / `%` | **当前作业** |
| `%-` | **前一个作业** |
| `%string` | 以 string **开头**的作业 |
| `%?string` | 命令行中**包含** string 的作业 |

> 前缀或子串匹配到多个作业时，bash 会报错。

### 相关的 shell 机制

| 操作 | 说明 |
|---|---|
| `cmd &` | 后台启动，shell 打印 `[1] 25647`（作业号 + 末进程 PID） |
| `Ctrl-Z` | 挂起前台进程（发 SIGTSTP），回到 shell |
| `Ctrl-C` | 中断前台进程（发 SIGINT） |
| `Ctrl-Y` | 延迟挂起（进程下次读终端时停止） |
| `wait [jobspec]` | 等待作业结束 |
| `disown` | 把作业从作业表中移除（免受 SIGHUP） |
| `kill %1` | 杀掉 1 号作业 |

### 示例

```bash
sleep 300 &                  # 后台运行
jobs                         # 列出作业
jobs -l                      # 带 PID
jobs -r                      # 只看运行中的
jobs -s                      # 只看已停止的
# 按 Ctrl-Z 挂起当前任务
jobs                         # 显示 [1]+  Stopped   sleep 300
bg %1                        # 让它后台继续
fg %1                        # 调回前台
fg                           # 不带参数 = 当前作业
kill %1                      # 杀掉 1 号作业
nohup ./long_task.sh &       # 后台 + 免疫挂断
wait                         # 等所有后台作业结束
```

### 易踩坑

- **关掉终端后，后台作业默认会收到 SIGHUP 而退出** —— 想真正不挂断用 `nohup`、`setsid` 或 `disown`。
- `bg`/`fg` 只在**作业控制启用**的交互式 shell 里有效；脚本中默认关闭（`set -m` 可开）。
- 后台进程尝试**读终端**会被内核发 `SIGTTIN` 挂起；写终端（若 `stty tostop`）会收到 `SIGTTOU`。
- `jobs` 输出里 `+` 是当前作业、`-` 是前一个作业。

**BSD/macOS**：bash/zsh 的作业控制行为一致；但 **zsh 的 `%` 语义更丰富**（支持 `%?string`、`%+`/`%-` 等，且 `jobs` 输出格式不同）。

---

## 7. nohup — 后台不挂断运行

**语法**：`nohup COMMAND [ARG]...` / `nohup OPTION`

让 COMMAND **忽略挂断信号（SIGHUP）**，输出重定向到非终端。

### 选项

| 选项 | 含义 |
|---|---|
| `--help` | 显示帮助并退出 |
| `--version` | 输出版本信息并退出 |

### 重定向行为（手册原文要点）

- 若标准输入是终端 → 从一个不可读文件重定向
- 若标准输出是终端 → 追加输出到 `nohup.out`（若不可写则用 `$HOME/nohup.out`）
- 若标准错误是终端 → 重定向到标准输出
- 想保存到指定文件：`nohup COMMAND > FILE`

### 退出状态

| 码 | 含义 |
|---|---|
| 125 | `nohup` 命令自身失败 |
| 126 | COMMAND 找到但无法调用 |
| 127 | COMMAND 找不到 |
| 其他 | COMMAND 自身的退出状态 |

### 示例

```bash
nohup ./long_task.sh &                # 后台运行，输出进 nohup.out
nohup ./long_task.sh > task.log 2>&1 &   # 输出到指定文件（推荐写法）
nohup python train.py > train.log 2>&1 &
nohup ./server --port 8080 </dev/null >server.log 2>&1 &
```

### 易踩坑

- **`nohup` 只免疫 SIGHUP**，不解决“终端关闭后标准输入被关闭”的问题 —— 稳妥写法是显式 `</dev/null` 并把 stdout/stderr 都重定向。
- 不加 `2>&1`，标准错误仍可能指向终端。
- `nohup` 不带 `&` 时是**前台**运行，只是免疫 HUP；想后台必须自己加 `&`。
- `nohup.out` 会**追加**而非覆盖，日志会越滚越大。
- 更彻底的方式：`setsid ./cmd &`（脱离控制终端）或 `systemd-run`。

**BSD/macOS**：`nohup` 行为基本一致，`--help`/`--version` 的 GNU 长选项在 macOS 上不可用。

---

## 8. systemctl — systemd 服务管理

**语法**：`systemctl [OPTIONS...] COMMAND [UNIT...]`

### 常用子命令

| 子命令 | 作用 |
|---|---|
| `status UNIT` | 查看单元状态（含最近日志） |
| `start` / `stop` / `restart` | 启动 / 停止 / 重启 |
| `reload` | 重载配置（不重启进程） |
| `enable` / `disable` | 设置 / 取消开机自启 |
| `is-active` / `is-enabled` / `is-failed` | 查询状态（返回退出码，适合脚本） |
| `list-units` | 列出已加载单元 |
| `list-unit-files` | 列出所有已安装单元文件 |
| `list-dependencies` | 列出依赖 |
| `daemon-reload` | 重新加载 unit 文件（改完 `.service` 必须执行） |
| `mask` / `unmask` | 屏蔽 / 解除屏蔽（比 disable 更强，无法被依赖拉起） |
| `cat UNIT` | 显示 unit 文件内容 |
| `edit UNIT` | 编辑 unit（生成 override 片段） |
| `show UNIT` | 显示单元属性（可 `-p` 过滤） |
| `kill UNIT` | 向单元进程发信号 |
| `clean UNIT` | 清理单元资源 |
| `reset-failed` | 清除失败状态 |
| `list-timers` | 列出定时器 |
| `list-jobs` | 列出排队作业 |
| `halt` / `poweroff` / `reboot` | **关机 / 关机 / 重启（危险）** |
| `suspend` / `hibernate` / `hybrid-sleep` | 休眠相关 |
| `default` / `rescue` / `emergency` | 切换运行目标 |
| `preset` / `preset-all` | 按预设启用/禁用 |

### 选项（节选，手册共 66 项）

| 选项 | 含义 |
|---|---|
| `-t, --type=` | 限定单元类型（`service`、`socket` 等，逗号分隔） |
| `--state=` | 限定 LOAD/SUB/ACTIVE 状态（`--state=failed` 或 `--failed`） |
| `-p, --property=` | 配合 `show`，限定显示的属性 |
| `-P` | 等价 `--value --property=`，只显示值（v246+） |
| `-a, --all` | 列出时也显示非活动单元；`show` 时显示所有属性 |
| `-r, --recursive` | 也显示本地容器中的单元 |
| `--reverse` | `list-dependencies` 时显示反向依赖 |
| `--after` / `--before` | `list-dependencies` 时显示之后 / 之前的单元 |
| `--with-dependencies` | `status`/`cat`/`list-units`/`list-unit-files` 时一并打印依赖 |
| `-l, --full` | 不省略单元名、进程树、日志、描述 |
| `--value` | `show` 时只打印值 |
| `--job-mode=` | 排队新作业时如何处理已有作业（`fail`/`replace`/`isolate`/`flush` 等，默认 `replace`） |
| `-T, --show-transaction` | 入队时显示所有相关作业的简要信息 |
| `--fail` | `--job-mode=fail` 的简写 |
| `--check-inhibitors=` | 关机/休眠时是否检查 inhibitor 锁（`auto`/`yes`/`no`） |
| `-i` | `--check-inhibitors=no` 的简写 |
| `--dry-run` | 只打印将要做的事（`halt`/`poweroff`/`reboot` 等支持） |
| `-q, --quiet` | 抑制命令结果与截断日志的提示（错误仍打印） |
| `-v, --verbose` | 执行单元操作时显示单元日志（v258+） |
| `--no-warn` | 不产生默认警告 |
| `--no-block` | 不同步等待操作完成（只验证并入队） |
| `--wait` | 配合 `start`/`restart`，同步等待启动的单元再次终止 |
| `--user` | 与当前用户的服务管理器通信 |
| `--system` | 与系统服务管理器通信（**默认**） |
| `--failed` | 列出失败状态的单元（等价 `--state=failed`） |
| `--no-wall` | 关机/重启前不发 wall 消息 |
| `--global` | `enable`/`disable` 时操作全局用户配置目录 |
| `--no-reload` | `enable`/`disable`/`preset`/`mask`/`unmask` 后不隐式重载 |
| `--kill-whom=` | `kill` 时选择目标：`main`/`control`/`cgroup`/`all` |
| `--kill-value=INT` | `kill` 时随信号发送整数值（仅实时信号） |
| `--kill-subgroup=PATH` | `kill` 时指定 cgroup 子路径 |
| `-s, --signal=` | `kill` 时指定信号（默认 SIGTERM；`help` 列出可选值） |
| `--what=` | `clean` 时选择要清理的资源类型 |
| `-f, --force` | 覆盖冲突符号链接 / 创建不存在的单元 / 忽略 logind 错误 |
| `--message=` | `halt`/`poweroff`/`reboot` 时附带说明消息 |
| `--now` | `enable`/`disable`/`mask` 后立即 start/stop |
| `--root=` | 操作指定根路径下的 unit 文件（直接操作文件系统） |
| `--image=` / `--image-policy=` | 操作磁盘镜像中的系统 |
| `--runtime` | 只做临时更改（重启即失效） |
| `--preset-mode=` | `preset` 行为：`full`/`enable-only`/`disable-only` |
| `-n, --lines=` | `status` 时显示多少行日志（默认 10，0 表示禁用） |
| `-o, --output=` | `status` 时日志格式（取值同 `journalctl`，默认 `short`） |
| `--firmware-setup` | 重启进固件设置界面 |
| `--boot-loader-menu=timeout` | 下次启动显示引导菜单 |
| `--boot-loader-entry=ID` | 下次启动进入指定引导项 |
| `--reboot-argument=` | 架构/固件相关的重启参数 |
| `--kernel-cmdline=` | `kexec` 时追加内核命令行 |
| `--kernel-cmdline-reuse` | `kexec` 时复用当前内核命令行 |
| `--plain` | `list-*` 时用列表而非树，省略圆点 |
| `--timestamp=` | 时间戳格式（`pretty`/`unix`/`us`/`utc`/`utc-us`） |
| `--mkdir` / `--read-only` | `bind` 时创建目标 / 只读绑定挂载 |
| `--drop-in=NAME` | `edit` 时用 NAME 作为 drop-in 文件名 |
| `--when=` | `halt`/`poweroff`/`reboot`/`kexec` 时定时执行 |
| `--stdin` | `edit` 时从 stdin 读内容（脚本用） |
| `-H, --host=` | 远程执行（`user@host:port/container`） |
| `-M, --machine=` | 在本地容器中执行 |
| `-C, --capsule=` | 在 capsule 中执行 |
| `--no-ask-password` | 特权操作不询问密码 |
| `--no-pager` | 输出不分页 |
| `--legend=BOOL` | 是否打印图例 |
| `-h, --help` | 帮助 |
| `--version` | 版本 |

### 示例

```bash
systemctl status nginx                  # 看状态
systemctl start nginx                   # 启动
systemctl stop nginx                    # 停止
systemctl restart nginx                 # 重启
systemctl reload nginx                  # 重载配置（不重启进程）
systemctl enable --now nginx            # 设为自启并立即启动
systemctl disable nginx                 # 取消自启
systemctl is-active nginx               # 脚本里判断（退出码 0=active）
systemctl is-enabled nginx              # 是否自启
systemctl daemon-reload                 # 改完 unit 文件后必须执行
systemctl list-units --type=service --state=running   # 运行中的服务
systemctl list-units --failed           # 失败的服务
systemctl list-unit-files --type=service              # 所有已安装服务
systemctl cat nginx                     # 查看 unit 文件
systemctl edit nginx                    # 生成 override 片段
systemctl show nginx -p MainPID         # 取主进程 PID
systemctl list-timers                   # 定时器
systemctl --user status myapp           # 用户级服务
systemctl -H user@host status nginx     # 远程操作
systemctl mask nginx                    # 屏蔽（比 disable 更彻底）
```

### 易踩坑

- **改完 `/etc/systemd/system/*.service` 必须 `systemctl daemon-reload`**，否则不生效。
- `systemctl stop` **不等于** `disable`：stop 只停当前，disable 才取消自启。
- `disable` 后仍可能被其他单元依赖拉起；彻底屏蔽用 `mask`。
- `enable` 只是创建符号链接，**不会启动**；要同时启动用 `enable --now`。
- `systemctl` 只在 **systemd 系统**上存在 —— Alpine（OpenRC）、WSL1、容器内通常没有。
- ⚠️ **`systemctl halt` / `poweroff` / `reboot` 会立即关机/重启**，在生产环境是最高危操作。

**BSD/macOS**：macOS **没有 systemd**，用 `launchctl`（`launchctl list` / `load` / `unload` / `kickstart`）替代。

---

## 9. journalctl — 查看 systemd 日志

**语法**：`journalctl [OPTIONS...] [MATCHES...]`

### 范围与来源

| 选项 | 含义 |
|---|---|
| `--system` | 显示系统服务与内核消息 |
| `--user` | 显示当前用户的服务消息 |
| `-M, --machine=` | 显示本地容器消息 |
| `-m, --merge` | 交错显示所有可用日志（含远程） |
| `-D DIR, --directory=DIR` | 操作指定日志目录 |
| `-i GLOB, --file=GLOB` | 操作匹配 GLOB 的日志文件（可多次） |
| `--root=ROOT` | 操作指定根目录下的日志层次 |
| `--image=` / `--image-policy=` | 操作磁盘镜像中的日志 |
| `--namespace=NAMESPACE` | 指定 journal 命名空间（`*` 表示全部） |

### 过滤

| 选项 | 含义 |
|---|---|
| `-S, --since=` / `-U, --until=` | 起始 / 结束时间（`"2012-10-30 18:17:16"`；可省略时间或秒） |
| `-b [[ID][±offset]\|all], --boot[=...]` | 指定某次启动（空参数=本次启动；`-1` 上次；`all` 全部） |
| `-u, --unit=UNIT\|PATTERN` | 按 systemd 单元过滤（可给模式） |
| `--user-unit=` | 按用户会话单元过滤 |
| `-I, --invocation=ID[±offset]\|offset` | 按某次调用过滤 |
| `-t, --identifier=SYSLOG_IDENTIFIER` | 按 syslog 标识符过滤（可多次） |
| `-T, --exclude-identifier=` | 排除指定标识符（v256+） |
| `-p, --priority=` | 按优先级过滤（`0`/`emerg` … `7`/`debug`，或 `FROM..TO` 区间） |
| `--facility=` | 按 syslog facility 过滤（逗号分隔；`help` 列出可选值） |
| `-g, --grep=` | MESSAGE 字段按 **PCRE** 正则过滤（**全小写模式则大小写不敏感**） |
| `--case-sensitive[=BOOL]` | 指定模式匹配是否大小写敏感 |
| `-k, --dmesg` | 只看内核消息（隐含 `--boot=0`） |
| `-c, --cursor=` | 从指定游标处开始 |
| `--after-cursor=` | 从指定游标之后开始 |
| `--cursor-file=FILE` | 从文件读游标开始，结束时把最后一条的游标写回文件 |

### 输出格式

| 选项 | 含义 |
|---|---|
| `-o, --output=` | 输出格式：`short`(默认) `short-full` `short-iso` `short-iso-precise` `short-precise` `short-monotonic` `short-unix` `verbose` `export` `json` `json-pretty` `json-sse` `json-seq` `cat` `with-unit` |
| `--output-fields=` | 限定输出哪些字段（对 verbose/export/json*/cat 有效） |
| `-n, --lines=` | 显示最近 N 条（正整数或 `all`；`+N` 表示最旧的 N 条；默认 10） |
| `-r, --reverse` | 反转输出（最新的在最前） |
| `--show-cursor` | 最后一条后显示游标 |
| `--utc` | 用 UTC 时间 |
| `-x, --catalog` | 附加消息目录中的解释文本 |
| `-W, --no-hostname` | 不显示主机名字段（仅 short 系列） |
| `--no-full` / `--full` / `-l` | 省略 / 完整显示超长字段 |
| `-a, --all` | 完整显示所有字段（含不可打印字符） |
| `--truncate-newline` | 每条消息在第一个换行处截断（v254+） |
| `-q, --quiet` | 抑制所有信息性消息 |
| `--no-pager` | 不分页 |
| `-e, --pager-end` | 立即跳到日志末尾（隐含 `--lines=1000` 与 `--boot=0`） |
| `-f, --follow` | 只显示最新条目并持续跟踪新条目（`Ctrl-C` 退出） |
| `--no-tail` | 跟踪模式下也显示所有已存输出（撤销 `--lines=`） |
| `--synchronize-on-exit=` | `--follow` 模式下收到 SIGTERM/SIGINT 时请求同步日志 |

### 维护与查询

| 选项 | 含义 |
|---|---|
| `-N, --fields` | 打印日志中所有字段名 |
| `-F, --field=FIELD` | 打印某字段所有可能取值 |
| `--list-boots` | 列出启动编号、ID 与首末消息时间 |
| `--list-invocations` | 列出单元的调用 ID（需配合 `-u`） |
| `--disk-usage` | 显示所有日志文件的磁盘占用 |
| `--vacuum-size=` | 删除最旧归档日志直到占用低于指定大小（`K/M/G/T`，按 1024） |
| `--vacuum-time=` | 删除早于指定时间的归档日志 |
| `--vacuum-files=` | 保留最多 N 个归档日志文件 |
| `--verify` / `--verify-key=` | 校验日志文件一致性 / 用 FSS 验证密钥校验真实性 |
| `--sync` | 让 journald 把所有未写入数据落盘并同步 |
| `--flush` | 把 `/run/log/journal/` 数据刷到 `/var/log/journal/` |
| `--relinquish-var` / `--smart-relinquish-var` | 让守护进程改回写 `/run/log/journal/` |
| `--rotate` | 轮转日志文件 |
| `--header` | 显示内部头部信息而非日志内容 |
| `--list-catalog` / `--dump-catalog` / `--update-catalog` | 消息目录操作 |
| `--setup-keys` / `--interval=` / `--force` | FSS 密钥操作 |
| `-h, --help` | 帮助 |
| `--version` | 版本 |

### 示例

```bash
journalctl -u nginx                  # 看 nginx 服务日志
journalctl -u nginx -f               # 实时跟踪
journalctl -u nginx --since today    # 今天的
journalctl -u nginx --since "2026-10-14 09:00" --until "2026-10-14 10:00"
journalctl -u nginx -n 100           # 最近 100 条
journalctl -u nginx -r               # 倒序（最新在前）
journalctl -p err -b                 # 本次启动的错误及以上
journalctl -p err..alert             # 优先级区间
journalctl -k                        # 内核消息（等价 dmesg）
journalctl -k -b -1                  # 上次启动的内核消息
journalctl -g 'timeout' -u nginx     # 正则过滤 MESSAGE
journalctl --since "1 hour ago"      # 最近一小时
journalctl -b -1                     # 上次启动
journalctl --list-boots              # 所有启动
journalctl -o json-pretty -n 1       # JSON 格式看一条
journalctl -o verbose -n 1           # 详细字段
journalctl -u nginx -o cat           # 只要消息正文
journalctl --disk-usage              # 日志占多少磁盘
journalctl --vacuum-size=500M        # 清理到 500M 以内
journalctl --vacuum-time=7d          # 只保留 7 天
journalctl -f -u nginx -u php-fpm    # 同时跟踪多个服务
journalctl _PID=1234                 # 按字段精确匹配
journalctl /usr/bin/dbus-daemon      # 按可执行文件路径
```

### 易踩坑

- **`journalctl -u` 默认显示全部历史**，大日志会刷屏 —— 先加 `-n` 或 `--since`。
- 普通用户默认**看不到系统日志**，需加入 `systemd-journal` 组或 `sudo`。
- `--vacuum-*` 会**真正删除**归档日志，不可恢复；`--rotate` 只是轮转不删除。
- `-g` 用 PCRE 且**全小写模式自动大小写不敏感**，这个“贴心”行为常让人困惑。
- 日志默认存在 `/run/log/journal/`（**内存**，重启即失）；持久化需要 `/var/log/journal/` 目录存在，用 `--flush` 迁移。
- 内核命令行 `systemd.journald.forward_to_syslog` 等配置会影响是否同时写 `/var/log/messages`。

**BSD/macOS**：macOS 没有 journald，用 `log show --predicate 'process == "nginx"' --last 1h` 或 `/var/log/system.log`。

---

## 10. free — 内存使用

**语法**：`free [option ...]`

数据来自解析 `/proc/meminfo`。

### 列的含义（手册原文）

| 列 | 含义 |
|---|---|
| `total` | 可用总内存（MemTotal 与 SwapTotal）——物理与交换内存减去少量保留位与内核二进制代码 |
| `used` | 已用或不可用内存（计算为 `total - available`） |
| `free` | 未使用内存（MemFree 与 SwapFree） |
| `shared` | （主要）被 tmpfs 使用的内存（Shmem） |
| `buffers` | 内核缓冲区使用的内存（Buffers） |
| `cache` | 页缓存与 slab 使用的内存（Cached + SReclaimable） |
| `buff/cache` | buffers 与 cache 之和 |
| `available` | **估算的、可用来启动新应用而不触发交换的内存**（MemAvailable） |

> `available` 与 `free` 的关键差别：它把**可回收的页缓存与部分 slab** 也算进去了，所以更接近真实可用量。内核 3.14 起提供，2.6.27+ 模拟，更早则等同 `free`。

### 选项

| 选项 | 含义 |
|---|---|
| `-b, --bytes` | 以字节显示 |
| `-k, --kibi` | 以 KiB 显示（**默认**） |
| `-m, --mebi` | 以 MiB 显示 |
| `-g, --gibi` | 以 GiB 显示 |
| `--tebi` | 以 TiB 显示 |
| `--pebi` | 以 PiB 显示 |
| `--kilo` | 以 KB 显示（隐含 `--si`） |
| `--mega` | 以 MB 显示（隐含 `--si`） |
| `--giga` | 以 GB 显示（隐含 `--si`） |
| `--tera` | 以 TB 显示（隐含 `--si`） |
| `--peta` | 以 PB 显示（隐含 `--si`） |
| `-h, --human` | 自动缩放到最短的三位数单位（`B`/`Ki`/`Mi`/`Gi`/`Ti`/`Pi`） |
| `-w, --wide` | 宽模式（行长超 80 字符，buffers 与 cache 分成两列） |
| `-c, --count count` | 显示 count 次（需配合 `-s`） |
| `-l, --lohi` | 显示详细的 low / high 内存统计 |
| `-L, --line` | 单行输出（常配合 `-s` 反复显示） |
| `-s, --seconds delay` | 每 delay 秒持续显示（支持浮点，用 `.` 或 `,`） |
| `--si` | 用 1000 进制（kilo/mega/giga）代替 1024 进制 |
| `-t, --total` | 显示列总计行 |
| `-v, --committed` | 显示内存提交上限与已提交/未提交内存（**关闭 overcommit 时才有意义**） |
| `--help` | 帮助 |
| `-V, --version` | 版本 |

### 示例

```bash
free                    # 默认 KiB
free -h                 # 人类可读（最常用）
free -m                 # MiB
free -h -w              # 宽模式，buffers 与 cache 分开
free -h -s 2            # 每 2 秒刷新
free -h -t              # 带总计行
free -h -l              # 含 low/high 内存
free -s 1 -c 5          # 每秒 1 次，共 5 次
```

### 易踩坑

- **看 `available`，不要看 `free`**。Linux 会把空闲内存大量用于页缓存，`free` 很小是正常的。
- `buff/cache` 里的缓存**在需要时会被回收**，不算“被占用”。
- `-h` 用的是 1024 进制（Ki/Mi/Gi），`--si`/`-H` 才是 1000 进制 —— 与硬盘厂商口径不同。
- 交换分区 `used` 持续增长且 `available` 很低时，说明内存真的吃紧。

**BSD/macOS**：macOS **没有 `free`**。用 `vm_stat`、`top`、`memory_pressure` 或 `sysctl hw.memsize` 替代。

---

## 11. uptime — 运行时长 / 负载

**语法**：`uptime [option ...]`

单行显示：当前时间、系统运行时长、当前登录用户数、过去 1/5/15 分钟的系统负载均值。与 `w(1)` 的头部行是同一份信息。

### 负载均值的含义（手册原文要点）

> 系统负载均值是**处于可运行或不可中断状态的进程的平均数量**。可运行状态指正在使用 CPU 或等待使用 CPU；不可中断状态指等待 I/O 访问（例如等磁盘）。均值取三个时间区间。

> **负载均值没有按 CPU 数量归一化**：负载 1 在单核系统上表示一直满载，而在 4 核系统上表示有 75% 的时间是空闲的。

### 选项

| 选项 | 含义 |
|---|---|
| `-c, --container` | 显示容器运行时长而非系统运行时长 |
| `-p, --pretty` | 以易读格式显示 |
| `-r, --raw` | 原始格式（当前时间与运行时长以秒为单位） |
| `-s, --since` | 系统启动时刻，格式 `yyyy-mm-dd HH:MM:SS` |
| `-h, --help` | 帮助 |
| `-V, --version` | 版本 |

> 环境变量 `PROCPS_CONTAINER` 被设置时，`uptime` 等价于给了 `--container`。

### 示例

```bash
uptime                  # 默认单行
uptime -p               # up 3 days, 4 hours, 12 minutes
uptime -s               # 系统启动时刻
uptime -r               # 原始秒数
uptime --container      # 容器内运行时长
```

### 易踩坑

- **负载均值不是百分比，要除以 CPU 核数**。8 核机器负载 8.0 才是满载。
- 负载包含 `D`（不可中断 I/O 等待）状态的进程 —— 所以**磁盘 I/O 打满也会推高负载**，即使 CPU 空闲。
- `-p`（pretty）与 `-s`（since）的输出用途不同，别混。
- 数据源是 `/proc/loadavg` 与 `/proc/uptime`。

**BSD/macOS**：macOS 有 `uptime`，但**无 `-p`/`-s`/`-r`/`--container`**（这些是 procps-ng 的扩展）。

---

## 12. uname — 系统 / 内核信息

**语法**：`uname [OPTION]...`

### 选项

| 选项 | 含义 |
|---|---|
| `-a, --all` | 按以下顺序打印所有信息（`-p` 与 `-i` 未知时省略） |
| `-s, --kernel-name` | 内核名 |
| `-n, --nodename` | 网络节点主机名 |
| `-r, --kernel-release` | 内核发行版本号 |
| `-v, --kernel-version` | 内核版本 |
| `-m, --machine` | 机器硬件名（架构） |
| `-p, --processor` | 处理器类型（**非可移植**） |
| `-i, --hardware-platform` | 硬件平台（**非可移植**） |
| `-o, --operating-system` | 操作系统 |
| `--help` | 帮助 |
| `--version` | 版本 |

> `-a` 的输出顺序：`-s -n -r -v -m -p -i -o`。

### 示例

```bash
uname -a                 # 全部信息（最常用）
uname -r                 # 内核版本，如 6.8.0-45-generic
uname -m                 # 架构，如 x86_64 / aarch64
uname -s                 # 内核名，通常是 Linux
uname -sr                # 内核名 + 版本
uname -o                 # 操作系统，GNU/Linux
```

### 易踩坑

- `-p` 与 `-i` 在现代 Linux 上常输出 `unknown` —— 这是正常的，它们是历史遗留项。
- 判断发行版**不能靠 `uname`** —— 用 `cat /etc/os-release` 或 `lsb_release -a`。
- 判断架构用 `-m`（`x86_64`/`aarch64`），别用 `-p`。

**BSD/macOS**：macOS 的 `uname` **没有 `-o`**；`uname -a` 输出格式不同（含 Darwin 版本）。

---

## 13. lscpu — CPU 信息

**语法**：`lscpu [options]`

数据来自 `/proc/cpuinfo`、`/sys/devices/system/cpu/` 等。

### 选项

| 选项 | 含义 |
|---|---|
| `-a, --all` | 输出中包含在线与离线 CPU（`-e` 的默认行为）；只能配合 `-e` 或 `-p` |
| `-b, --online` | 只输出在线 CPU（`-p` 的默认行为）；只能配合 `-e` 或 `-p` |
| `-c, --offline` | 只输出离线 CPU；只能配合 `-e` 或 `-p` |
| `-e, --extended[=list]` | 以人类可读格式显示 CPU 信息（可指定列，如 `-e=cpu,node`） |
| `-p, --parse[=list]` | 优化为易解析输出（可指定列；指定时缓存列用 `:` 分隔） |
| `-C, --caches[=list]` | 显示 CPU 缓存详情（可指定列，如 `-C=NAME,ONE-SIZE`） |
| `-H, --list-columns` | 列出 `--extended`/`--parse`/`--caches` 可用的列 |
| `--output-all` | 输出所有可用列（需配合 `-e`/`-p`/`-C`） |
| `-r, --raw` | 原始格式输出（仅对 `-e`/`-p`/`-C` 有效） |
| `-B, --bytes` | 以字节而非人类可读形式打印大小 |
| `-x, --hex` | CPU 集合用十六进制掩码（如 `ff`），默认是列表格式（如 `0,1`） |
| `-y, --physical` | 显示所有拓扑列的**物理** ID（内核提供，不保证唯一或连续） |
| `-s, --sysroot directory` | 收集其他 Linux 实例的 CPU 数据（指定其系统根） |
| `-J, --json` | JSON 输出格式 |
| `--hierarchic[=when]` | 汇总输出中使用子节（`never`/`always`/`auto`，默认终端下用子节） |
| `--annotate[=when]` | 给列名加注解（`always`/`never`/`auto`） |
| `--arm-id[=list]` | 打印已知 ARM 核实现者及其名称 |
| `--arm-model id` | 打印指定核型号名称（需配合 `--arm-id=id`） |
| `-h, --help` | 帮助 |
| `-V, --version` | 版本 |

### 常用输出字段

| 字段 | 含义 |
|---|---|
| `Architecture` | 架构（`x86_64`、`aarch64`） |
| `CPU(s)` | 逻辑 CPU 总数 |
| `Thread(s) per core` | 每核线程数（超线程） |
| `Core(s) per socket` | 每插槽核数 |
| `Socket(s)` | 物理 CPU 插槽数 |
| `NUMA node(s)` | NUMA 节点数 |
| `CPU MHz` | 当前频率 |
| `CPU max MHz` / `CPU min MHz` | 最大 / 最小频率 |
| `BogoMIPS` | 粗略性能指标 |
| `Virtualization` | 虚拟化类型（`VT-x`/`AMD-V`） |
| `Hypervisor vendor` | 虚拟化厂商（在虚机里才显示） |
| `L1d/L1i/L2/L3 cache` | 各级缓存大小 |
| `Flags` | CPU 特性标志（`avx512f`、`sse4_2` 等） |

### 示例

```bash
lscpu                        # 默认汇总
lscpu | grep -E 'Model name|Socket|Core|Thread'   # 看型号与拓扑
lscpu -e                     # 每个逻辑 CPU 一行
lscpu -e=cpu,node,CACHE      # 指定列
lscpu -p                     # 易解析格式（适合脚本）
lscpu -C                     # 缓存详情
lscpu -J                     # JSON
lscpu -e -b                  # 只看在线 CPU
lscpu -B                     # 大小以字节显示
lscpu -y                     # 物理 ID
```

### 易踩坑

- `lscpu` 的 `CPU(s)` 是**逻辑 CPU 数**（含超线程），物理核数要自己算：`Socket(s) × Core(s) per socket`。
- 虚机里 `Model name` 可能显示宿主 CPU 型号，`Hypervisor vendor` 才说明是虚机。
- `-p` 的缓存列在**不指定 list 时**用两个逗号分隔（兼容旧版），指定 list 时用 `:`。
- `-a`/`-b`/`-c` **必须**配合 `-e` 或 `-p`，单独用会报错。

**BSD/macOS**：macOS 没有 `lscpu`。用 `sysctl -n machdep.cpu.brand_string`、`sysctl -n hw.ncpu`、`sysctl -n hw.physicalcpu`。

---

## 14. lsblk — 块设备列表

**语法**：`lsblk [options] [device...]`

以树状（默认）列出块设备及其分区、挂载点。**不会列出 RAM 盘**（除非 `-a`）。

### 选项

| 选项 | 含义 |
|---|---|
| `-a, --all` | 禁用所有内置过滤，列出所有空设备与 RAM 盘 |
| `-A, --noempty` | 不打印空设备 |
| `-b, --bytes` | 大小以字节而非人类可读形式显示 |
| `-d, --nodeps` | 不打印持有者/从属设备（如 `lsblk -d /dev/sda` 只看 sda 本身） |
| `-D, --discard` | 打印每个设备的丢弃能力（TRIM、UNMAP） |
| `-E, --dedup column` | 用该列作为去重键对输出树去重（多路径设备常用 `-E WWN`） |
| `-e, --exclude list` | 排除指定主设备号的设备（逗号分隔）；**过滤只作用于顶层设备** |
| `-I, --include list` | 只包含指定主设备号的设备 |
| `-f, --fs` | 输出文件系统信息（等价 `-o NAME,FSTYPE,FSVER,LABEL,UUID,FSAVAIL,FSUSE%,MOUNTPOINTS`） |
| `-i, --ascii` | 用 ASCII 字符画树 |
| `-J, --json` | JSON 输出（建议配合 `--output`，必要时加 `--tree`） |
| `-l, --list` | 列表形式输出（不体现设备间关系；2.34 起每个设备只打印一次） |
| `-m, --perms` | 输出设备属主、属组、权限（等价 `-o NAME,SIZE,OWNER,GROUP,MODE`） |
| `-M, --merge` | 合并子树父节点（RAID 与多路径更易读，需树状输出） |
| `-N, --nvme` | 只输出 NVMe 设备 |
| `-v, --virtio` | 只输出 virtio 设备 |
| `-n, --noheadings` | 不打印表头 |
| `-o, --output list` | 指定输出列（`--list-columns` 可查；`+list` 表示在默认列基础上追加） |
| `-O, --output-all` | 输出所有可用列 |
| `-P, --pairs` | 以 `key="value"` 键值对输出（不安全字符十六进制转义） |
| `-p, --paths` | 打印完整设备路径 |
| `-Q, --filter expression` | 只打印满足表达式的设备（**先过滤后取数，比 grep 高效**） |
| `--highlight expression` | 给匹配行着色（实验特性） |
| `--ct name[:parameter[:function]]` | 定义自定义计数器（函数：`count`/`min`/`max`/`sum`） |
| `--ct-filter expression` | 为下一个计数器定义限制（实验特性） |
| `-r, --raw` | 原始格式输出（NAME/KNAME/LABEL/PARTLABEL/MOUNTPOINT 中的不安全字符转义） |
| `-S, --scsi` | 只输出 SCSI 设备（忽略所有分区、从属与持有设备） |
| `-s, --inverse` | 逆序打印依赖关系 |
| `-T, --tree[=column]` | 强制树状输出（可指定用哪列画树，默认 NAME） |
| `-t, --topology` | 输出块设备拓扑（等价 `-o NAME,ALIGNMENT,MIN-IO,OPT-IO,PHY-SEC,LOG-SEC,ROTA,SCHED,RQ-SIZE,RA,WSAME`） |
| `-w, --width number` | 输出宽度（字符数） |
| `-x, --sort column` | 按列排序（会默认启用 `--list` 格式） |
| `-y, --shell` | 列名改为只含 shell 变量允许的字符（`MIN_IO` 而非 `MIN-IO`） |
| `-z, --zoned` | 打印每个设备的 zone 相关信息 |
| `--sysroot directory` | 收集其他 Linux 实例的数据 |
| `--properties-by list` | 指定收集文件系统与分区表信息的方法（默认 `file,udev,blkid`） |
| `--hyperlink[=when]` | 路径以终端超链接形式打印 |
| `--annotate[=when]` | 给列名加注解 |
| `-H, --list-columns` | 列出 `--output` 可用的列 |
| `-h, --help` | 帮助 |
| `-V, --version` | 版本 |

### 常用列

`NAME` `KNAME` `PATH` `TYPE` `SIZE` `RO` `RM` `MODEL` `SERIAL` `WWN` `UUID` `FSTYPE` `FSVER` `LABEL` `PARTLABEL` `PARTUUID` `MOUNTPOINTS` `FSAVAIL` `FSUSE%` `OWNER` `GROUP` `MODE` `ROTA` `SCHED` `ALIGNMENT` `MIN-IO` `OPT-IO` `PHY-SEC` `LOG-SEC` `RQ-SIZE` `DISC-ALN` `DISC-GRAN` `DISC-MAX` `DISC-ZERO` `ZONED` `DAX`

### 示例

```bash
lsblk                        # 树状列出（最常用）
lsblk -f                     # 含文件系统类型、UUID、挂载点
lsblk -o NAME,SIZE,FSTYPE,MOUNTPOINTS,UUID
lsblk -o NAME,SIZE,TYPE,MOUNTPOINT -e 7   # 排除 loop 设备
lsblk -d                     # 只看磁盘，不展开分区
lsblk -p                     # 完整路径
lsblk -l                     # 列表形式
lsblk -J -o NAME,SIZE        # JSON
lsblk -P -o NAME,SIZE        # key="value" 对
lsblk -t                     # 拓扑信息（含 ROTA，判断 SSD/HDD）
lsblk -D                     # 丢弃/TRIM 能力
lsblk -S                     # 只列 SCSI
lsblk -N                     # 只列 NVMe
lsblk /dev/sda               # 只看某个设备
lsblk -Q 'TYPE=="part" && SIZE>1G'   # 过滤
lsblk -n -o SIZE /dev/sda1   # 只取大小（脚本用）
```

### 易踩坑

- `-e`/`-I` 的过滤**只作用于顶层设备** —— 在 `-l` 列表模式下容易产生困惑（层级不明显）。
- **默认不显示 RAM 盘（major=1）**，需要 `-a`。
- `-d` 只显示设备本身、不展开分区，与 `-l`（列表）不同。
- 判断 SSD/HDD 看 `ROTA` 列：`0`=SSD/NVMe，`1`=机械盘。
- `-f` 的信息来自 `blkid`，**未格式化的设备不会显示 FSTYPE**。

**BSD/macOS**：macOS 没有 `lsblk`。用 `diskutil list`。

---

## 15. df — 文件系统空间

**语法**：`df [OPTION]... [FILE]...`

显示每个文件名参数所在**文件系统**的可用空间。不给文件名则显示所有已挂载文件系统。**默认以 1K 块显示**（设了 `POSIXLY_CORRECT` 则用 512 字节块）。

### 输出列

`Filesystem` `1K-blocks`（或 `Size`） `Used` `Available` `Use%` `Mounted on`

### 选项

| 选项 | 含义 |
|---|---|
| `-a, --all` | 包含伪文件系统、重复文件系统、不可访问的文件系统 |
| `-h, --human-readable` | 以 **1024** 的幂显示大小（如 `1023M`） |
| `-H, --si` | 以 **1000** 的幂显示大小（如 `1.1G`） |
| `-B, --block-size=SIZE` | 按 SIZE 缩放（如 `-BM` 以 1,048,576 字节为单位） |
| `-k` | 等价 `--block-size=1K` |
| `-i, --inodes` | 显示 inode 信息而非块使用 |
| `-l, --local` | 只列出本地文件系统 |
| `-P, --portability` | 使用 POSIX 输出格式 |
| `-t, --type=TYPE` | 只列出指定类型的文件系统 |
| `-T, --print-type` | 打印文件系统类型 |
| `-x, --exclude-type=TYPE` | 排除指定类型的文件系统 |
| `--output[=FIELD_LIST]` | 用 FIELD_LIST 定义的输出格式，省略则打印所有字段 |
| `--total` | 省略对可用空间无意义的条目，并产生一个总计 |
| `--sync` | 获取使用信息前调用 sync |
| `--no-sync` | 不调用 sync（**默认**） |
| `-v` | （被忽略，为兼容保留） |
| `--help` | 帮助 |
| `--version` | 版本 |

> `SIZE` 也可通过环境变量 `DF_BLOCK_SIZE`、`BLOCK_SIZE`、`BLOCKSIZE` 指定。

### 示例

```bash
df -h                        # 人类可读（最常用）
df -hT                       # 带文件系统类型
df -h /                      # 只看根分区
df -h --total                # 带总计
df -i                        # inode 使用情况（排查“磁盘没满但写不进去”）
df -hT -x tmpfs -x devtmpfs  # 排除内存文件系统
df -h --output=source,fstype,size,used,avail,pcent,target   # 自定义列
df -h /data /var             # 看多个挂载点
df -l                        # 只看本地
df -B1 /                     # 以字节为单位（脚本用）
```

### 易踩坑

- **`Use%` 高但还有空间**：可能是**保留块**（ext4 默认给 root 保留 5%），用 `tune2fs -m` 调整。
- **磁盘有空间但写不进去** → 查 `df -i`，多半是 **inode 耗尽**（大量小文件）。
- `-h` 是 1024 进制，`-H` 是 1000 进制；与 `lsblk`/硬盘厂商标称容量口径不同。
- 挂载点路径**很深时列会被截断**，用 `df -h --output=target` 或加宽终端。
- 已删除但被进程占用的文件**不会释放空间**，`df` 仍显示满 —— 用 `lsof +L1` 找。

**BSD/macOS**：macOS 的 `df` **无 `-T`/`--total`/`--output`**；`-h` 存在；用 `df -H` 表示 1000 进制。

---

## 16. du — 目录占用

**语法**：`du [OPTION]... [FILE]...`

估算文件/目录的**磁盘使用量**（默认按块，不是表观大小）。**递归**，所以要配合 `-s` 或 `-d` 控制输出量。

### 选项

| 选项 | 含义 |
|---|---|
| `-s, --summarize` | 只显示每个参数的总计 |
| `-h, --human-readable` | 人类可读（`1K 234M 2G`） |
| `--si` | 类似 `-h`，但用 1000 的幂 |
| `-d, --max-depth=N` | 只显示深度 ≤ N 的目录总计；`-d 0` 等同 `--summarize` |
| `-a, --all` | 为所有文件（不只目录）写出计数 |
| `-c, --total` | 产生一个总计 |
| `-b, --bytes` | 等价 `--apparent-size --block-size=1` |
| `-A, --apparent-size` | 打印**表观大小**而非磁盘占用（稀疏文件、内部碎片、间接块会让两者不同） |
| `-B, --block-size=SIZE` | 按 SIZE 缩放 |
| `-k` / `-m` | 等价 `--block-size=1K` / `--block-size=1M` |
| `-L, --dereference` | 跟随**所有**符号链接 |
| `-D, --dereference-args` / `-H` | 只跟随**命令行上列出**的符号链接 |
| `-P, --no-dereference` | 不跟随任何符号链接（**默认**） |
| `-l, --count-links` | 硬链接时重复计数 |
| `-S, --separate-dirs` | 目录不计入子目录的大小 |
| `-x, --one-file-system` | 跳过不同文件系统上的目录 |
| `-t, --threshold=SIZE` | 排除小于 SIZE 的条目（负值则排除大于的） |
| `--exclude=PATTERN` | 排除匹配 PATTERN 的文件 |
| `-X, --exclude-from=FILE` | 排除匹配 FILE 中任一模式的文件 |
| `--time` | 显示目录（或子目录）中最近修改时间 |
| `--time=WORD` | 用 `atime`/`access`/`use`/`ctime`/`status` 替代修改时间 |
| `--time-style=STYLE` | 配合 `--time` 的时间格式 |
| `--inodes` | 列出 inode 使用而非块使用 |
| `-0, --null` | 每行以 NUL 结尾（配合 `xargs -0`） |
| `--files0-from=F` | 汇总 F 中 NUL 分隔文件名对应的占用 |
| `--help` | 帮助 |
| `--version` | 版本 |

> `SIZE` 也可通过 `DU_BLOCK_SIZE`、`BLOCK_SIZE`、`BLOCKSIZE` 环境变量指定。

### 示例

```bash
du -sh *                     # 当前目录下每个项的总占用（最常用）
du -sh .                     # 当前目录总计
du -h --max-depth=1          # 只到第一层
du -h -d 1 /var              # 同上
du -ah | sort -rh | head -20 # 找出最大的 20 个文件/目录
du -sh */ | sort -rh         # 按目录大小排序
du -sh --exclude='*.log' *   # 排除日志
du -sh --exclude-from=.gitignore .
du -x -sh /                  # 不跨文件系统
du -sh --apparent-size *     # 表观大小
du -h --time -d 1            # 带最近修改时间
du -shc *                    # 带总计
du -s -B1 dir                # 以字节为单位（脚本用）
du -0 -a dir | xargs -0 ...  # 处理含空格/换行的文件名
```

### 易踩坑

- **`du` 默认是递归的**，在 `/` 上直接跑会扫全盘且极慢 —— 一定加 `-s` 或 `-d 1`。
- `du` 显示的是**磁盘占用（块）**，`ls -l` 显示的是**表观大小**，稀疏文件下差异巨大。
- **硬链接默认只算一次**（先遇到的那个），想重复计数用 `-l`。
- `du` 与 `df` 不一致是常见的：`du` 看不到**已被删除但仍被进程打开**的文件，也看不到文件系统元数据与保留块。
- 目录有读权限但无执行权限时，`du` 无法进入。
- 默认**不跟随符号链接**（`-P`），要跟随用 `-L`。

**BSD/macOS**：macOS 的 `du` 无 `--exclude`、`--exclude-from`、`-t/--threshold`、`--time-style`；`-d` 支持较晚。

---

## 附录 A：信号编号速查（Linux，经内核源码核实）

```
 1 SIGHUP   2 SIGINT   3 SIGQUIT  4 SIGILL   5 SIGTRAP  6 SIGABRT  7 SIGBUS   8 SIGFPE
 9 SIGKILL 10 SIGUSR1 11 SIGSEGV 12 SIGUSR2 13 SIGPIPE 14 SIGALRM 15 SIGTERM 16 SIGSTKFLT
17 SIGCHLD 18 SIGCONT 19 SIGSTOP 20 SIGTSTP 21 SIGTTIN 22 SIGTTOU 23 SIGURG  24 SIGXCPU
25 SIGXFSZ 26 SIGVTALRM 27 SIGPROF 28 SIGWINCH 29 SIGIO 30 SIGPWR 31 SIGSYS
32..64 实时信号（glibc 下 SIGRTMIN 通常为 34）
```

> ⚠️ **这些编号是 Linux 特有的**。macOS/BSD 不同（SIGUSR1=30、SIGUSR2=31、SIGSTOP=17、SIGTSTP=18、SIGCHLD=20）。跨平台脚本请用**信号名**而非编号。

## 附录 B：排查套路速查

| 症状 | 命令 |
|---|---|
| 谁在吃 CPU | `top -o %CPU` / `ps aux --sort=-%cpu \| head` |
| 谁在吃内存 | `top -o %MEM` / `ps aux --sort=-%mem \| head` |
| 哪个进程占了端口 | `ss -lntp`（或 `lsof -i:8080`） |
| 某服务为什么不起来 | `systemctl status X` → `journalctl -u X -n 100` |
| 磁盘满了但找不到大文件 | `du -ah / \| sort -rh \| head -20` |
| 磁盘有空间但写不进去 | `df -i`（inode 耗尽） |
| 删了文件空间没释放 | `lsof +L1 \| grep deleted` |
| 负载高但 CPU 空闲 | `ps -eo stat,pid,args \| grep '^D'`（I/O 等待） |
| 找僵尸进程 | `ps -eo stat,pid,ppid,args \| awk '$1 ~ /Z/'` |
| 某命令杀了不退出 | `kill -9`；若仍不退，查 `ps -o stat` 是否为 `D` |
| 谁把机器关机了 | `journalctl -b -1 -n 50` / `last -x shutdown` |
| 内存到底够不够 | `free -h` 看 **available** 列 |
| CPU 核数与拓扑 | `lscpu \| grep -E 'Socket\|Core\|Thread'` |
| SSD 还是 HDD | `lsblk -d -o NAME,ROTA,SIZE`（`ROTA=0` 是 SSD） |
| 分区与挂载 | `lsblk -f` |

## 附录 C：危险操作清单

| 操作 | 风险 |
|---|---|
| `kill -9 PID` | 进程无清理机会，丢数据、留脏锁 |
| `kill -9 -1` | 杀光当前用户几乎所有进程（含 shell） |
| `killall` / `pkill -f` | 模式过宽会**误杀无关进程**，`-f` 还可能匹配到自己 |
| `systemctl stop/disable` | 线上服务中断 / 取消自启 |
| `systemctl halt/poweroff/reboot` | **立即关机 / 重启** |
| `journalctl --vacuum-*` | **永久删除**归档日志 |
| `du` 在 `/` 无 `-s` | 扫全盘，长时间高 I/O（非破坏但影响生产） |
| `rm` 配合 `du`/`df` 结果 | 见 batch-1 参考，勿在个人目录批量删 |

**建议**：杀进程先 `kill -15` 给机会；杀之前先 `pgrep -af` 预览；操作服务前确认 `systemctl is-active` 与依赖关系。
