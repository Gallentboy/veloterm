# Linux 文件与目录命令参考

> 覆盖 `ls / cd / pwd / cp / mv / rm / mkdir / touch / find / locate / tree / stat / file / ln` 共 14 个命令。
>
> **来源**：GNU coreutils 9.11、GNU findutils、file 5.46、plocate、tree 2.1/2.2 官方手册（man7.org / GNU / Debian / mankier）；`cd`、`pwd` 的 shell 内建行为取自 GNU Bash 手册（bash 5.x）。**BSD/macOS 差异在每节末尾单列**，因为本机是 macOS、而实际运维目标通常是 Linux。
>
> 不确定的地方一律查了手册，未凭记忆推断。

## 速查表

| 命令 | 作用 | 危险级别 | 本参考中的参数模板 |
|---|---|---|---|
| `ls` | 列出目录内容 | safe | `ls {{options}} {{path}}` |
| `cd` | 切换目录（shell 内建） | safe | `cd {{path}}` |
| `pwd` | 显示当前路径 | safe | `pwd` |
| `cp` | 复制文件/目录 | confirm | `cp {{options}} {{source}} {{dest}}` |
| `mv` | 移动/重命名 | confirm | `mv {{options}} {{source}} {{dest}}` |
| `rm` | 删除文件/目录 | **destructive** | `rm {{options}} {{path}}` |
| `mkdir` | 创建目录 | safe | `mkdir {{options}} {{path}}` |
| `touch` | 创建空文件/改时间戳 | safe | `touch {{options}} {{file}}` |
| `find` | 递归查找文件 | confirm | `find {{path}} {{expression}}` |
| `locate` | 基于索引快速查找 | safe | `locate {{options}} {{pattern}}` |
| `tree` | 树状显示目录 | safe | `tree {{options}} {{path}}` |
| `stat` | 查看文件元信息 | safe | `stat {{options}} {{file}}` |
| `file` | 判断文件类型 | safe | `file {{options}} {{file}}` |
| `ln` | 创建链接 | confirm | `ln {{options}} {{target}} {{link_name}}` |

---

## 1. ls — 列出目录内容

**语法**：`ls [OPTION]... [FILE]...`

不指定路径时列出当前目录；默认按名称排序，除非使用了 `-c -f -t -u -v -S -U -X` 或 `--sort`。

### `-l` 长格式的 7 列含义

| 列 | 含义 |
|---|---|
| 1 | 权限（文件类型 + rwx 三段） |
| 2 | 硬链接数 |
| 3 | 属主 |
| 4 | 属组 |
| 5 | 大小 |
| 6 | 时间 |
| 7 | 名称 |

### 筛选

| 选项 | 含义 |
|---|---|
| `-a, --all` | 不忽略 `.` 开头的项（含隐含的 `.` 与 `..`） |
| `-A, --almost-all` | 不忽略隐藏项，但不列出 `.` 和 `..` |
| `-B, --ignore-backups` | 不列出 `~` 结尾的备份文件 |
| `--hide=PATTERN` | 不列出匹配 PATTERN 的项（被 `-a`/`-A` 覆盖） |
| `-I, --ignore=PATTERN` | 不列出匹配 PATTERN 的项 |
| `-d, --directory` | 只列出目录本身，不展开其内容 |
| `-R, --recursive` | 递归列出子目录 |
| `-f` | 等价 `-a -U`（不排序、显示全部） |

### 格式

| 选项 | 含义 |
|---|---|
| `-l` | 长格式 |
| `-1` | 每行一个 |
| `-C` | 按列排列 |
| `-x` | 按行排列 |
| `-m` | 逗号分隔填充整行 |
| `-g` | 同 `-l`，但不显示属主 |
| `-o` | 同 `-l`，但不显示属组 |
| `-G, --no-group` | 长格式下不打印属组名 |
| `-n, --numeric-uid-gid` | 同 `-l`，但用数字 UID/GID |
| `--full-time` | 等价 `-l --time-style=full-iso` |
| `-h, --human-readable` | 大小用 1K/234M/2G 形式 |
| `--si` | 同上，但按 1000 进制 |
| `-s, --size` | 打印每个文件占用的块数 |
| `-k, --kibibytes` | 以 1024 字节块显示（仅用于 `-s` 与目录总计） |
| `-i, --inode` | 打印 inode 号 |
| `-Z, --context` | 打印 SELinux 安全上下文 |
| `--author` | 配合 `-l` 打印作者 |
| `-D, --dired` | 生成 Emacs dired 模式输出 |
| `--zero` | 行尾用 NUL 而非换行 |

### 排序

| 选项 | 含义 |
|---|---|
| `-t` | 按修改时间，最新在前 |
| `-S` | 按文件大小，最大在前 |
| `-X` | 按扩展名字母序 |
| `-v` | 文本中的版本号自然排序 |
| `-U` | 不排序（目录原序） |
| `-r, --reverse` | 反向排序 |
| `-c` | 配合 `-lt` 按/显示 ctime；配合 `-l` 显示 ctime 并按名称排序；否则按 ctime 倒序 |
| `-u` | 同上，但针对 atime |
| `--sort=WORD` | `none`(-U) / `size`(-S) / `time`(-t) / `version`(-v) / `extension`(-X) / `name` / `width` |
| `--time=WORD` | `atime,access,use`(-u) / `ctime,status`(-c) / `mtime,modification`(默认) / `birth,creation` |
| `--time-style=STYLE` | `full-iso` / `long-iso` / `iso` / `locale` / `+FORMAT`；加 `posix-` 前缀仅在非 POSIX locale 生效 |

### 链接与类型标记

| 选项 | 含义 |
|---|---|
| `-L, --dereference` | 显示符号链接指向目标的信息 |
| `-H, --dereference-command-line` | 跟随命令行上列出的符号链接 |
| `--dereference-command-line-symlink-to-dir` | 只跟随命令行上指向目录的链接 |
| `-p, --indicator-style=slash` | 目录名后追加 `/` |
| `-F, --classify[=WHEN]` | 追加类型标记：`*` 可执行 `/` 目录 `=` 套接字 `>` 门 `@` 符号链接 `\|` FIFO |
| `--file-type` | 同 `-F`，但不追加 `*` |
| `--indicator-style=WORD` | `none`(默认) / `slash`(-p) / `file-type` / `classify`(-F) |

### 显示控制

`--color[=WHEN]`（`always` 默认 / `auto` / `never`，配合 `LS_COLORS`、`dircolors`）、`--group-directories-first`、`-q`（不可打印字符显示为 `?`）、`--show-control-chars`、`-Q`（名称加双引号）、`-N`（不加引号）、`-b`（C 风格转义）、`--quoting-style=WORD`（`literal/locale/shell/shell-always/shell-escape/shell-escape-always/c/escape`）、`-w COLS`（输出宽度，0=不限）、`-T COLS`（制表位，默认 8）、`--block-size=SIZE`（`K/M/G/T/P/E/Z/Y/R/Q` 为 1024 进制，`KB/MB…` 为 1000 进制，也可用 `KiB/MiB`）。

### 退出码

`0` 正常；`1` 小问题（如子目录不可读）；`2` 严重错误（如命令行参数无效）。

### 示例

```bash
ls -lah /var/log            # 长格式 + 人类可读大小 + 隐藏文件
ls -lt | head -n 10         # 最近修改的 10 个文件
ls -lS --block-size=M       # 按大小排序，以 MB 显示
ls -d */                    # 只列目录
ls -1F --color=never        # 每行一个 + 类型标记 + 无色（便于管道）
```

**坑**：`ls -l` 显示的是**链接本身**的信息，要显示目标信息需 `-L`；`ls` 的排序受 locale 影响，脚本里比较输出建议 `LC_ALL=C ls`。

**BSD/macOS 差异**：macOS 的 `ls` 没有 `--time-style` / `--sort` / `--time`。彩色用 `-G`，完整时间用 `-T`，扩展属性用 `-@`，文件标志用 `-O`，ACL 用 `-e`，`-B` 表示强制显示不可打印字符。选项集为 `[-@ABCFGHILOPRSTUWabcdefghiklmnopqrstuvwxy1%,]`。

---

## 2. cd — 切换目录（shell 内建）

**语法**（bash 5.x）：`cd [-L] [-@] [directory]` / `cd -P [-e] [-@] [directory]`

`cd` **不是独立程序**，而是 bash/zsh 的内建命令，因此不能 `sudo cd`、也不能在非 shell 上下文中调用。

| 选项 | 含义 |
|---|---|
| `-L` | 跟随符号链接（默认行为） |
| `-P` | 使用物理目录结构，不跟随符号链接 |
| `-e` | 仅在配合 `-P` 时有效：切换成功但无法确定当前目录时返回非零状态 |
| `-@` | 在支持的系统上，把文件的扩展属性作为目录呈现 |

### 特殊用法

| 写法 | 行为 |
|---|---|
| `cd` | 回到 `$HOME` |
| `cd -` | 回到 `$OLDPWD`，并打印新路径 |
| `cd ..` | 上一级 |
| `cd ~user` | 进入指定用户家目录 |

### CDPATH

若设置了 `CDPATH` 且目标不以 `/` 开头，`cd` 会按冒号分隔的顺序在各目录中查找目标；`CDPATH` 中的空目录名等价于当前目录。目标以 `/` 开头时忽略 `CDPATH`。命中非空目录名时会打印新路径。

### 环境变量

`HOME` 默认目标；`PWD` 当前目录（由 shell 维护）；`OLDPWD` 上一次目录；`CDPATH` 搜索路径。若目录未找到且 shell 选项 `cdable_vars` 已开启，则把该词当作变量名解析。

### 关键差别：`-P` 与 `-L`

`-P` 在处理路径中的 `..` **之前**解析符号链接，`-L` 在**之后**解析。因此 `cd -L link/..` 与 `cd -P link/..` 可能落在不同目录。要消除歧义可用 `cd -P "$(pwd)"`。

### 示例

```bash
cd /var/log
cd -                        # 回到上一个目录
cd -P /var/www/current      # 解析掉符号链接，进入真实物理目录
```

**退出码**：`0` 成功；非零表示目录不存在 / 无权限 / 不是目录。

---

## 3. pwd — 显示当前路径

**语法**：`pwd [OPTION]...`（coreutils 版）；bash 内建为 `pwd [-LP]`

| 选项 | 含义 |
|---|---|
| `-L, --logical` | 使用环境变量 `$PWD`，即使其中含符号链接 |
| `-P, --physical` | 解析所有符号链接，输出真实物理路径 |

未指定任何选项时，`/bin/pwd` 默认按 `-P` 处理。

**环境变量**：`PWD` 由 shell 在每次 `cd` 时维护。内建 `pwd -L` 直接采用它；若 `PWD` 不可信（被篡改、或目录被重命名/删除），应改用 `pwd -P` 或 `/bin/pwd`。

**注意**：shell 内建 `pwd` 的默认行为受 `set -P` / `set -L` 影响，可能与 `/bin/pwd` 不一致。要拿物理路径请显式 `pwd -P`。

**退出码**：`0` 成功；非零表示当前目录已不存在或无法读取。

---

## 4. cp — 复制文件/目录

**语法**：

```
cp [OPTION]... [-T] SOURCE DEST
cp [OPTION]... SOURCE... DIRECTORY
cp [OPTION]... -t DIRECTORY SOURCE...
```

### 核心选项

| 选项 | 含义 |
|---|---|
| `-a, --archive` | 等价 `-dR --preserve=all`（归档备份首选） |
| `-r, -R, --recursive` | 递归复制目录 |
| `-p` | 等价 `--preserve=mode,ownership,timestamps` |
| `-d` | 等价 `--no-dereference --preserve=links` |
| `-l, --link` | 建立硬链接而非复制 |
| `-s, --symbolic-link` | 建立符号链接而非复制 |
| `-v, --verbose` | 显示过程 |
| `--debug` | 解释复制方式（隐含 `-v`） |

### 覆盖策略

| 选项 | 含义 |
|---|---|
| `-i, --interactive` | 覆盖前询问（会覆盖之前的 `-n`） |
| `-n, --no-clobber` | 静默跳过已存在文件（已废弃，建议用 `--update=none`） |
| `-f, --force` | 目标无法打开时先删除再试（与 `-n` 同用时被忽略） |
| `--remove-destination` | 先删除目标再打开（与 `--force` 的差别） |
| `-u` | 等价 `--update=older` |
| `--update[=UPDATE]` | `all`（默认） / `none` / `none-fail` / `older`（指定 `--update` 时的默认） |

### 属性保留

| 选项 | 含义 |
|---|---|
| `--preserve[=ATTR_LIST]` | `ATTR_LIST` 逗号分隔：`mode`（含 ACL/xattr 权限）`ownership` `timestamps` `links` `context` `xattr` `all` |
| `--no-preserve=ATTR_LIST` | 反向，不保留指定属性 |
| `-Z` | 设目标 SELinux 默认上下文 |
| `--context[=CTX]` | 指定 SELinux/SMACK 上下文 |
| `--attributes-only` | 只复制属性，不复制数据 |

### 链接处理

`-H`（跟随命令行上的符号链接）、`-L, --dereference`（总是跟随源中的符号链接）、`-P, --no-dereference`（从不跟随，**默认**）、`--keep-directory-symlink`（跟随已存在的目录符号链接）。

### 其他

| 选项 | 含义 |
|---|---|
| `--reflink[=WHEN]` | `auto`（默认，尽量 CoW，否则回退） / `always`（不支持则失败） / `never` |
| `--sparse=WHEN` | `auto`（默认，启发式） / `always` / `never`，控制稀疏文件 |
| `--backup[=CONTROL]` | 备份已存在目标；`CONTROL`：`none,off` / `numbered,t` / `existing,nil` / `simple,never` |
| `-b` | 同 `--backup`，但不接受参数 |
| `-S, --suffix=SUFFIX` | 自定义备份后缀（默认 `~`） |
| `-t, --target-directory=DIR` | 把所有源复制进 `DIR` |
| `-T, --no-target-directory` | 把目标当普通文件 |
| `--parents` | 在 `DIR` 下保留源的完整路径 |
| `--strip-trailing-slashes` | 去掉源参数末尾的 `/` |
| `-x, --one-file-system` | 不跨文件系统 |
| `--copy-contents` | 递归时复制特殊文件的内容 |

### 尾斜杠语义

源带尾 `/` 表示“目录内容”，不带则复制目录本身；目标带尾 `/` 必须是已存在目录。配合 `--strip-trailing-slashes` 可消除歧义。

**退出码**：`0` 全部成功；非零表示有文件复制失败。

### 示例

```bash
cp -a /etc/nginx /backup/nginx-$(date +%F)   # 归档备份
cp -rv src/ dst/                             # 递归复制并显示过程
cp -u --backup=numbered a.conf /etc/a.conf   # 仅在较新时覆盖，并做编号备份
```

**坑**：`cp -r src dst` 与 `cp -r src/ dst` 在目标不存在时行为不同——前者会创建 `dst` 并放入 `src`，后者只复制内容。生产环境建议 `-a` 而非 `-r`。

**BSD/macOS 差异**：macOS 的 `cp` 没有 `--reflink` / `--sparse` / `--preserve=ATTR_LIST` / `--parents`。它用 `-c`（clonefile 快速克隆）、`-X`（不复制扩展属性）、`-N`（不复制 ACL）、`-S`（稀疏文件）。

---

## 5. mv — 移动/重命名

**语法**：

```
mv [OPTION]... [-T] SOURCE DEST
mv [OPTION]... SOURCE... DIRECTORY
mv [OPTION]... -t DIRECTORY SOURCE...
```

同一文件系统内是 `rename(2)` 原子操作；跨文件系统时退化为“复制 + 删除”。

| 选项 | 含义 |
|---|---|
| `-f, --force` | 覆盖前不询问 |
| `-i, --interactive` | 覆盖前询问 |
| `-n, --no-clobber` | 不覆盖已存在文件 |
| `--backup[=CONTROL]` | 备份已存在目标（取值同 `cp`） |
| `-b` | 同 `--backup`，但不接受参数 |
| `-S, --suffix=SUFFIX` | 自定义备份后缀（默认 `~`） |
| `-u` | 等价 `--update=older` |
| `--update[=UPDATE]` | `all`（默认） / `none` / `none-fail` / `older` |
| `-v, --verbose` | 显示过程 |
| `--debug` | 解释复制方式（隐含 `-v`） |
| `--exchange` | 交换源与目标（较新 GNU 版本） |
| `--no-copy` | 重命名失败时不退化为复制 |
| `-t, --target-directory=DIR` | 把源全部移入 `DIR` |
| `-T, --no-target-directory` | 把目标当普通文件 |
| `--strip-trailing-slashes` | 去掉源末尾的 `/` |
| `-Z, --context` | 设目标 SELinux 默认上下文 |

`-i`、`-f`、`-n` 同时出现时**只有最后一个生效**。

**坑**：`mv` 默认会静默覆盖同名目标（不像 `rm` 有 `--preserve-root` 保护）。生产环境建议用 `mv -i` 或 `mv -n`。

**退出码**：`0` 成功；非零表示有文件移动失败。

**BSD/macOS 差异**：macOS 的 `mv` 只有 `[-f | -i | -n] [-hv]`，没有 `--backup` / `--update` / `-t` / `-T`。

---

## 6. rm — 删除文件/目录

**语法**：`rm [OPTION]... [FILE]...`

默认不删除目录，需 `-r`。

| 选项 | 含义 |
|---|---|
| `-f, --force` | 忽略不存在的文件与参数，永不提示 |
| `-i` | 删除每一个文件前都询问 |
| `-I` | 删除超过 3 个文件或递归删除前**只询问一次**（比 `-i` 温和，仍能挡住大部分误操作） |
| `--interactive[=WHEN]` | `never` / `once`（同 `-I`） / `always`（同 `-i`）；不带 WHEN 时等价 `always` |
| `-r, -R, --recursive` | 递归删除目录及其内容 |
| `-d, --dir` | 删除空目录 |
| `-v, --verbose` | 显示过程 |
| `--one-file-system` | 递归时跳过其他文件系统上的目录 |
| `--preserve-root[=all]` | 拒绝删除 `/`（**默认开启**）；加 `all` 还会拒绝删除与父目录不同设备的参数 |
| `--no-preserve-root` | 关闭对 `/` 的保护 |

### 行为细节

- `-I` 或 `--interactive=once` 时，若文件多于 3 个或使用 `-r`/`-R`，会先整体确认一次，回答否定则**整个命令中止**。
- 否则，当文件不可写、标准输入是终端、且未给 `-f` 时，会逐个询问。
- 删除最后一级名称为 `.` 或 `..` 的参数一律被拒绝。

### 以 `-` 开头的文件名

用 `rm -- -foo` 或 `rm ./-foo`，否则会被当作选项。

**重要提示**：`rm` 删除后数据仍可能被恢复；需要不可恢复时用 `shred(1)`。执行前务必先 `ls` 确认目标，尤其是变量参与路径拼接时。

**退出码**：`0` 全部删除成功；非零表示有文件未能删除。

**BSD/macOS 差异**：macOS 的 `rm` 没有 `--preserve-root` / `--one-file-system` / `-I`。它有 `-W`（撤销删除白化文件）、`-x`（不跨设备）、`-P`（覆盖写后再删）、`-v`、`-R`、`-d`。

---

## 7. mkdir — 创建目录

**语法**：`mkdir [OPTION]... DIRECTORY...`

| 选项 | 含义 |
|---|---|
| `-p, --parents` | 目录已存在也不报错，并按需创建所有父目录（**父目录不受 `-m` 影响**） |
| `-m, --mode=MODE` | 设置权限，写法同 `chmod`（如 `755`、`u=rwx,g=rx,o=`），不是 `a=rwx - umask` |
| `-v, --verbose` | 每创建一个目录打印一条消息 |
| `-Z` | 设为 SELinux 默认上下文 |
| `--context[=CTX]` | 同 `-Z`，指定 CTX 时设为该 SELinux/SMACK 上下文 |

**`-p` 的行为**：配合 `-p` 时，若目标目录已存在不会报错，退出码仍为 `0`；但若中间路径被已存在的**文件**挡住，仍会报错。

**默认权限**：未指定 `-m` 时按 `0777 & ~umask` 计算（常见 `umask 022` → `755`）。

**退出码**：`0` 成功；非零表示有目录创建失败。

**BSD/macOS 差异**：macOS 的 `mkdir` 只有 `[-pv] [-m mode]`，没有 `-Z` / `--context`。

---

## 8. touch — 创建空文件/改时间戳

**语法**：`touch [OPTION]... FILE...`

把文件的访问时间与修改时间更新为当前时间；文件不存在则创建空文件（除非给了 `-c` 或 `-h`）。参数为 `-` 时特殊处理，改为修改标准输出关联文件的时间。

| 选项 | 含义 |
|---|---|
| `-a` | 只改访问时间（atime） |
| `-m` | 只改修改时间（mtime） |
| `-c, --no-create` | 不创建任何文件 |
| `-d, --date=STRING` | 用可读日期字符串代替当前时间（如 `"2004-02-29 16:21:42"`、`"next Thursday"`；空字符串表示当天 00:00） |
| `-t [[CC]YY]MMDDhhmm[.ss]` | 用指定时间，格式与 `-d` 不同 |
| `-r, --reference=FILE` | 使用该文件的时间戳 |
| `-h, --no-dereference` | 作用于符号链接本身而非其指向的文件（仅部分系统支持） |
| `-f` | 被忽略（为兼容保留） |
| `--time=WORD` | 指定要改哪个时间：`access/atime/use` 或 `modify/mtime` |

**常见用途**：创建占位文件；把时间戳对齐到另一文件（`-r`）；伪造/回拨时间做测试；配合 `make` 强制重编译。

**退出码**：`0` 成功；非零表示有时间戳更新失败。

**BSD/macOS 差异**：macOS 的 `touch` 用 `-A [-][[hh]mm]SS` 微调访问时间，`-d` 只接受 ISO 形式 `YYYY-MM-DDThh:mm:SS[.frac][tz]`，没有 `--time` / `-c` 的长选项形式（`-c` 存在但语义是“不创建”）。

---

## 9. find — 递归查找文件

**语法**：`find [-H] [-L] [-P] [-D debugopts] [-Olevel] [starting-point...] [expression]`

表达式由「选项 + 测试 + 动作」组成，从左到右求值，支持 `-a` / `-o` / `!` / `()` 运算符。

### 全局选项

| 选项 | 含义 |
|---|---|
| `-maxdepth N` | 最多下探 N 层 |
| `-mindepth N` | 至少 N 层才开始处理 |
| `-depth` | 先处理目录内容，再处理目录本身 |
| `-d` | 同 `-depth`（BSD 兼容） |
| `-follow` | 等价 `-L`（已废弃） |
| `-xdev` / `-mount` | 不跨文件系统 |
| `-ignore_readdir_race` / `-noignore_readdir_race` | 忽略/不忽略 stat 竞态错误 |
| `-noleaf` | 关闭“目录硬链接数减 2”的优化（非 Unix 语义文件系统需要） |
| `-regextype TYPE` | 改变 `-regex`/`-iregex` 的正则语法 |
| `-daystart` | 以今天 00:00 为基准计算时间（影响 `-amin/-atime/-cmin/-ctime/-mmin/-mtime`） |
| `-files0-from FILE` | 从文件读取起始路径 |
| `-D debugopts` | 调试选项 |
| `-warn` / `-nowarn` | 开关命令行警告 |
| `-help` / `-version` | 帮助 / 版本 |

**符号链接**：`-P` 不跟随（默认）、`-L` 跟随、`-H` 只跟随命令行上的链接。

### 测试

| 选项 | 含义 |
|---|---|
| `-name PATTERN` | 按文件名匹配 |
| `-iname` | 同 `-name`，忽略大小写 |
| `-path PATTERN` | 按整条路径匹配 |
| `-ipath` / `-wholename` / `-iwholename` | 路径匹配的变体 |
| `-lname` / `-ilname` | 符号链接内容匹配 |
| `-regex` / `-iregex` | 正则匹配整条路径 |
| `-type c` | 类型：`b` 块设备 `c` 字符设备 `d` 目录 `p` FIFO `f` 普通文件 `l` 符号链接 `s` 套接字 `D` 门（少数系统） |
| `-xtype c` | 同 `-type`，但针对符号链接的目标 |
| `-size n[cwbkMG]` | 大小，**向上取整**；`b`=512 字节块（默认） `c`=字节 `w`=2 字节 `k`=KiB `M`=MiB `G`=GiB |
| `-mtime` / `-atime` / `-ctime n` | 按 24 小时计 |
| `-mmin` / `-amin` / `-cmin n` | 按分钟计 |
| `-newer REF` | 比参考文件新 |
| `-newerXY REF` | `X`/`Y` ∈ `a,c,m,t,B`；X 为被比较对象的时间，Y 为参考时间 |
| `-anewer` / `-cnewer` | `-newer` 的 atime/ctime 变体 |
| `-used n` | 状态变更后 n 天被访问过 |
| `-user NAME` / `-uid N` | 属主 |
| `-group NAME` / `-gid N` | 属组 |
| `-nouser` / `-nogroup` | 无对应用户/组 |
| `-perm MODE` | 权限**精确匹配** |
| `-perm -MODE` | 必须**包含**这些位 |
| `-perm /MODE` | **任一**位被置即可（`-perm +MODE` 自 2005 年起已废弃） |
| `-empty` | 空文件或空目录 |
| `-readable` / `-writable` / `-executable` | 当前用户可读/可写/可执行（考虑 ACL） |
| `-links N` | 硬链接数 |
| `-inum N` | inode 号 |
| `-samefile NAME` | 同一 inode |
| `-fstype TYPE` | 文件系统类型 |
| `-false` / `-true` | 恒假 / 恒真 |
| `-context PATTERN` | SELinux 上下文 |

**数值参数的 `+n` / `-n` / `n`**：`+n` 大于 n；`-n` 小于 n；`n` 恰好等于。

### 动作

| 选项 | 含义 |
|---|---|
| `-print` | 打印并换行 |
| `-print0` | 以 NUL 结尾（配合 `xargs -0`） |
| `-printf FORMAT` | 按格式打印 |
| `-ls` | 以 `ls -dils` 格式列出 |
| `-delete` | 删除匹配项（隐含 `-depth`） |
| `-exec CMD ;` | 对每个文件执行一次 |
| `-exec CMD {} +` | 批量追加文件名执行一次 |
| `-execdir` | 在文件所在目录执行 |
| `-ok` / `-okdir` | 执行前询问 |
| `-fprint` / `-fprint0` / `-fprintf` / `-fls` | 输出到文件 |
| `-prune` | 不进入该目录 |
| `-quit` | 立即退出 |

### 运算符优先级（从高到低）

`!` / `-not` ＞ `-a` / `-and`（隐式 AND） ＞ `-o` / `-or` ＞ `,`（列表，两侧都求值，取右侧的值）

**危险动作**：`-delete`、`-exec rm`、`-execdir` 会真正改动系统。**执行前务必先用 `-print` 空跑确认**。

**退出码**：`0` 全部成功；非零表示发生错误。

### 示例

```bash
find / -name "*.log" -type f
find /var/log -type f -size +100M -printf "%s %p\n"
find . -mtime +30 -name "*.tmp" -print        # 先空跑
find . -name .git -prune -o -type f -print    # 跳过 .git
find /data -type f -exec grep -l ERROR {} +
```

**坑**：`-size` 是向上取整且默认单位是 512 字节块，`-size 1` 不是 1 字节；`-name` 只匹配文件名（不含目录），匹配路径要用 `-path`；`-mtime 0` 表示“不到 24 小时”。

**BSD/macOS 差异**：macOS 的 `find` 用 `-E` 启用扩展正则、`-X` 按扩展属性过滤、`-s` 不跟随符号链接、`-x` 不跨设备、`-d` 深度优先；`-perm` 用 `+mode` 表示“任一位”。`-printf` / `-maxdepth` 等 GNU 扩展**不可用**，需 `brew install findutils` 后使用 `gfind`。

---

## 10. locate — 基于索引快速查找

**语法**：`locate [OPTION]... PATTERN...`（plocate）

在预建的路径索引数据库中快速查找文件，速度远快于 `find`，但结果可能滞后于磁盘现状。

**前置条件**：必须先用 `updatedb(8)` 建立/更新索引（现代发行版由 systemd 的 `updatedb.timer` 定期执行）。索引默认在 `/var/lib/mlocate/mlocate.db` 或 `/var/lib/plocate/plocate.db`。**刚创建的文件在 `updatedb` 跑之前查不到。**

### 匹配规则

- 不含通配符的模式按**子串**匹配（等价 `*模式*`）；
- 含 `*` `?` `[]` 之一时按 **glob** 匹配（此时需自己写前后 `*` 才是子串）；
- `--regexp` 用 POSIX 基本正则，`--regex` 用扩展正则（两者都会退化为全库线性扫描，较慢）。

### 选项

| 选项 | 含义 |
|---|---|
| `-b, --basename` | 只匹配文件名部分（目录名不参与匹配，但仍会打印） |
| `-w, --wholename` | 匹配整条路径（**默认**） |
| `-i, --ignore-case` | 忽略大小写 |
| `-c, --count` | 只输出匹配总数 |
| `-l, --limit N` | 最多输出 N 条 |
| `-0, --null` | 以 NUL 代替换行（配合 `xargs -0`） |
| `-e, --existing` | 只输出当前仍存在的文件 |
| `-d, --database DBPATH` | 指定数据库（可多次，或用 `:` 分隔多个） |
| `-N, --literal` | 不加引号输出（默认会做 shell 转义） |
| `-A, --all` | 仅用于兼容 mlocate（plocate 忽略它） |
| `-r, --regexp` / `--regex` | 基本 / 扩展正则 |
| `-q, --quiet` | 不输出错误 |
| `-S, --statistics` | 显示数据库统计 |
| `-V, --version` / `--help` | 版本 / 帮助 |

**mlocate 与 plocate 的差别**：给多个模式时，mlocate 匹配“任一”模式，plocate 要求“**全部**”匹配。

**环境变量**：`LOCATE_PATH` 追加到数据库列表之后。

**macOS 注意**：macOS 默认**没有** `locate`。要么用 Spotlight 的 `mdfind`（如 `mdfind -name nginx.conf`），要么手动启用 `/System/Library/LaunchDaemons/com.apple.locate.plist`。

**退出码**：`0` 找到匹配（或使用 `--help`/`--version`）；`1` 出错或没有匹配。

---

## 11. tree — 树状显示目录

**语法**：

```
tree [-acdfghilnpqrstuvxACDFJQNSUX] [-L level [-R]] [-H baseHREF] [-T title]
     [-o filename] [-P pattern] [-I pattern] [--gitignore] [--gitfile[=]file]
     [--matchdirs] [--metafirst] [--ignore-case] [--nolinks] [--hintro[=]file]
     [--houtro[=]file] [--inodes] [--device] [--sort[=]name] [--dirsfirst]
     [--filesfirst] [--filelimit #] [--si] [--du] [--prune] [--charset[=]X]
     [--timefmt[=]format] [--fromfile] [--fflinks] [--info] [--infofile[=]file]
     [--noreport] [--version] [--help] [--] [directory ...]
```

以缩进的树状结构递归列出目录内容，结束时打印文件/目录总数。无参数时列出当前目录。

### 列表选项

| 选项 | 含义 |
|---|---|
| `-a` | 显示全部（默认不显示 `.` 开头的隐藏项；**无论如何都不显示 `.` 与 `..`**） |
| `-d` | 只列目录 |
| `-l` | 跟随指向目录的符号链接（会检测并避免递归） |
| `-f` | 显示每个文件的完整路径前缀 |
| `-x` | 只停留在当前文件系统（同 `find -xdev`） |
| `-L LEVEL` | 最大显示深度 |
| `-R` | 按 `-L` 的层数递归向下，各自输出到 `00Tree.html` |
| `-P PATTERN` | 只列匹配通配符的文件（可多次；需配合 `-a` 才能匹配隐藏文件；支持 `*` `?` `[...]` `[^...]` 与 `\|` 分隔多模式；模式末尾 `/` 只匹配目录） |
| `-I PATTERN` | 排除匹配项（可多次） |
| `-i` | 不打印缩进线（常与 `-f` 配合） |

### 文件信息选项

| 选项 | 含义 |
|---|---|
| `-p` | 显示类型与权限（同 `ls -l`） |
| `-u` | 显示属主 |
| `-g` | 显示属组 |
| `-s` | 显示字节大小 |
| `-h` | 人类可读大小（注意：tree 的 `-h` **四舍五入**，与 `ls -h` 向上取整不同） |
| `--si` | 1000 进制 |
| `-D` | 显示最后修改时间 |
| `--inodes` | 显示 inode |
| `--device` | 显示设备号 |
| `-F` | 追加类型标记（目录 `/` 套接字 `=` 可执行 `*` FIFO `\|`） |
| `-q` | 不可打印字符显示为 `?` |
| `-N` | 原样输出不可打印字符 |
| `-Q` | 文件名加双引号 |

### 排序选项

| 选项 | 含义 |
|---|---|
| `-t` | 按修改时间 |
| `-r` | 反向字母序 |
| `-v` | 按版本号 |
| `--sort[=name]` | `name` / `mtime` / `size` / `ctime` / `none` 等 |
| `--dirsfirst` | 目录排前 |
| `--filesfirst` | 文件排前 |
| `--timefmt FORMAT` | 自定义时间格式 |

### 图形/输出选项

`-C`（强制彩色）、`-n`（强制关闭彩色）、`-A`（ANSI 线条）、`-S`（ASCII 线条，等价 `--charset=IBM437`）、`--charset X`、`-H baseHREF`（输出 HTML）、`-T title`（HTML 标题与 H1）、`--nolinks`、`-J`（JSON）、`-X`（XML）、`-o FILE`（输出到文件）、`--noreport`（不打印末尾统计）、`--filelimit #`（目录条目超过 # 则不下探）、`--prune`（省略空目录）、`--du`（目录大小按内容累计）、`--gitignore`（遵循 `.gitignore`）、`--gitfile FILE`、`--matchdirs`（让 `-P` 也作用于目录名）、`--fromfile`（从文件读取路径列表）、`--metafirst`、`--ignore-case`、`--info` / `--infofile`（读取 `.info` 文件描述）。

**安装**：多数发行版默认不带 tree：Debian/Ubuntu 用 `apt install tree`，RHEL 用 `yum install tree`，macOS 用 `brew install tree`。

**退出码**：`0` 成功；非零表示目录不存在等错误。

**已知问题**（官方手册自述）：`-P`/`-I` 不会剪掉因此变空的目录；`-I`/`-P`/`--filelimit` 会导致末尾文件/目录计数不准。

### 示例

```bash
tree -L 2
tree -a -I '.git|node_modules' --dirsfirst
tree -J -L 3 -o tree.json          # 导出 JSON
tree --du -h                       # 目录累计大小
```

---

## 12. stat — 查看文件元信息

**语法**：`stat [OPTION]... FILE...`

显示文件或文件系统的详细状态（比 `ls -l` 更全：设备号、inode、硬链接数、三种时间戳、块信息）。

| 选项 | 含义 |
|---|---|
| `-L, --dereference` | 跟随符号链接 |
| `-f, --file-system` | 显示文件系统状态而非文件状态 |
| `-c, --format=FORMAT` | 自定义输出格式（每次输出后加换行） |
| `--printf=FORMAT` | 同 `-c`，但解释反斜杠转义且**不加**末尾换行（需自己写 `\n`） |
| `-t, --terse` | 精简形式（等价于固定的 FORMAT） |
| `--cached=MODE` | 缓存策略 `always` / `never` / `default`（对远程文件系统有用） |

### 文件格式指令（`-c`）

| 指令 | 含义 | 指令 | 含义 |
|---|---|---|---|
| `%%` | 百分号 | `%n` | 文件名 |
| `%a` | 八进制权限 | `%N` | 带引号的文件名（符号链接显示 `->` 目标） |
| `%A` | 人类可读权限与类型 | `%o` | 最优 I/O 传输大小提示 |
| `%b` | 已分配块数 | `%s` | 总大小（字节） |
| `%B` | `%b` 每块的字节数 | `%t` | 十六进制主设备类型 |
| `%C` | SELinux 上下文 | `%T` | 十六进制次设备类型 |
| `%d` | 十进制设备号 | `%u` | 属主 UID |
| `%D` | 十六进制设备号 | `%U` | 属主名 |
| `%f` | 原始 mode（十六进制） | `%w` | 文件创建时间（可读，未知为 `-`） |
| `%F` | 文件类型 | `%W` | 创建时间（epoch 秒，未知为 0） |
| `%g` | 属组 GID | `%x` | 最后访问时间（可读） |
| `%G` | 属组名 | `%X` | 最后访问（epoch） |
| `%h` | 硬链接数 | `%y` | 最后修改时间（可读） |
| `%i` | inode 号 | `%Y` | 最后修改（epoch） |
| `%m` | 挂载点 | `%z` | 最后状态变更时间（可读） |
| `%Hd` / `%Ld` | 十进制主/次设备号 | `%Z` | 状态变更（epoch） |
| `%r` / `%R` | `st_rdev` 十进制 / 十六进制 | `%Hr` / `%Lr` | 字符/块设备的主/次设备号（十进制） |

### 文件系统格式指令（`-f -c`）

`%a` 非特权用户可用块、`%b` 总数据块、`%c` 总 inode 数、`%d` 空闲 inode、`%f` 空闲块、`%i` 文件系统 ID（十六进制）、`%l` 文件名最大长度、`%n` 文件名、`%s` 块大小、`%S` 基本块大小、`%t` 文件系统类型（十六进制）、`%T` 文件系统类型（可读）。

### `-t` 等价格式

- 文件：`%n %s %b %f %u %g %D %i %h %t %T %X %Y %Z %W %o %C`
- 文件系统：`%n %i %l %t %s %S %b %f %a %c %d`

**退出码**：`0` 成功；非零表示文件不存在等错误。

### 示例

```bash
stat file.txt
stat -c "%A %U:%G %s %n" *.conf
stat -f -c "%T %a/%b blocks free" /var
stat -L /usr/local/bin/app        # 跟随符号链接看真实文件
```

**BSD/macOS 差异**：macOS 的 `stat` **完全另一套接口**：`stat [-FLnq] [-f format | -l | -r | -s | -x] [-t timefmt] [file ...]`，**没有 `-c`**。用 `-f` 加 printf 风格格式符：`%z` 大小、`%N` 名称、`%Su` 属主、`%Sg` 属组、`%Sp` 权限、`%Sm` 修改时间、`%Sc` 状态变更、`%Sa` 访问时间。`-x` 输出 XML，`-r` 输出原始，`-s` 输出 shell 可 `eval` 的赋值。

---

## 13. file — 判断文件类型

**语法**：

```
file [-bcdEhiklLNnprsSvzZ0] [--apple] [--exclude-quiet] [--extension]
     [--mime-encoding] [--mime-type] [-e testname] [-F separator]
     [-f namefile] [-m magicfiles] [-P name=value] file ...
file -C [-m magicfiles]
```

通过三组测试（**文件系统测试 → magic 测试 → 语言测试**）判断文件类型，先成功者即输出。常见输出含 `text` / `executable` / `data` 等关键词。

| 选项 | 含义 |
|---|---|
| `-b, --brief` | 不输出文件名前缀 |
| `-i, --mime` | 输出 MIME 类型（如 `text/plain; charset=us-ascii`） |
| `--mime-type` | 只输出 MIME 类型 |
| `--mime-encoding` | 只输出字符编码 |
| `--extension` | 输出该类型可能的扩展名（斜杠分隔） |
| `-c, --checking-printout` | 打印 magic 文件的解析结果（配合 `-m` 调试新 magic） |
| `-C, --compile` | 把 magic 文件编译成 `magic.mgc` |
| `-d` | 打印内部调试信息到 stderr |
| `-e, --exclude TESTNAME` | 排除某类测试（`ascii` / `troff` / `apptype` 等） |
| `--exclude-quiet` | 同 `--exclude`，但忽略未知测试名 |
| `-E` | 文件系统错误时（如文件不存在）报错并以非零退出，而非按 POSIX 当普通输出继续 |
| `-f, --files-from NAMEFILE` | 从文件读取待检查的文件名（每行一个；`-` 表示标准输入） |
| `-F, --separator SEP` | 设置文件名与结果之间的分隔符（默认 `:`） |
| `-h, --no-dereference` | 不跟随符号链接（未设 `POSIXLY_CORRECT` 时的默认） |
| `-L, --dereference` | 跟随符号链接（设了 `POSIXLY_CORRECT` 时的默认） |
| `-k, --keep-going` | 不因首个匹配而停止，后续匹配前加 `\012- ` |
| `-l, --list` | 按 magic 强度降序列出模式 |
| `-m, --magic-file MAGICFILES` | 指定替代的 magic 文件或目录（冒号分隔） |
| `-n, --no-buffer` | 每检查完一个文件就刷新 stdout |
| `-N, --no-pad` | 不填充文件名对齐 |
| `-p, --preserve-date` | 尽量保留被分析文件的访问时间 |
| `-P, --parameter NAME=VALUE` | 设置资源上限（`bytes` / `elf_notes` / `elf_phnum` / `elf_shnum` / `indir` / `name` / `maxrec` 等） |
| `-r, --raw` | 不把不可打印字符转成 `\ooo` 八进制 |
| `-s, --special-files` | 也读取特殊文件（默认只读普通文件，读特殊文件可能有副作用） |
| `-S, --no-sandbox` | 关闭 seccomp 沙箱（配合 `-z` 解压时才需要） |
| `-v, --version` | 版本 |
| `-z, --uncompress` | 尝试窥视压缩文件内部 |
| `-Z, --uncompress-noreport` | 窥视压缩文件，但只报告内容不报告压缩信息 |
| `-0, --print0` | 文件名后输出 NUL（便于 `cut`） |
| `--apple` | 输出旧 MacOS 的类型与创建者代码 |
| `--help` | 帮助 |

**环境变量**：`MAGIC` 指定默认 magic 文件（设置后不再尝试 `$HOME/.magic`）；`POSIXLY_CORRECT` 控制是否跟随符号链接。

**退出码**：`0` 操作成功；`>0` 遇到错误。文件不存在/无权限/无法判断类型会产生诊断信息，但按 POSIX 要求**不影响退出码**，除非用了 `-E`。

### 示例

```bash
file -b a.bin
file -bi a.bin                     # MIME + 编码
file --mime-type -b *              # 只输出 MIME 类型
file -z backup.tar.gz              # 窥视压缩包内容
file -f list.txt                   # 从文件批量读取
```

**坑**：`file` 的结论基于 magic 数据库，可能误判（尤其对自定义二进制格式）；`file` 与 `file -i` 走的 magic 规则不完全一致，脚本里建议固定用 `-i --mime-type`。

**BSD/macOS 差异**：macOS 自带 `file`（BSD 分支），核心选项与 GNU 基本一致；GNU 独有的 `-P`（参数上限）、`-S`（seccomp 沙箱）在 macOS 上不存在。

---

## 14. ln — 创建链接

**语法**：

```
ln [OPTION]... [-T] TARGET LINK_NAME
ln [OPTION]... TARGET
ln [OPTION]... TARGET... DIRECTORY
ln [OPTION]... -t DIRECTORY TARGET...
```

默认建立**硬链接**，加 `-s` 建立**符号链接**（软链接）。

### 硬链接 vs 符号链接

| | 硬链接 | 符号链接 |
|---|---|---|
| 本质 | 与目标共享 inode | 内容是任意文本路径 |
| 目标必须存在 | 是 | 否（可为悬空链接） |
| 跨文件系统 | 不可以 | 可以 |
| 指向目录 | 不可以（除超级用户） | 可以 |
| 相对路径解析 | — | 相对**其父目录**解析 |

| 选项 | 含义 |
|---|---|
| `-s, --symbolic` | 建立符号链接而非硬链接 |
| `-f, --force` | 先删除已存在的目标文件再建立 |
| `-i, --interactive` | 覆盖前询问 |
| `-n, --no-dereference` | 若链接名是指向目录的符号链接，把它当普通文件处理（在其位置替换，而不是放进目录里） |
| `-r, --relative` | 配合 `-s`，创建相对链接位置解析的链接 |
| `-v, --verbose` | 打印每个已建立链接的名字 |
| `-L, --logical` | 解引用符号链接形式的 TARGET |
| `-P, --physical` | 直接对符号链接建立硬链接（**默认**） |
| `-d, -F, --directory` | 允许超级用户尝试对目录建硬链接（需系统支持） |
| `-T, --no-target-directory` | 始终把链接名当普通文件 |
| `-t, --target-directory=DIR` | 在 `DIR` 中建立链接 |
| `-b` | 同 `--backup`，但不接受参数 |
| `--backup[=CONTROL]` | 备份已存在的链接名（取值同 `cp`） |
| `-S, --suffix=SUFFIX` | 自定义备份后缀（默认 `~`） |

### 选项优先级

默认每个链接名不应已存在；`-f`、`-i`、`-n` 中**最后指定的生效**。使用 `-s` 会忽略 `-L` 与 `-P`；否则当 `TARGET` 是符号链接时，由最后指定的选项决定行为，默认按 `-P`。

**退出码**：`0` 成功；非零表示有链接创建失败。

### 示例

```bash
ln -sfn /opt/app/1.2/app /usr/local/bin/app   # 原子替换软链接（平滑升级）
ln -s ../shared/config ./config               # 相对软链接
ln -r -s /opt/a /opt/b/link                   # 自动计算相对路径
```

**坑**：`ln -sf target link` 在 `link` 已是指向目录的软链接时，会在**目录内**创建文件而不是替换链接本身——务必加 `-n`，即 `ln -sfn`。

**BSD/macOS 差异**：macOS 的 `ln` 用 `-w` 在源不存在时告警，`-F` 配合 `-s` 时替换已有的目录符号链接，`-h` 配合 `-f` 时不跟随符号链接；没有 `--backup` / `-r`（相对链接）。

---

## 附：macOS 与 Linux 的主要差异速查

| 命令 | macOS（BSD）关键差异 |
|---|---|
| `ls` | 无 `--time-style`/`--sort`/`--time`；彩色 `-G`，完整时间 `-T`，xattr `-@` |
| `cp` | 无 `--reflink`/`--sparse`/`--preserve=`；用 `-c` 克隆、`-X` 不带 xattr、`-N` 不带 ACL |
| `mv` | 只有 `-f/-i/-n/-h/-v` |
| `rm` | 无 `--preserve-root`/`-I`；有 `-W`/`-P`/`-x` |
| `mkdir` | 只有 `-p/-v/-m` |
| `touch` | 用 `-A` 微调 atime；`-d` 仅 ISO 格式 |
| `find` | 用 `-E` 扩展正则、`-X` xattr；无 `-printf`/`-maxdepth`（需 `gfind`） |
| `locate` | **默认不存在**；用 `mdfind`（Spotlight）或手动启用 locate.plist |
| `tree` | **默认不装**；`brew install tree` |
| `stat` | **完全不同**：`-f format` 而非 `-c`；格式符为 `%z %N %Su %Sg %Sp %Sm` 等 |
| `file` | 核心选项一致；无 GNU 的 `-P`/`-S` |
| `ln` | 有 `-w`/`-F`/`-h`；无 `--backup`/`-r` |
