# Linux 文本处理与编辑命令参考

> 覆盖 `cat / less / more / head / tail / grep / sed / awk / cut / sort / uniq / wc / diff / vim / nano` 共 15 个命令。
>
> **来源**：GNU coreutils 9.11（cat/head/tail/cut/sort/uniq/wc）、GNU grep 3.x、GNU sed、GNU diffutils 3.10、GNU gawk 5.x、GNU less 6xx、util-linux more、Vim 9.x、GNU nano 8.x 官方手册（man7.org / man.archlinux.org）。
>
> **BSD/macOS 差异在每节末尾单列**，因为本机是 macOS、而实际运维目标通常是 Linux。
>
> 不确定的地方一律查了手册，未凭记忆推断。

## 速查表

| 命令 | 作用 | 危险级别 | 本参考中的参数模板 |
|---|---|---|---|
| `cat` | 查看全文 / 拼接文件 | safe | `cat {{options}} {{file}}` |
| `less` | 分页查看（可前后翻） | safe | `less {{options}} {{file}}` |
| `more` | 分页查看（只能向前） | safe | `more {{options}} {{file}}` |
| `head` | 看头部 | safe | `head {{options}} {{file}}` |
| `tail` | 看尾部 / 跟踪日志 | safe | `tail {{options}} {{file}}` |
| `grep` | 文本搜索 | safe | `grep {{options}} {{pattern}} {{path}}` |
| `sed` | 流编辑 | **confirm** | `sed {{options}} {{script}} {{file}}` |
| `awk` | 列处理 / 报表 | safe | `awk {{options}} {{program}} {{file}}` |
| `cut` | 按列截取 | safe | `cut {{options}} {{file}}` |
| `sort` | 排序 | safe | `sort {{options}} {{file}}` |
| `uniq` | 去重（需先排序） | safe | `uniq {{options}} {{file}}` |
| `wc` | 统计行/词/字节 | safe | `wc {{options}} {{file}}` |
| `diff` | 比较文件 | safe | `diff {{options}} {{file1}} {{file2}}` |
| `vim` | 全功能编辑器 | **confirm** | `vim {{options}} {{file}}` |
| `nano` | 轻量编辑器 | **confirm** | `nano {{options}} {{file}}` |

> `sed` / `vim` / `nano` 之所以是 **confirm**：`sed -i` 会就地改写文件，`vim`/`nano` 保存时会覆盖原文件。

---

## 1. cat — 查看全文 / 拼接

**语法**：`cat [OPTION]... [FILE]...`

最常用的“把文件内容原样打到 stdout”。不指定文件时读标准输入。

### 选项

| 选项 | 含义 |
|---|---|
| `-A, --show-all` | 等价 `-vET`，把不可见字符全显出来（**排查配置文件首选**） |
| `-b, --number-nonblank` | 给**非空**行编号（覆盖 `-n`） |
| `-e` | 等价 `-vE` |
| `-E, --show-ends` | 行尾显示 `$`（可暴露 CRLF 的 `^M`） |
| `-n, --number` | 给所有输出行编号 |
| `-s, --squeeze-blank` | 把连续空行压缩成一行 |
| `-t` | 等价 `-vT` |
| `-T, --show-tabs` | 把 TAB 显示为 `^I` |
| `-u` | （被忽略，为兼容保留） |
| `-v, --show-nonprinting` | 用 `^` 和 `M-` 记法显示非打印字符（LFD 与 TAB 除外） |
| `--help` | 显示帮助并退出 |
| `--version` | 显示版本并退出 |

### 示例

```bash
cat file.txt                 # 查看全文
cat -n file.txt              # 带行号
cat -A config.conf           # 暴露 ^I(TAB) 与 ^M(CRLF)
cat -s messy.log             # 压缩连续空行
cat a.txt b.txt > c.txt      # 拼接（> 会覆盖！）
cat a.txt b.txt >> c.txt     # 拼接（追加）
cat                          # 从 stdin 读，Ctrl-D 结束
```

### 易踩坑

- `cat` 无分页，大文件会把终端刷爆 —— 用 `less`。
- `cat file > file` **会清空文件**（shell 先截断再执行），永远不要这么写。
- 想合并时用 `>>` 而不是 `>`，否则会丢数据。
- `cat -A` 里 `^I` = TAB、`^M` = CR、`$` = 行尾；CRLF 文件会显示成 `^M$`。

**BSD/macOS**：macOS 的 `cat` 无 `-A`，需拆成 `-v -e -t`。

---

## 2. less — 分页查看（推荐）

**语法**：`less [OPTION]... [FILE]...`

比 `more` 强：可前后翻页、可搜索、不加载整个文件。`less` 是 `more` 的增强版，日常应优先用它。

### 交互按键（查看时）

| 按键 | 作用 |
|---|---|
| `Space` / `b` | 下翻一屏 / 上翻一屏 |
| `d` / `u` | 下翻半屏 / 上翻半屏 |
| `j` / `k` 或 `↓` `↑` | 下/上一行 |
| `g` / `G` | 跳到文件头 / 文件尾 |
| `/pattern` | 向下搜索；`n` 下一个，`N` 上一个 |
| `?pattern` | 向上搜索 |
| `F` | 进入 follow 模式（**等价 `tail -f`**，`Ctrl-C` 退出） |
| `-N` | 切换显示行号 |
| `-S` | 切换长行截断（不折行） |
| `:e file` | 打开另一个文件 |
| `q` | 退出 |

### 常用选项

| 选项 | 含义 |
|---|---|
| `-N, --LINE-NUMBERS` | 每行行首显示行号 |
| `-S, --chop-long-lines` | 长行截断而非折行（看宽表首选） |
| `-R, --RAW-CONTROL-CHARS` | 只把 ANSI 颜色序列按原样输出（`git log`/`grep --color` 管道过来必须加） |
| `-r, --raw-control-chars` | 所有控制字符按原样输出（屏幕会乱，慎用） |
| `-i, --ignore-case` | 搜索忽略大小写（模式含大写时失效） |
| `-I, --IGNORE-CASE` | 搜索忽略大小写（**即使模式含大写**） |
| `-F, --quit-if-one-screen` | 内容不足一屏时直接退出（`git log` 首选） |
| `-X, --no-init` | 退出时不清屏（保留内容在终端） |
| `-g, --hilite-search` | 只高亮最近一次搜索命中的那一个串 |
| `-G, --HILITE-SEARCH` | 完全不高亮搜索命中 |
| `-m, --long-prompt` | 用 `more` 风格的详细提示（带百分比） |
| `-M, --LONG-PROMPT` | 比 `-m` 更详细 |
| `-s, --squeeze-blank-lines` | 连续空行压缩成一行 |
| `-e, --quit-at-eof` | 第二次到达 EOF 时自动退出 |
| `-E, --QUIT-AT-EOF` | 第一次到达 EOF 就自动退出 |
| `-c, --clear-screen` | 全屏重绘自上而下（而非滚动） |
| `-f, --force` | 强制打开非普通文件（目录、设备） |
| `-n, --line-numbers` | **关闭**行号（默认开启；大文件下开行号会慢） |
| `-p pattern, --pattern=pattern` | 从第一个匹配 pattern 的位置开始显示 |
| `-t tag, --tag=tag` | 从 ctags 的 tag 位置开始 |
| `-j n, --jump-target=n` | 指定“目标行”在屏幕上的位置 |
| `-x n, --tabs=n` | 设置 tab 宽度（如 `-x4`） |
| `-z n, --window=n` | 设置滚动窗口行数 |
| `-~, --tilde` | EOF 之后显示空行而非 `~` |
| `-u, --underline-special` | 退格与回车当可打印字符处理 |
| `-U, --UNDERLINE-SPECIAL` | 退格/TAB/回车/格式字符当控制字符处理 |
| `-w, --hilite-unread` | 翻整页后高亮第一行新内容 |
| `-W, --HILITE-UNREAD` | 任意移动命令后都高亮第一行新内容 |
| `-q, --quiet, --silent` | 适度静音（不响铃） |
| `-Q, --QUIET, --SILENT` | 完全静音 |
| `-K, --quit-on-intr` | 收到 `Ctrl-C` 立即退出 |
| `-J, --status-column` | 左边缘显示状态列 |
| `-L, --no-lessopen` | 忽略 `LESSOPEN` 环境变量 |
| `-o file, --log-file=file` | 把输入复制到 file（仅管道输入） |
| `-O file, --LOG-FILE=file` | 同 `-o`，但覆盖已存在文件不询问 |
| `-b n, --buffers=n` | 每个文件的缓冲大小（KB；`-1` 为无限制） |
| `-B, --auto-buffers` | 关闭管道输入的自动缓冲分配 |
| `-h n, --max-back-scroll=n` | 向后滚动的最大行数 |
| `-y n, --max-forw-scroll=n` | 向前滚动的最大行数 |
| `--use-color` | 启用彩色文本（配合 `-D` 改色） |
| `--wordwrap` | 折行时尽量在空白处断，不切断单词 |
| `--mouse` / `--MOUSE` | 启用鼠标（后者滚动方向相反） |
| `--incsearch` | 增量搜索（边输入边跳） |
| `--follow-name` | `F` 模式下按文件名重新打开（配合日志轮转） |
| `--header=L,C,N` | 固定表头行/列（看宽表时锁定首行首列） |
| `--save-marks` | 跨会话保存标记 |
| `--no-init` | 同 `-X` |
| `--help`, `-?` | 显示帮助 |
| `-V, --version` | 显示版本 |

> 手册共列出 **107 个**命令行选项（含 `--lesskey-*`、`--proc-*`、`--search-options` 等高级项），上表覆盖日常与运维高频项。

### 环境变量

| 变量 | 作用 |
|---|---|
| `LESS` | 默认选项（如 `export LESS='-R -F -X'`） |
| `LESSOPEN` | 输入预处理器（如自动解压） |
| `LESSCLOSE` | 关闭预处理器时执行 |
| `LESSCHARSET` | 字符集（`utf-8`） |
| `LESSHISTFILE` | 历史文件路径（设 `-` 关闭） |

### 示例

```bash
less big.log                       # 基础分页
less -N -S wide_table.csv          # 行号 + 不折行
grep --color=always x f | less -R  # 保留颜色
git log | less -F -X               # 不足一屏直接退出且不清屏
less +F /var/log/syslog            # 实时跟踪（Ctrl-C 退出跟踪，q 退出）
less -p ERROR app.log              # 直接跳到第一个 ERROR
```

### 易踩坑

- 管道接 `grep --color=always` 时必须加 `-R`，否则满屏 `ESC[31m`。
- `-F` 与 `+F` 不同：`-F` 是“不足一屏就退出”，`+F` 是“启动即 follow”。
- `less` 默认**开**行号（`-n` 是关），大文件下开行号会慢。
- `q` 退出；若已进入 `+F` 跟踪，先 `Ctrl-C` 再 `q`。

**BSD/macOS**：`less` 行为基本一致；macOS 自带版本较旧，部分 `--` 长选项（如 `--header`）可能缺失。

---

## 3. more — 分页查看（基础版）

**语法**：`more [OPTION]... [FILE]...`

只能向前翻，功能比 `less` 少。现在基本被 `less` 取代，但某些精简系统只有 `more`。

### 选项

| 选项 | 含义 |
|---|---|
| `-d, --silent` | 提示 `[Press space to continue, 'q' to quit.]`，非法按键时显示 `[Press 'h' for instructions.]` 而非响铃 |
| `-l, --logical` | 不在含 `^L`(换页符) 的行暂停 |
| `-e, --exit-on-eof` | 到 EOF 时退出（未设 `POSIXLY_CORRECT` 或非终端时默认开启） |
| `-f, --no-pause` | 按**逻辑行**计数而非屏幕行（长行不折行） |
| `-p, --print-over` | 不滚动，清整屏后显示（可执行文件名为 `page` 时自动开启） |
| `-c, --clean-print` | 不滚动，从顶部逐屏绘制并清除每行余部 |
| `-s, --squeeze` | 多个空行压成一个 |
| `-u, --plain` | 抑制下划线（向后兼容，被静默忽略） |
| `-n number, --lines number` | 每屏行数（正整数，覆盖终端推算值） |
| `-number` | 等价于 `--lines`，从第 number 行开始显示 |
| `+number` | 从第 number 行开始 |
| `+/pattern` | 从第一个匹配 pattern 处开始 |
| `-h, --help` | 显示帮助 |
| `-V, --version` | 显示版本 |

### 交互按键

| 按键 | 作用 |
|---|---|
| `Space` | 下翻一屏 |
| `Enter` | 下滚一行 |
| `b` / `Ctrl-B` | 上翻一屏（需终端支持） |
| `/pattern` | 搜索 |
| `h` / `?` | 帮助 |
| `q` | 退出 |

### 示例

```bash
more file.txt
more -d file.txt        # 带操作提示
more +100 file.txt      # 从第 100 行开始
more +/ERROR app.log    # 从第一个 ERROR 开始
```

### 易踩坑

- `more` **不能向上翻**（部分实现支持 `b` 但受限）。
- 长行会折行导致计数混乱，`-f` 可让计数按逻辑行。

**BSD/macOS**：macOS 的 `more` 实际是 `less` 的软链或精简实现，选项支持不全。

---

## 4. head — 看头部

**语法**：`head [OPTION]... [FILE]...`

默认输出前 10 行。

### 选项

| 选项 | 含义 |
|---|---|
| `-n, --lines=[-]NUM` | 输出前 NUM 行（默认 10）；`-NUM` 表示**除末尾 NUM 行外**全部 |
| `-c, --bytes=[-]NUM` | 输出前 NUM 字节；`-NUM` 表示除末尾 NUM 字节外全部 |
| `-q, --quiet, --silent` | 不打印文件名头 |
| `-v, --verbose` | 总是打印文件名头 |
| `-z, --zero-terminated` | 行分隔符为 NUL 而非换行 |
| `--help` | 显示帮助 |
| `--version` | 显示版本 |

> 旧式写法 `head -5 file`、`head -20 file` 仍可用（NUM 前可不带 `-n`）。

### 示例

```bash
head file.txt              # 前 10 行
head -n 20 file.txt        # 前 20 行
head -n -5 file.txt        # 除最后 5 行外的全部
head -c 100 file.bin       # 前 100 字节
head -q *.log              # 多个文件，不显示文件名头
```

### 易踩坑

- `head -n -5`（负数）是 GNU 扩展，BSD 不支持。
- `head -c` 与 `-n` 单位不同：字节 vs 行，处理多字节字符时 `-c` 会截断字符。

**BSD/macOS**：macOS 的 `head -n -5` 不支持负数。

---

## 5. tail — 看尾部 / 跟踪日志

**语法**：`tail [OPTION]... [FILE]...`

默认输出最后 10 行。**运维最高频命令之一**（`tail -f` 看日志）。

### 选项

| 选项 | 含义 |
|---|---|
| `-n, --lines=[+]NUM` | 输出最后 NUM 行（默认 10）；`+NUM` 表示**从第 NUM 行开始** |
| `-c, --bytes=[+]NUM` | 输出最后 NUM 字节；`+NUM` 表示从第 NUM 字节开始 |
| `-f, --follow[={name\|descriptor}]` | 文件增长时持续输出追加内容（缺省参数为 `descriptor`） |
| `-F` | 等价 `--follow=name --retry`（**日志轮转时用这个**） |
| `--retry` | 文件不可访问时持续重试打开 |
| `--pid=PID` | 配合 `-f`，PID 消失后退出（可重复指定多个） |
| `-s, --sleep-interval=N` | `-f` 时每次轮询间隔约 N 秒（默认 1.0） |
| `--max-unchanged-stats=N` | `--follow=name` 下，N 次迭代后文件大小未变则重开（检测轮转，默认 5） |
| `--debug` | 显示实际使用的 `--follow` 实现 |
| `-q, --quiet, --silent` | 不输出文件名头 |
| `-v, --verbose` | 总是输出文件名头 |
| `-z, --zero-terminated` | 行分隔符为 NUL |
| `--help` | 显示帮助 |
| `--version` | 显示版本 |

### `-f` 与 `-F` 的区别（重要）

| | `-f`（= `--follow=descriptor`） | `-F`（= `--follow=name --retry`） |
|---|---|---|
| 跟踪对象 | 文件描述符 | 文件名 |
| 日志被轮转（rename + 新建同名文件） | **跟丢**，仍盯着旧 inode | **自动跟上**新文件 |
| 文件被删除重建 | 失效 | 自动重试重开 |
| 适用场景 | 临时看 | **长期跟踪生产日志** |

### 示例

```bash
tail file.txt               # 最后 10 行
tail -n 50 file.txt         # 最后 50 行
tail -n +2 file.txt         # 从第 2 行到结尾（跳过表头）
tail -f app.log             # 实时跟踪
tail -F /var/log/nginx/access.log   # 生产环境跟踪（抗轮转）
tail -f app.log | grep ERROR        # 只看 ERROR
tail -f --pid=1234 app.log          # 进程 1234 退出后自动停止
tail -n 100 -f app.log              # 先显示最后 100 行再跟踪
```

### 易踩坑

- `tail -f` 后按 `Ctrl-C` 退出；在脚本里长时间运行要留意。
- 日志轮转场景**务必用 `-F`**，`-f` 会跟丢。
- `tail -n +1` = 从头开始（等价 `cat`）。

**BSD/macOS**：macOS 的 `tail -r` 可反转行序，Linux GNU tail 无此选项；`-F` 两者都有。

---

## 6. grep — 文本搜索

**语法**：`grep [OPTION]... PATTERNS [FILE]...`

### 模式解释

| 选项 | 含义 |
|---|---|
| `-E, --extended-regexp` | 按扩展正则（ERE）解释 |
| `-F, --fixed-strings` | 按固定字符串解释（不解析正则，**最快**） |
| `-G, --basic-regexp` | 按基本正则（BRE）解释（**默认**） |
| `-P, --perl-regexp` | 按 Perl 兼容正则（PCRE）解释 |
| `-e PATTERNS, --regexp=PATTERNS` | 指定模式（可多次；也可保护以 `-` 开头的模式） |
| `-f FILE, --file=FILE` | 从文件逐行读取模式（`-` 表示 stdin） |

### 匹配控制

| 选项 | 含义 |
|---|---|
| `-i, --ignore-case` | 忽略大小写 |
| `--no-ignore-case` | 不忽略大小写（默认；用于抵消脚本里已有的 `-i`） |
| `-v, --invert-match` | 反向选择（输出**不**匹配的行） |
| `-w, --word-regexp` | 只匹配**完整单词** |
| `-x, --line-regexp` | 只匹配**整行** |

### 输出控制

| 选项 | 含义 |
|---|---|
| `-c, --count` | 只输出匹配行数（配合 `-v` 则统计不匹配行数） |
| `--color[=WHEN], --colour[=WHEN]` | 高亮匹配（WHEN 为 `never`/`always`/`auto`） |
| `-L, --files-without-match` | 只输出**不含**匹配的文件名 |
| `-l, --files-with-matches` | 只输出**含**匹配的文件名 |
| `-m NUM, --max-count=NUM` | 每个文件最多匹配 NUM 行后停止 |
| `-o, --only-matching` | 只输出匹配到的部分（每个一行） |
| `-q, --quiet, --silent` | 静默；有匹配即返回 0 |
| `-s, --no-messages` | 抑制文件不存在/不可读的错误信息 |
| `-b, --byte-offset` | 输出匹配行的字节偏移 |
| `-H, --with-filename` | 输出文件名（多文件时默认开） |
| `-h, --no-filename` | 不输出文件名前缀（单文件时默认开） |
| `-n, --line-number` | 输出行号 |
| `-T, --initial-tab` | 让内容首字符对齐到制表位（配合 `-H`/`-n`/`-b`） |
| `-Z, --null` | 文件名后输出 NUL 而非换行（配合 `xargs -0`） |
| `--label=LABEL` | 把 stdin 标记为 LABEL 文件名 |

### 上下文

| 选项 | 含义 |
|---|---|
| `-A NUM, --after-context=NUM` | 额外输出匹配行之后的 NUM 行 |
| `-B NUM, --before-context=NUM` | 额外输出匹配行之前的 NUM 行 |
| `-C NUM, -NUM, --context=NUM` | 前后各 NUM 行 |
| `--group-separator=SEP` | 组间分隔符（默认 `--`） |
| `--no-group-separator` | 不输出组间分隔符 |

### 文件与目录选择

| 选项 | 含义 |
|---|---|
| `-r, --recursive` | 递归搜索目录（不跟符号链接，除非在命令行上） |
| `-R, --dereference-recursive` | 递归搜索并**跟随所有**符号链接 |
| `-d ACTION, --directories=ACTION` | 目录处理方式：`read`(默认)/`skip`/`recurse` |
| `-D ACTION, --devices=ACTION` | 设备/FIFO/socket 处理：`read`(默认)/`skip` |
| `--include=GLOB` | 只搜索 basename 匹配 GLOB 的文件 |
| `--exclude=GLOB` | 跳过 basename 匹配 GLOB 的文件 |
| `--exclude-from=FILE` | 从 FILE 读取排除的 glob |
| `--exclude-dir=GLOB` | 跳过匹配 GLOB 的目录 |
| `-a, --text` | 把二进制当文本处理 |
| `-I` | 把二进制当无匹配处理（等价 `--binary-files=without-match`） |
| `--binary-files=TYPE` | 二进制文件类型：`binary`(默认)/`text`/`without-match` |
| `-U, --binary` | 强制按二进制处理（Windows 相关） |
| `--line-buffered` | 行缓冲输出（有性能损耗，管道实时性需要时用） |
| `-z, --null-data` | 输入输出按 NUL 分隔 |

### 退出码

| 码 | 含义 |
|---|---|
| 0 | 有匹配 |
| 1 | 无匹配 |
| 2 | 出错（文件不存在、正则非法等） |

### 示例

```bash
grep "error" app.log                    # 基础搜索
grep -i "error" app.log                 # 忽略大小写
grep -rn "TODO" src/                    # 递归 + 行号
grep -rn --include='*.ts' "TODO" .      # 只在 .ts 文件里搜
grep -rn --exclude-dir=node_modules "TODO" .
grep -c "404" access.log                # 统计匹配行数
grep -v "^#" config.conf                # 排除注释行
grep -A 3 -B 1 "Exception" app.log      # 上下文
grep -w "cat" words.txt                 # 整词匹配
grep -E "^(ERROR|WARN)" app.log         # 扩展正则
grep -F "[error]" app.log               # 固定串（不解析 []）
grep -l "pattern" *.txt                 # 只列文件名
grep -oE '[0-9]+' file                  # 只输出匹配片段
grep -P '(?<=user=)\w+' app.log         # PCRE 后行断言
```

### 易踩坑

- 默认是 **BRE**：`+ ? { } | ( )` 需转义；想省事用 `-E`。
- 搜含正则元字符的字符串（如 `1.2.3`）要 `-F`，否则 `.` 匹配任意字符。
- `-r` **不跟符号链接**，需要跟用 `-R`。
- `-l` 遇到第一个匹配就停止扫描该文件，比 `-n` 快。
- `--color=always` 管道给 `less` 时，`less` 要加 `-R`。

**BSD/macOS**：macOS 的 BSD grep 无 `-P`（需 `brew install grep` 用 `ggrep`），`--exclude-dir` 支持也较晚。

---

## 7. sed — 流编辑

**语法**：`sed [OPTION]... {script} [input-file]...`

逐行读入、按脚本处理、输出。**`-i` 会就地改写文件**，属于危险操作。

### 选项

| 选项 | 含义 |
|---|---|
| `-n, --quiet, --silent` | 抑制模式空间的自动打印（配合 `p` 只打印指定行） |
| `-e script, --expression=script` | 追加脚本片段（可多次） |
| `-f script-file, --file=script-file` | 从文件读取脚本 |
| `-i[SUFFIX], --in-place[=SUFFIX]` | **就地编辑**；给了 SUFFIX 则先备份（如 `-i.bak`） |
| `--follow-symlinks` | 就地编辑时跟随符号链接 |
| `-E, -r, --regexp-extended` | 使用扩展正则（可移植性建议用 POSIX 的 `-E`） |
| `-s, --separate` | 把多个文件当独立流处理（而非一个长流） |
| `-z, --null-data` | 以 NUL 分隔行 |
| `-u, --unbuffered` | 少读多刷，降低输出延迟 |
| `-l N, --line-length=N` | `l` 命令的折行长度 |
| `--posix` | 禁用所有 GNU 扩展 |
| `--sandbox` | 沙箱模式（禁用 `e`/`r`/`w` 命令） |
| `--debug` | 注解脚本执行过程 |
| `--help` | 显示帮助 |
| `--version` | 显示版本 |

### 常用脚本命令

| 命令 | 作用 |
|---|---|
| `s/regexp/replacement/flags` | 替换。flags：`g` 全局、`N` 第 N 个、`p` 打印、`i`/`I` 忽略大小写、`w file` 写文件 |
| `d` | 删除模式空间，开始下一轮 |
| `p` | 打印模式空间（常配 `-n`） |
| `a text` | 在匹配行**后**追加 text |
| `i text` | 在匹配行**前**插入 text |
| `c text` | 用 text **替换**整行 |
| `y/src/dst/` | 字符逐一转换 |
| `q [exit-code]` | 立即退出（不再处理后续输入） |
| `n` / `N` | 读下一行到模式空间（`N` 追加而非替换） |
| `h` / `H` / `g` / `G` / `x` | 保持空间与模式空间之间存取/交换 |
| `b label` / `t label` / `T label` | 跳转 / 条件跳转（`t` 上次替换成功则跳） |
| `=` | 打印当前行号 |

### 地址（作用范围）

| 形式 | 含义 |
|---|---|
| `N` | 第 N 行 |
| `$` | 最后一行 |
| `/regexp/` | 匹配 regexp 的行 |
| `/regexp/I` | 忽略大小写匹配 |
| `/regexp/M` | 多行模式匹配 |
| `first~step` | 从 first 起每 step 行 |
| `addr1,addr2` | 从 addr1 到 addr2 |
| `addr1,+N` | 从 addr1 起往后 N 行 |
| `addr!` | 取反 |

### 示例

```bash
sed 's/foo/bar/' file.txt              # 每行第一个 foo 换 bar
sed 's/foo/bar/g' file.txt             # 全局替换
sed -i 's/foo/bar/g' file.txt          # 就地替换（危险！）
sed -i.bak 's/foo/bar/g' file.txt      # 就地替换并备份为 file.txt.bak
sed -n '5,10p' file.txt                # 只打印第 5-10 行
sed -n '/ERROR/p' app.log              # 只打印含 ERROR 的行
sed '/^#/d' config.conf                # 删除注释行
sed '/^$/d' file.txt                   # 删除空行
sed 's/#.*//' config.conf              # 去掉行内注释
sed -E 's/(\w+)@(\w+)/\2@\1/' f.txt    # 扩展正则 + 捕获组交换
sed '2a\新的一行' file.txt              # 第 2 行后追加
sed '1i\表头' file.txt                  # 第 1 行前插入
sed 's/[[:space:]]\+/ /g' file.txt     # 多空白压成单个空格
sed -n '/start/,/end/p' file.txt       # 打印两模式之间
```

### 易踩坑

- `-i` **无输出**且直接改原文件；先在 `-n` 下试跑再 `-i`。
- GNU 的 `-i` 与 BSD 的 `-i` 语法不同（见下）。
- `-i` 会**断开硬链接**并**替换符号链接**（GNU 加 `--follow-symlinks` 可改行为）。
- 分隔符可换：`s|/a/b|/c/d|` 避免转义斜杠。
- `-n` 与 `p` 是黄金搭档：`-n '/x/p'` = 只输出匹配行。

**BSD/macOS**：macOS 的 `sed -i` **必须**带参数（哪怕是空串）：`sed -i '' 's/a/b/' file`，否则会把下一个参数当备份后缀。GNU 允许 `-i` 裸用。

---

## 8. awk — 列处理 / 报表

**语法**：`awk [POSIX or GNU style options] -f progfile [--] file...`
`awk [POSIX or GNU style options] [--] 'program' file...`

Linux 上的 `awk` 通常是 **gawk**（GNU awk）。下表为 gawk 的选项。

### 程序结构

```
pattern { action statements }
```

- `pattern` 为真时执行 `action`；省略 action 默认打印 `$0`。
- 特殊模式：`BEGIN { ... }`（读输入前）、`END { ... }`（读完后）。
- 内置变量：`$0` 整行、`$1..$NF` 字段、`NF` 字段数、`NR` 当前行号、`FNR` 文件内行号、`FS` 输入分隔符、`OFS` 输出分隔符、`RS` 记录分隔符、`ORS` 输出记录分隔符、`FILENAME` 文件名、`ARGC`/`ARGV`。
- 内置函数：`print`、`printf`、`length`、`substr`、`index`、`split`、`sub`/`gsub`、`match`、`toupper`/`tolower`、`sprintf`、`int`、`gensub`(gawk)。

### 选项

| 选项 | 含义 |
|---|---|
| `-f program-file, --file program-file` | 从文件读取 awk 程序（可多次） |
| `-F fs, --field-separator fs` | 设置输入字段分隔符 `FS` |
| `-v var=val, --assign var=val` | 程序执行前给变量赋值 |
| `-e program-text, --source program-text` | 用 program-text 作为源码 |
| `-E file, --exec file` | 类似 `-f`，但作为**最后**处理的选项（`#!` 脚本/CGI 用） |
| `-i include-file, --include include-file` | 加载 awk 源库 |
| `-l lib, --load lib` | 加载 gawk 扩展共享库 |
| `-k, --csv` | 启用 CSV 特殊处理 |
| `-c, --traditional` | 兼容模式 |
| `-P, --posix` | POSIX 模式（禁用诸多扩展） |
| `-L [value], --lint[=value]` | 对可疑/不可移植构造给出警告 |
| `-M, --bignum` | 强制任意精度运算（需 MPFR/GMP） |
| `-N, --use-lc-numeric` | 用本地小数点字符解析数字 |
| `-n, --non-decimal-data` | 识别输入中的八进制/十六进制（**慎用**） |
| `-b, --characters-as-bytes` | 按单字节字符处理所有输入 |
| `-O, --optimize` | 启用默认优化（默认行为） |
| `-s, --no-optimize` | 禁用默认优化 |
| `-d[file], --dump-variables[=file]` | 输出全局变量列表（默认 `awkvars.out`） |
| `-o[file], --pretty-print[=file]` | 输出美化后的程序（默认 `awkprof.out`） |
| `-p[prof-file], --profile[=prof-file]` | 性能剖析（默认 `awkprof.out`） |
| `-D[file], --debug[=file]` | 启用调试器 |
| `-I, --trace` | 打印执行的内部字节码名 |
| `-g, --gen-pot` | 生成 `.pot` 本地化模板 |
| `-S, --sandbox` | 沙箱模式 |
| `-C, --copyright` | 打印版权信息 |
| `-h, --help` | 显示帮助 |
| `-V, --version` | 显示版本 |
| `--` | 选项结束标记（允许程序参数以 `-` 开头） |

### 示例

```bash
awk '{print $1}' file.txt                  # 打印第 1 列
awk '{print $1, $3}' file.txt              # 打印第 1、3 列
awk -F: '{print $1}' /etc/passwd           # 以 : 为分隔符取第 1 列
awk -F, '{print $2}' data.csv              # CSV 取第 2 列
awk '{print NR": "$0}' file.txt            # 带行号
awk '$3 > 100 {print $1}' data.txt         # 条件过滤
awk '/ERROR/ {print}' app.log              # 类似 grep
awk '/ERROR/ && /timeout/' app.log         # 多条件
awk '{sum += $1} END {print sum}' nums.txt # 求和
awk '{sum += $1} END {print sum/NR}' nums  # 求平均
awk '!seen[$0]++' file.txt                 # 去重（保持顺序）
awk 'END {print NR}' file.txt              # 统计行数（等价 wc -l）
awk -v threshold=50 '$2 > threshold' f.txt # 传入外部变量
awk 'BEGIN {OFS=","} {$1=$1; print}' f.txt # 重排分隔符为逗号
awk '{a[$1]+=$2} END {for(k in a) print k, a[k]}' f.txt   # 分组汇总
printf '%s\n' "$(awk 'NF' file)"           # 删除空行
awk 'length > 80' file.txt                 # 打印超长行
```

### 易踩坑

- 默认分隔符是**空白（空格/TAB 混合）**，`-F` 才是显式分隔符。
- `$1` 是第 1 列，`$0` 是整行；`$NF` 是最后一列。
- 单引号包裹程序，shell 不会展开 `$` —— 这是正确的。
- 想用 shell 变量需 `-v` 或 `-v name="$VAR"`，不能直接写 `$VAR`。
- `awk '{print $1}'` 的输出分隔符是空格（`OFS`），不是原分隔符。

**BSD/macOS**：macOS 的 `awk` 是较老的 BWK awk（`onetrue awk`），无 `--csv`、`--lint`、`gensub` 等 gawk 扩展；需要 gawk 请 `brew install gawk`。

---

## 9. cut — 按列截取

**语法**：`cut OPTION... [FILE]...`

按字节、字符或字段截取每行。**不擅长处理多空格对齐的列**（那是 `awk` 的活）。

### 选项

| 选项 | 含义 |
|---|---|
| `-b, --bytes=LIST` | 只选这些**字节**位置 |
| `-c, --characters=LIST` | 只选这些**字符**位置 |
| `-f, --fields=LIST` | 只选这些**字段**（默认 TAB 分隔） |
| `-d, --delimiter=DELIM` | 用 DELIM 代替 TAB 作字段分隔符 |
| `-s, --only-delimited` | 不打印不含分隔符的行 |
| `--complement` | 取补集（反选） |
| `-O, --output-delimiter=STRING` | 输出分隔符（默认同输入分隔符） |
| `-n` | 配合 `-b`，不输出被截断的多字节字符 |
| `-z, --zero-terminated` | 行分隔符为 NUL |
| `-w, --whitespace-delimited[=trimmed]` | 以连续空白为字段分隔符；`trimmed` 时忽略首尾空白 |
| `-F LIST` | 类似 `-f`，但隐含 `-w` 与 `-O ' '` |
| `--help` | 显示帮助 |
| `--version` | 显示版本 |

### LIST 语法

| 形式 | 含义 |
|---|---|
| `N` | 第 N 项 |
| `N-` | 第 N 项到结尾 |
| `N-M` | 第 N 到第 M 项 |
| `-M` | 第 1 到第 M 项 |
| `N,M,...` | 多个不连续项 |

### 示例

```bash
cut -d: -f1 /etc/passwd            # 以 : 分隔取第 1 列
cut -d: -f1,7 /etc/passwd          # 取第 1、7 列
cut -d, -f2- data.csv              # 取第 2 列到末尾
cut -c1-10 file.txt                # 每行前 10 个字符
cut -c5- file.txt                  # 从第 5 个字符到行尾
cut -d, -f1 --complement d.csv     # 除第 1 列外全部
cut -d, -f1,3 --output-delimiter='|' d.csv   # 输出改分隔符
cut -d: -f1 -s /etc/passwd         # 跳过无分隔符的行
```

### 易踩坑

- `cut` **不支持**“以任意空白为分隔符”（GNU 的 `-w` 可部分解决），多空格对齐的列用 `awk`。
- `-c` 按**字符**，`-b` 按**字节**；UTF-8 中文一个字符 3 字节，用 `-b` 会截断乱码。
- 不指定 `-d` 时 `-f` 以 TAB 分隔，而**不是空格**。
- 行内没有分隔符时，`-f` 默认打印整行；`-s` 可跳过。

**BSD/macOS**：macOS 的 `cut` 无 `--complement`、`-w`、`-F`。

---

## 10. sort — 排序

**语法**：`sort [OPTION]... [FILE]...`

默认按整行字典序升序。

### 排序键（决定怎么比）

| 选项 | 含义 |
|---|---|
| `-n, --numeric-sort` | 按数值比较 |
| `-g, --general-numeric-sort` | 按通用数值（支持科学计数法） |
| `-h, --human-numeric-sort` | 按人类可读数字（`2K`、`1G`） |
| `-M, --month-sort` | 按月份名（`JAN` < ... < `DEC`） |
| `-V, --version-sort` | 版本号自然排序 |
| `-R, --random-sort` | 随机打乱，但相同键归组 |
| `--sort=WORD` | 指定排序方式：`general-numeric`/`human-numeric`/`month`/`numeric`/`random`/`version` |

### 比较修饰

| 选项 | 含义 |
|---|---|
| `-b, --ignore-leading-blanks` | 忽略前导空白 |
| `-d, --dictionary-order` | 只考虑空白与字母数字 |
| `-f, --ignore-case` | 忽略大小写 |
| `-i, --ignore-nonprinting` | 只考虑可打印字符 |
| `-r, --reverse` | 反序 |
| `-s, --stable` | 稳定排序（禁用最后手段比较） |
| `-u, --unique` | 相同键只输出第一条；配合 `-c` 检查严格有序 |
| `-k, --key=KEYDEF` | 按指定键排序（KEYDEF 给出位置与类型） |
| `-t, --field-separator=SEP` | 字段分隔符 |

### 输出与其他

| 选项 | 含义 |
|---|---|
| `-o, --output=FILE` | 结果写到 FILE（可与输入同名，安全） |
| `-m, --merge` | 合并已排序文件（不再排序） |
| `-c, --check, --check=diagnose-first` | 检查是否已排序（不排序） |
| `-C, --check=quiet, --check=silent` | 同 `-c`，但不报首个乱序行 |
| `-z, --zero-terminated` | 行分隔符为 NUL |
| `-S, --buffer-size=SIZE` | 主内存缓冲大小 |
| `-T, --temporary-directory=DIR` | 临时目录（可多次指定） |
| `--parallel=N` | 并行排序线程数 |
| `--batch-size=NMERGE` | 一次最多合并 NMERGE 个输入 |
| `--compress-program=PROG` | 用 PROG 压缩临时文件 |
| `--files0-from=F` | 从 F 中 NUL 分隔的文件名读取输入 |
| `--random-source=FILE` | 从 FILE 取随机字节 |
| `--debug` | 注解用于排序的部分并告警可疑用法 |
| `--help` | 显示帮助 |
| `--version` | 显示版本 |

### KEYDEF 语法

`-k F[.C][OPTS][,F[.C][OPTS]]`

- `F` 字段号（1 起），`C` 字段内字符位置（1 起）。
- `OPTS` 可为 `n`/`g`/`h`/`M`/`V`/`r`/`b`/`d`/`f`/`i`。
- 例：`-k2,2n` 表示“第 2 字段，按数值”。

### 示例

```bash
sort file.txt                      # 字典序
sort -n nums.txt                   # 数值序
sort -rn nums.txt                  # 数值倒序
sort -k2 -n data.txt               # 按第 2 列数值排序
sort -t: -k3 -n /etc/passwd        # 以 : 分隔按第 3 列数值
sort -u file.txt                   # 排序并去重
sort -h sizes.txt                  # 按 1K/2M/3G 大小排序
sort -V versions.txt               # 版本号排序
sort -o file.txt file.txt          # 原地排序（安全写法）
sort -c file.txt                   # 检查是否已排序
sort -f -k1,1 file.txt             # 按第 1 列忽略大小写
LC_ALL=C sort file.txt             # 按字节序（最快、最可预测）
```

### 易踩坑

- 默认按**本地化**字典序，`LC_ALL=C` 才是纯字节序（脚本中建议显式指定）。
- `-n` 只认数字开头，`2abc` 会被当 2；混有单位的用 `-h`。
- `sort file > file` **会清空文件**（shell 先截断）；用 `-o file`。
- `-k2` 默认从第 2 字段起**到行尾**比较，只想比第 2 字段用 `-k2,2`。

**BSD/macOS**：macOS 的 `sort` 无 `-h`、`-V`（`-V` 支持较晚），无 `--parallel`、`--debug`。

---

## 11. uniq — 去重（需先排序）

**语法**：`uniq [OPTION]... [INPUT [OUTPUT]]`

**只去除相邻重复行**，所以几乎总是先 `sort`。

### 选项

| 选项 | 含义 |
|---|---|
| `-c, --count` | 行首加上出现次数 |
| `-d, --repeated` | 只输出有重复的行（每组一行） |
| `-D` | 输出**所有**重复行 |
| `--all-repeated[=METHOD]` | 类似 `-D`，允许空行分隔组；METHOD：`none`(默认)/`prepend`/`separate` |
| `-u, --unique` | 只输出不重复的行 |
| `-i, --ignore-case` | 比较时忽略大小写 |
| `-f, --skip-fields=N` | 跳过前 N 个字段再比较 |
| `-s, --skip-chars=N` | 跳过前 N 个字符再比较 |
| `-w, --check-chars=N` | 最多比较 N 个字符 |
| `--group[=METHOD]` | 显示所有行，用空行分组；METHOD：`separate`(默认)/`prepend`/`append`/`both` |
| `-z, --zero-terminated` | 行分隔符为 NUL |
| `--help` | 显示帮助 |
| `--version` | 显示版本 |

### 示例

```bash
sort file.txt | uniq                   # 去重
sort file.txt | uniq -c                # 去重并计数
sort file.txt | uniq -c | sort -rn     # 按出现次数倒序（经典组合）
sort file.txt | uniq -d                # 只看重复行
sort file.txt | uniq -u                # 只看唯一行
sort -f file.txt | uniq -i             # 忽略大小写去重
uniq -f 1 file.txt                     # 跳过第 1 个字段比较
sort -u file.txt                       # 等价于 sort | uniq（更省一次遍历）
```

### 易踩坑

- `uniq` **不去除非相邻的重复**：必须先 `sort`，否则漏掉大量重复。
- `sort -u` 与 `sort | uniq` 结果相同，前者更快。
- `-c` 计的是**连续出现次数**，未排序时会分多段统计。

**BSD/macOS**：macOS 的 `uniq` 无 `--group`、`-D`（有 `-d`），`--all-repeated` 缺失。

---

## 12. wc — 统计行/词/字节

**语法**：`wc [OPTION]... [FILE]...`

默认输出 **行数 单词数 字节数**。

### 选项

| 选项 | 含义 |
|---|---|
| `-l, --lines` | 统计**换行符**个数（行数） |
| `-w, --words` | 统计单词数 |
| `-c, --bytes` | 统计字节数 |
| `-m, --chars` | 统计字符数 |
| `-L, --max-line-length` | 最长行的显示宽度 |
| `--total=WHEN` | 何时输出总计行：`auto`/`always`/`only`/`never` |
| `--files0-from=F` | 从 F 的 NUL 分隔文件名读输入 |
| `--debug` | 显示使用的行计数加速方式 |
| `--help` | 显示帮助 |
| `--version` | 显示版本 |

### 示例

```bash
wc file.txt                  # 行 词 字节
wc -l file.txt               # 只统计行数
wc -l *.log                  # 多文件 + 总计
wc -m file.txt               # 字符数（UTF-8 中文按 1 字符算）
wc -c file.txt               # 字节数
wc -L file.txt               # 最长行宽度
wc -l < file.txt             # 只要数字，不带文件名（脚本常用）
find . -name '*.ts' | wc -l  # 统计文件个数
cat file.txt | wc -l         # 等价 wc -l file.txt
```

### 易踩坑

- `-l` 统计的是**换行符数量**：末行若无换行符则不计数（与“视觉行数”可能差 1）。
- `-c` 是字节、`-m` 是字符；UTF-8 下两者对中文差异很大。
- `wc -l file` 输出带文件名；只要数字用 `wc -l < file` 或 `cat file | wc -l`。

**BSD/macOS**：macOS 的 `wc` 无 `--total`、`--files0-from`。

---

## 13. diff — 比较文件

**语法**：`diff [OPTION]... FILES`

比较两个文件（或目录）差异。**输出格式决定后续怎么用**。

### 输出格式

| 选项 | 含义 |
|---|---|
| `--normal` | 普通格式（默认） |
| `-c, -C NUM, --context[=NUM]` | 上下文格式（默认 3 行） |
| `-u, -U NUM, --unified[=NUM]` | 统一格式（默认 3 行，**最常用**，即 `patch` 格式） |
| `-e, --ed` | ed 脚本格式 |
| `-n, --rcs` | RCS 格式 |
| `-y, --side-by-side` | 左右两列并排 |
| `-q, --brief` | 只报告是否不同 |
| `-s, --report-identical-files` | 相同也报告 |
| `-D, --ifdef=NAME` | 输出带 `#ifdef NAME` 的合并文件 |

### 并排格式相关

| 选项 | 含义 |
|---|---|
| `-W, --width=NUM` | 输出宽度（默认 130） |
| `--left-column` | 相同行只显示左列 |
| `--suppress-common-lines` | 不显示相同行 |
| `-t, --expand-tabs` | 展开 TAB 为空格 |
| `-T, --initial-tab` | 前置 TAB 对齐 |
| `--tabsize=NUM` | TAB 宽度（默认 8） |
| `--suppress-blank-empty` | 抑制空输出行前的空格/TAB |

### 忽略差异（**比对配置文件时最关键**）

| 选项 | 含义 |
|---|---|
| `-i, --ignore-case` | 忽略大小写 |
| `-w, --ignore-all-space` | 忽略**所有**空白 |
| `-b, --ignore-space-change` | 忽略空白数量变化 |
| `-Z, --ignore-trailing-space` | 忽略行尾空白 |
| `-E, --ignore-tab-expansion` | 忽略 TAB 展开造成的差异 |
| `-B, --ignore-blank-lines` | 忽略纯空行变化 |
| `-I RE, --ignore-matching-lines=RE` | 忽略所有行都匹配 RE 的变化 |
| `-a, --text` | 把所有文件当文本 |
| `--strip-trailing-cr` | 去掉输入行尾的 CR（**CRLF 与 LF 混用时救命**） |

### 目录比较

| 选项 | 含义 |
|---|---|
| `-r, --recursive` | 递归比较子目录 |
| `-N, --new-file` | 缺失文件当空文件（配合 `-r` 可做完整 diff） |
| `--unidirectional-new-file` | 缺失的**第一个**文件当空文件 |
| `-x PAT, --exclude=PAT` | 排除匹配 PAT 的文件 |
| `-X FILE, --exclude-from=FILE` | 从 FILE 读取排除模式 |
| `-S FILE, --starting-file=FILE` | 从 FILE 开始比较 |
| `--from-file=FILE1` | FILE1 与所有操作数比较 |
| `--to-file=FILE2` | 所有操作数与 FILE2 比较 |
| `--ignore-file-name-case` | 忽略文件名大小写 |
| `--no-ignore-file-name-case` | 考虑文件名大小写 |
| `--no-dereference` | 不跟随符号链接 |

### 其他

| 选项 | 含义 |
|---|---|
| `-d, --minimal` | 尽力找更小的差异集 |
| `--horizon-lines=NUM` | 保留 NUM 行公共前缀/后缀 |
| `--speed-large-files` | 假设大文件、多处分散小改动 |
| `-l, --paginate` | 输出经 `pr` 分页 |
| `--color[=WHEN]` | 彩色输出（`never`/`always`/`auto`） |
| `--palette=PALETTE` | `--color` 生效时的配色 |
| `--label LABEL` | 用 LABEL 替代文件名与时间戳（可重复） |
| `--GTYPE-group-format=GFMT` | 自定义 `GTYPE` 组的格式 |
| `--line-format=LFMT` | 自定义所有输入行的格式 |
| `--LTYPE-line-format=LFMT` | 自定义 `LTYPE` 输入行的格式 |
| `--help` | 显示帮助 |
| `-v, --version` | 显示版本 |

### 示例

```bash
diff a.txt b.txt                      # 普通格式
diff -u a.txt b.txt                   # 统一格式（生成 patch）
diff -u a.txt b.txt > fix.patch       # 存为补丁
diff -y a.txt b.txt                   # 并排对比
diff -y --suppress-common-lines a b   # 只显示不同
diff -w a.txt b.txt                   # 忽略所有空白
diff -B a.txt b.txt                   # 忽略空行差异
diff --strip-trailing-cr a.txt b.txt  # 忽略 CRLF/LF 差异
diff -rq dir1 dir2                    # 递归只看哪些文件不同
diff -ruN dir1 dir2 > all.patch       # 目录级补丁（含新增文件）
diff -q a.txt b.txt                   # 只问是否不同
```

### 退出码

| 码 | 含义 |
|---|---|
| 0 | 无差异 |
| 1 | 有差异 |
| 2 | 出错 |

### 易踩坑

- 退出码 1 表示“有差异”而非错误 —— 脚本里 `if diff a b; then` 要留意。
- 只看“是否相同”用 `diff -q`（快）。
- 生成的补丁用 `patch -p1 < fix.patch` 应用。
- 目录比较想包含“新增文件”，需要 `-N`。

**BSD/macOS**：macOS 的 `diff` 无 `--color`、`--palette`，`--suppress-blank-empty` 缺失。

---

## 14. vim — 全功能编辑器

**语法**：`vim [options] [file ..]`

### 三种模式（核心）

| 模式 | 进入方式 | 说明 |
|---|---|---|
| 普通模式 | 启动即进入 / 任何模式按 `Esc` | 移动、删除、复制、粘贴 |
| 插入模式 | `i` `a` `o` `O` `I` `A` | 输入文本 |
| 命令行模式 | `:` `/` `?` | 保存、退出、搜索、替换 |

### 必会操作

| 操作 | 按键 |
|---|---|
| 保存 | `:w` |
| 退出 | `:q` |
| 保存并退出 | `:wq` 或 `ZZ` |
| 强制退出（不保存） | `:q!` |
| 保存并强制退出 | `:wq!`（只读文件） |
| 进入插入（光标前/后） | `i` / `a` |
| 进入插入（下/上新行） | `o` / `O` |
| 删除整行 | `dd`（`5dd` 删 5 行） |
| 复制整行 | `yy`（`5yy` 复制 5 行） |
| 粘贴 | `p`（后）/ `P`（前） |
| 撤销 / 重做 | `u` / `Ctrl-R` |
| 搜索 | `/pattern`，`n` 下一个，`N` 上一个 |
| 替换当前行 | `:s/old/new/g` |
| 替换全文 | `:%s/old/new/g` |
| 替换全文（逐个确认） | `:%s/old/new/gc` |
| 跳到指定行 | `:42` 或 `42G` |
| 跳到文件头/尾 | `gg` / `G` |
| 显示行号 | `:set nu`（关 `:set nonu`） |
| 打开另一个文件 | `:e file` |
| 不保存退出 | `:q!` |

### 命令行选项（节选，手册共 80+ 项）

| 选项 | 含义 |
|---|---|
| `+[num]` | 光标定位到第 num 行（省略 num 则到末行） |
| `+/{pat}` | 定位到第一个匹配 {pat} 的行 |
| `+{command}` / `-c {command}` | 读入首个文件后执行 Ex 命令（最多 10 个） |
| `--cmd {command}` | 在处理任何 vimrc **之前**执行命令 |
| `-b` | 二进制模式 |
| `-C` | 兼容模式（设为 `compatible`，尽量像 Vi） |
| `-N` | 非兼容模式（重置 `compatible`） |
| `-d` | diff 模式（`vimdiff`） |
| `-R` | 只读模式（隐含 `-n`） |
| `-m` | 禁止修改文件（可改缓冲，不能写盘） |
| `-M` | 完全禁止修改 |
| `-n` | 不使用交换文件（崩溃无法恢复） |
| `-o[N]` | 水平分屏打开 N 个窗口 |
| `-O[N]` | 垂直分屏打开 N 个窗口 |
| `-p[N]` | 打开 N 个标签页 |
| `-r` | 列出交换文件 |
| `-r {file}` | 恢复模式（用交换文件恢复崩溃会话） |
| `-S {file}` | 读入首个文件后 source {file}（缺省 `Session.vim`） |
| `-u {vimrc}` | 使用指定 vimrc（`-u NONE` 跳过所有初始化） |
| `-U {gvimrc}` | 使用指定 gvimrc |
| `-i {viminfo}` | 指定 viminfo 文件（`NONE` 表示不用） |
| `-s {scriptin}` | 把脚本文件当键盘输入回放 |
| `-w {scriptout}` | 记录所有按键到文件 |
| `-W {scriptout}` | 同 `-w`，但覆盖已存在文件 |
| `-V[N]` | 详细模式（`verbose` 级别，默认 10） |
| `-x` | 加密写入（提示输入密钥） |
| `-X` | 不连接 X server（启动更快） |
| `-y` | easy 模式（像点击即输入的编辑器） |
| `-Z` | 受限模式 |
| `--clean` | 不使用任何个人配置（排查问题用） |
| `--noplugin` | 跳过加载插件 |
| `--nofork` | 前台运行（GUI 版不分叉） |
| `--not-a-term` | 告知输入/输出非终端，避免警告与 2 秒延迟 |
| `--startuptime {file}` | 记录启动耗时到文件 |
| `--version` | 显示版本 |
| `--help, -h, -?` | 显示帮助 |

### 示例

```bash
vim file.txt                # 打开文件
vim +42 file.txt            # 打开并跳到第 42 行
vim +/ERROR app.log         # 打开并定位到第一个 ERROR
vim -O a.txt b.txt          # 左右分屏对比
vim -p a.txt b.txt c.txt    # 标签页打开
vim -R file.txt             # 只读打开
vim -u NONE file.txt        # 不加载任何配置
vim --clean file.txt        # 干净启动（排查插件问题）
vim -r file.txt             # 恢复崩溃会话
vim -c 'set nu' file.txt    # 打开后执行命令
```

### 易踩坑

- 新手最常卡住：进入后不知道按 `Esc` 回普通模式，或不知道 `:q!` 退出。
- `:wq` 会写盘覆盖原文件；只读文件需 `:wq!`。
- `-n` 不建交换文件，崩溃时**无法恢复**。
- `-R` 是只读，但可 `:w!` 强写；`-m`/`-M` 更严格。

**BSD/macOS**：macOS 自带的是老版 vim；`--clean`、`-p` 等较新选项可能缺失。

---

## 15. nano — 轻量编辑器

**语法**：`nano [OPTION]... [FILE]`

界面底部直接列出快捷键，对新手最友好。`^` 表示 Ctrl，`M-` 表示 Alt。

### 常用按键

| 按键 | 作用 |
|---|---|
| `Ctrl-O` | 保存（Write Out） |
| `Ctrl-X` | 退出 |
| `Ctrl-W` | 搜索 |
| `Ctrl-K` | 剪切整行 |
| `Ctrl-U` | 粘贴 |
| `Ctrl-\` | 替换 |
| `Ctrl-G` | 帮助 |
| `Ctrl-C` | 显示光标位置 |
| `Ctrl-_` | 跳转到指定行 |

### 选项（手册共 59 项）

| 选项 | 含义 |
|---|---|
| `-A, --smarthome` | Home 键更智能（跳到首个非空白或行首） |
| `-B, --backup` | 保存时把上一版备份为 `文件名~` |
| `-C dir, --backupdir=dir` | 备份到指定目录并唯一编号（需 `-B`） |
| `-D, --boldtext` | 界面用粗体代替反显 |
| `-E, --tabstospaces` | 把输入的 TAB 转为空格 |
| `-F, --newbuffer` | 默认读入新缓冲 |
| `-G, --locking` | 使用 vim 风格文件锁 |
| `-H, --historylog` | 记录最近 100 条搜索/替换/命令 |
| `-I, --ignorercfiles` | 不读系统与用户的 nanorc |
| `-J number, --guidestripe=number` | 在指定列画竖线辅助判断宽度 |
| `-K, --rawsequences` | 直接解释转义序列（会禁用鼠标） |
| `-L, --nonewlines` | 文件末尾无换行时不自动补换行 |
| `-M, --trimblanks` | 折行时去掉行尾空白 |
| `-N, --noconvert` | 禁用 DOS 格式自动转换 |
| `-O, --bookstyle` | 对齐时把以空白开头的行视为段落开头 |
| `-P, --positionlog` | 记录最近 200 个文件的光标位置 |
| `-Q "regex", --quotestr="regex"` | 设置引用前缀的正则 |
| `-R, --restricted` | 受限模式（只读写命令行指定的文件） |
| `-S, --softwrap` | 超宽行在屏幕上软折行显示 |
| `-T number, --tabsize=number` | TAB 宽度（默认 8） |
| `-U, --quickblank` | 状态栏消息按键一次即消失 |
| `-V, --version` | 显示版本 |
| `-W, --wordbounds` | 把标点也算作单词的一部分 |
| `-X "chars", --wordchars="chars"` | 指定哪些额外字符算单词一部分 |
| `-Y name, --syntax=name` | 指定语法高亮 |
| `-Z, --zap` | 未修改的 Backspace/Delete 删除标记区域 |
| `-a, --atblanks` | 软折行时在空白处断行 |
| `-b, --breaklonglines` | 自动硬折行（与 `-w` 相反） |
| `-c, --constantshow` | 状态栏常显光标位置 |
| `-d, --rebinddelete` | 修正 Delete/Backspace 行为 |
| `-e, --emptyline` | 标题栏下留空一整行 |
| `-f file, --rcfile=file` | 只读该文件作为配置 |
| `-g, --showcursor` | 文件浏览器中显示光标 |
| `-h, --help` | 显示帮助 |
| `-i, --autoindent` | 新行自动缩进对齐上一行 |
| `-j, --jumpyscrolling` | 每次滚动半屏而非一行 |
| `-k, --cutfromcursor` | 剪切从光标到行尾（而非整行） |
| `-l, --linenumbers` | 显示行号 |
| `-m, --mouse` | 启用鼠标 |
| `-n, --noread` | 把命令行给出的名字当新文件（可写命名管道） |
| `-o dir, --operatingdir=dir` | 限制只在该目录内读写 |
| `-p, --preserve` | 保留 `^S`/`^Q` 给终端 |
| `-q, --indicator` | 右侧显示“滚动条” |
| `-r number, --fill=number` | 对齐/硬折行的目标宽度（默认 -8） |
| `-s "prog [args]", --speller="prog [args]"` | 指定拼写检查程序 |
| `-t, --saveonexit` | 退出时直接保存不提示 |
| `-u, --unix` | 默认按 Unix 格式保存 |
| `-v, --view` | 只读查看模式 |
| `-w, --nowrap` | 不自动硬折行（默认，与 `-b` 相反） |
| `-x, --nohelp` | 不显示底部两行帮助 |
| `-y, --afterends` | `Ctrl-Right`/`Ctrl-Delete` 停在词尾 |
| `-z, --listsyntaxes` | 列出可用语法并退出 |
| `-!, --magic` | 用 libmagic 判断语法 |
| `-@, --colonparsing` | 解析 `文件名:行号` 形式 |
| `-%, --stateflags` | 右上角显示状态标志 |
| `-_, --minibar` | 用迷你状态栏代替标题栏 |
| `-0, --zero` | 隐藏所有界面元素，全屏显示内容 |
| `-1, --solosidescroll` | 只横向滚动当前行（旧行为） |
| `-/, --modernbindings` | 使用现代快捷键（`^X` 剪切、`^C` 复制、`^V` 粘贴等） |

### 示例

```bash
nano file.txt                  # 打开文件
nano -l file.txt               # 显示行号
nano -w file.txt               # 不硬折行
nano -m file.txt               # 启用鼠标
nano -v file.txt               # 只读查看
nano -R file.txt               # 受限模式
nano -u -w file.txt            # Unix 格式 + 不折行（改配置文件常用）
nano +42 file.txt              # 打开并跳到第 42 行
nano -Y json file.json         # 指定语法高亮
nano -/ file.txt               # 现代快捷键
```

### 易踩坑

- `Ctrl-X` 退出时若文件已改会提示保存，注意看清 `Y`/`N`。
- `-w` 与 `-b` 是互斥开关，后出现的生效。
- `nano` 的 `^` 是 Ctrl 而非 Shift+6；`M-` 是 Alt。

**BSD/macOS**：macOS 的 nano 版本较旧，`--minibar`、`--zero`、`-1`、`-/` 等新选项可能缺失。

---

## 附：管道组合速查

| 需求 | 命令 |
|---|---|
| 去重并统计次数 | `sort f \| uniq -c \| sort -rn` |
| 只看某几列 | `awk '{print $1, $3}' f` 或 `cut -d, -f1,3 f` |
| 过滤 + 取列 | `grep ERROR app.log \| awk '{print $NF}'` |
| 统计代码行数 | `find . -name '*.ts' \| xargs wc -l` |
| 实时看错误日志 | `tail -F app.log \| grep --color=always -i error \| less -R` |
| 批量替换 | `grep -rl old . \| xargs sed -i 's/old/new/g'` |
| 两文件差异摘要 | `diff -q a b` / `diff -y --suppress-common-lines a b` |
| 生成目录补丁 | `diff -ruN dir1 dir2 > x.patch` |
| 删除空行 | `sed '/^$/d' f` 或 `awk 'NF' f` |
| 提取数字 | `grep -oE '[0-9]+' f` |
| CSV 某列求和 | `awk -F, '{s+=$3} END {print s}' data.csv` |
| 按次数倒序 TOP N | `sort f \| uniq -c \| sort -rn \| head -n 10` |

---

## 危险操作清单

以下操作会**不可逆地修改或删除数据**，执行前务必确认：

| 操作 | 风险 |
|---|---|
| `sed -i 's/a/b/g' file` | 直接改写原文件，无备份 |
| `sed -i` 在 macOS | 语法不同，易误把下个参数当后缀 |
| `vim` / `nano` 保存 | 覆盖原文件 |
| `sort file > file` | 文件被清空 |
| `cat file > file` | 文件被清空 |
| `> file`（重定向到源文件） | 文件被清空 |
| `xargs sed -i` 批量替换 | 一次改一批文件 |

**建议**：`sed -i.bak` 保留备份；改前先 `diff` 或 `cp file file.bak`。
