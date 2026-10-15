# Linux 网络与远程命令参考

> 覆盖 `ip / ping / curl / wget / ss / netstat / tcpdump / nc / telnet / dig-nslookup / traceroute / scp / rsync / ssh` 共 **14 条命令**（`dig`/`nslookup` 合并为一节）。
>
> **来源**：iproute2 6.x（ip/ss）、iputils（ping/traceroute）、curl 8.x（curl 手册共 267 项）、GNU Wget 1.21+（手册共 204 项）、net-tools 2.x（netstat）、tcpdump 4.99/libpcap 1.10、OpenBSD netcat（nc）、GNU inetutils 2.8（telnet）、BIND 9.x（dig/nslookup）、OpenSSH 9.x（ssh/scp）、rsync 3.2+（手册共 337 项）。
>
> **BSD/macOS 差异在每节末尾单列**，因为本机是 macOS、而实际运维目标通常是 Linux。macOS 差异最大的三条是 `ip`（根本没有）、`ss`（没有，用 `netstat`/`lsof`）、`rsync`（系统自带版本是 2.6.9，非常旧）。
>
> 不确定的地方一律查了手册，未凭记忆推断。

## 速查表

| 命令 | 作用 | 危险级别 | 本参考中的参数模板 |
|---|---|---|---|
| `ip` | 网络配置（替代 ifconfig/route） | **confirm** | `ip {{object}} {{command}}` |
| `ping` | 连通性测试 | safe | `ping {{options}} {{host}}` |
| `curl` | HTTP/多协议请求 | safe | `curl {{options}} {{url}}` |
| `wget` | 下载 / 递归镜像 | safe | `wget {{options}} {{url}}` |
| `ss` | 查看端口/连接（替代 netstat） | safe | `ss {{options}}` |
| `netstat` | 网络状态（旧，已废弃） | safe | `netstat {{options}}` |
| `tcpdump` | 抓包 | safe | `tcpdump {{options}} {{filter}}` |
| `nc` | 网络调试 / 端口扫描 / 传数据 | **confirm** | `nc {{options}} {{host}} {{port}}` |
| `telnet` | 端口测试（明文） | safe | `telnet {{options}} {{host}} {{port}}` |
| `dig` / `nslookup` | DNS 查询 | safe | `dig {{options}} {{name}} {{type}}` |
| `traceroute` | 路由追踪 | safe | `traceroute {{options}} {{host}}` |
| `scp` | 远程复制 | **confirm** | `scp {{options}} {{src}} {{dst}}` |
| `rsync` | 增量同步 | **confirm** | `rsync {{options}} {{src}} {{dst}}` |
| `ssh` | 远程登录 / 隧道 | safe | `ssh {{options}} {{user}}@{{host}}` |

> `ip` 是 **confirm**：`ip addr flush`、`ip link set down`、`ip route del default` 都会当场切断网络，远程操作时极易把自己踢下线（俗称「把梯子踢了」）。
> `nc` 是 **confirm**：可以开监听端口、做端口扫描，属于主动探测行为。
> `scp` / `rsync` 是 **confirm**：会覆盖/删除目标端文件。**`rsync --delete` 是真正的 destructive**，见该节。
> `ssh` 保持 safe，但它承载隧道（`-L`/`-R`/`-D`）与端口转发，误用会暴露内网服务。

---

## 1. ip — 网络配置（替代 ifconfig / route / arp）

`ip` 是 iproute2 套件的核心，用 **`ip OBJECT COMMAND`** 的对象-命令模型统一了 `ifconfig`（地址/链路）、`route`（路由）、`arp`（邻居）三套旧工具。

### 基本语法

```
ip [ OPTIONS ] OBJECT { COMMAND | help }
```

常用 OBJECT：

| OBJECT | 作用 | 对应旧命令 |
|---|---|---|
| `addr`（`a`） | 接口 IP 地址 | `ifconfig` / `ip addr` |
| `link`（`l`） | 链路层（网卡）状态 | `ifconfig -a` / `ethtool` 部分 |
| `route`（`r`） | 路由表 | `route -n` |
| `neigh`（`n`） | ARP / NDISC 邻居缓存 | `arp -n` |
| `rule` | 策略路由规则 | — |
| `netns` | 网络命名空间 | — |
| `tunnel` / `tuntap` | 隧道 / TUN-TAP 设备 | — |
| `maddr` | 组播地址 | `netstat -g` |
| `-s link` | 接口收发统计 | `netstat -i` |

### 全局选项（手册共 27 项，全列）

| 选项 | 说明 |
|---|---|
| `-V, -Version` | 打印 ip 版本并退出 |
| `-h, -human, -human-readable` | 统计值输出人类可读格式（带单位后缀） |
| `-b, -batch <FILE>` | 从文件或 stdin 读取命令批量执行；首个失败即终止 |
| `-force` | 批处理模式遇错不终止（最终返回非 0 退出码） |
| `-s, -stats, -statistics` | 输出更多信息；可重复多次，信息量递增 |
| `-d, -details` | 输出更详细的信息 |
| `-l, -loops <COUNT>` | `ip address flush` 最多循环次数，默认 10；`0` 表示直到删光所有地址 |
| `-f, -family <FAMILY>` | 指定协议族：`inet`/`inet6`/`bridge`/`mpls`/`link` |
| `-4` | `-family inet` 的快捷方式 |
| `-6` | `-family inet6` 的快捷方式 |
| `-B` | `-family bridge` 的快捷方式 |
| `-M` | `-family mpls` 的快捷方式 |
| `-0` | `-family link` 的快捷方式 |
| `-o, -oneline` | 每条记录压成一行（换行替换为 `\`），便于 `wc`/`grep` |
| `-r, -resolve` | 用系统解析器把地址解析成域名 |
| `-n, -netns <NETNS>` | 切换到指定网络命名空间执行 |
| `-N, -Numeric` | 直接打印协议/scope/dsfield 编号，而非转成可读名 |
| `-a, -all` | 对所有对象执行（取决于命令是否支持） |
| `-c[color][={always\|auto\|never}]` | 彩色输出；`-json` 时忽略此选项 |
| `-t, -timestamp` | 使用 `monitor` 时显示当前时间 |
| `-ts, -tshort` | 同 `-timestamp`，但用短格式 |
| `-rc, -rcvbuf <SIZE>` | 设置 netlink socket 接收缓冲区，默认 1MB |
| `-iec` | 人类可读速率用 IEC 单位（1Ki = 1024） |
| `-br, -brief` | 仅打印表格化基本信息（仅 `ip addr show`/`ip link show`/`ip neigh show` 支持） |
| `-j, -json` | 以 JSON 输出结果 |
| `-p, -pretty` | JSON 加缩进，便于阅读 |
| `-echo` | 请求内核回送刚下发的配置 |

### 常用子命令速查

```bash
ip addr show                     # 列出所有接口地址（= ip a）
ip -4 addr show eth0             # 只看 eth0 的 IPv4
ip -br addr                      # 精简表格：接口/状态/地址
ip addr add 192.168.1.10/24 dev eth0     # 加地址
ip addr del 192.168.1.10/24 dev eth0     # 删地址
ip link set eth0 up              # 启用网卡
ip link set eth0 down            # 禁用网卡
ip link set eth0 mtu 1400        # 改 MTU
ip link set eth0 promisc on      # 开混杂模式
ip route show                    # 查看路由表
ip route add default via 192.168.1.1     # 加默认路由
ip route del default             # 删默认路由
ip route get 8.8.8.8             # 查某个目标走哪条路由
ip neigh show                    # 查看 ARP 表（= ip n）
ip -s link show eth0             # 接口收发统计
ip netns list                    # 列出网络命名空间
ip netns exec myns ip addr       # 在命名空间内执行命令
ip monitor route                 # 实时监视路由变化
```

### 示例

```bash
# 只看 IPv4 地址，精简表格
$ ip -br -4 addr
lo               UNKNOWN        127.0.0.1/8
eth0             UP             192.168.1.10/24

# 查 8.8.8.8 的实际出口
$ ip route get 8.8.8.8
8.8.8.8 via 192.168.1.1 dev eth0 src 192.168.1.10 uid 501

# JSON 输出（便于脚本解析）
$ ip -j -p addr show dev eth0
```

### 易踩坑

- **`ip addr flush dev eth0` 会清空该网卡所有地址**，远程 SSH 执行等于自杀式断网。要恢复只能去机房/控制台。
- `ip link set eth0 down` 同理，比 flush 更彻底。
- `ip route del default` 删掉默认路由后，只有同网段可达，外网全断。
- `ifconfig` 里的 `netmask`、`broadcast` 在 `ip` 里写成 `192.168.1.10/24`、`192.168.1.255`，语法不同。
- `ip -s link` 的统计**不包含** `ip addr` 的地址信息，两者要分开看。
- 修改默认**不会持久化**，重启即失效。持久化要写发行版自己的配置（`/etc/network/interfaces`、`netplan`、`NetworkManager`、`systemd-networkd`）。

### macOS 差异

**macOS 没有 `ip` 命令**。对应工具：

| Linux `ip` | macOS 对应 |
|---|---|
| `ip addr` | `ifconfig` |
| `ip link` | `ifconfig` / `networksetup` |
| `ip route` | `netstat -rn` / `route` |
| `ip neigh` | `arp -an` |

安装 iproute2mac（`brew install iproute2mac`）可提供 `ip` 的一个子集兼容层。

---

## 2. ping — 连通性测试

基于 ICMP ECHO_REQUEST / ECHO_REPLY 测试主机可达性与往返时延。

### 基本语法

```
ping [options] destination
```

### 选项（手册共 40 项，全列）

| 选项 | 说明 |
|---|---|
| `-3` | RTT 精度：不把结果时间向上取整 |
| `-4` | 只用 IPv4 |
| `-6` | 只用 IPv6 |
| `-a` | 收到应答时发声（audible ping） |
| `-A` | 自适应 ping：发包间隔随 RTT 自适应，默认间隔 2ms |
| `-b` | 允许 ping 广播地址 |
| `-B` | 不允许 ping 中途改源地址（启动时绑定） |
| `-c count` | 发 `count` 个包后停止 |
| `-C` | 创建 socket 时调用 `connect()` |
| `-d` | 在 socket 上设 `SO_DEBUG`（Linux 内核实际未使用） |
| `-D` | 每行前打印时间戳（unix 时间 + 微秒） |
| `-e identifier` | 设置 ECHO_REQUEST 的 identification 字段 |
| `-f` | 洪水 ping，需 root（间隔为 0） |
| `-F flow label` | IPv6：设置 20 位 flow label（十六进制） |
| `-h` | 显示帮助 |
| `-H` | 强制对输出做 DNS 反解（覆盖 `-n`） |
| `-i interval` | 发包间隔秒数，默认 1；`< 2ms` 需 root |
| `-I interface` | 指定源地址/接口名/VRF |
| `-j` | JSON 输出（实验性） |
| `-l preload` | 不等回复连发 `preload` 个包；`> 3` 需 root |
| `-L` | 抑制组播回环 |
| `-m mark` | 给包打 mark（策略路由用），需 `CAP_NET_ADMIN` |
| `-M pmtudisc_opt` | PMTU 发现策略：`do`/`want`/`probe`/`dont` |
| `-N nodeinfo_option` | IPv6：发节点信息查询（RFC4620） |
| `-n` | 只输出数字，不做反向 DNS |
| `-O` | 发下一个包前报告未应答的 ECHO_REPLY |
| `-p pattern` | 最多 16 字节填充内容，诊断数据相关问题 |
| `-q` | 安静模式，只显示启动与结束摘要 |
| `-Q tos` | 设置 ICMP 的 QoS/TOS 位 |
| `-r` | 绕过路由表，直接发给直连接口上的主机 |
| `-R` | 记录路由（IPv4，最多 9 跳，很多主机忽略） |
| `-s packetsize` | 数据字节数，默认 56（加 8 字节 ICMP 头 = 64） |
| `-S sndbuf` | 设置 socket 发送缓冲区 |
| `-t ttl` | 设置 IP TTL |
| `-T timestamp option` | IP 时间戳选项：`tsonly`/`tsandaddr`/`tsprespec` |
| `-U` | 打印完整用户到用户时延（旧行为） |
| `-v` | 详细输出 |
| `-V` | 显示版本并退出 |
| `-w deadline` | 总超时秒数（无论收没收到都退出） |
| `-W timeout` | 等待应答的超时秒数；`0` 表示无限等待 |

### 示例

```bash
ping -c 4 baidu.com              # 发 4 个包
ping -c 4 -i 0.2 10.0.0.1        # 200ms 间隔（需 root）
ping -c 3 -s 1472 10.0.0.1       # 测 MTU（1472 + 28 = 1500）
ping -c 3 -M do -s 1472 host     # 禁止分片，探 PMTU
ping -c 5 -W 1 -w 10 host        # 单包 1s、总 10s 超时
ping -f -c 10000 host            # 洪水测试（需 root）
ping -6 -I eth0 fe80::1%eth0     # IPv6 指定接口
```

### 易踩坑

- **`-c` 与 `-w` 同时用时**：`-c` 是发包数，`-w` 是总时限，两者哪个先到算哪个。
- `-i` 小于 2ms 需要 root；广播/组播对普通用户最小间隔是 1 秒。
- `-s` 最大值：IPv4 为 65507，IPv6 为 65527，但多数系统限制更小。
- **ping 通不代表端口通**，很多防火墙只放行 ICMP。反过来 ping 不通也不代表服务挂了（ICMP 常被禁）。
- `-f`（洪水）会瞬间打满带宽，是常见的「压垮网络」误操作，且需 root。
- `-R` 记录路由在现代网络基本被丢弃，别指望它。

### macOS 差异

macOS 的 `ping` 选项子集更小：**没有 `-3`、`-C`、`-e`、`-H`、`-j`、`-M`、`-m`、`-N`、`-O`、`-Q`、`-S`、`-T`、`-U`**。
`-W` 在 macOS 上单位是**毫秒**，而 Linux 是**秒** —— 这是最容易踩的差异（`ping -W 1000` 在 Linux 上要等 1000 秒）。
`-w`（总时限）macOS 也不支持。

---

## 3. curl — HTTP / 多协议请求

curl 是「用 URL 语法说话」的传输工具，支持 HTTP/HTTPS、FTP、SFTP、SCP、SMTP、IMAP、LDAP、TFTP、MQTT、TELNET 等。手册共 **267 个选项**，下面按用途分组列出主要项。

### 基本语法

```
curl [options...] <url>
```

### 选项分组

#### 输出控制

| 选项 | 说明 |
|---|---|
| `-o, --output <file>` | 写到文件而非 stdout |
| `-O, --remote-name` | 用远端文件名保存到当前目录 |
| `-J, --remote-header-name` | 配合 `-O`，用 `Content-Disposition` 里的文件名 |
| `--remote-name-all` | 所有 URL 都按 `-O` 处理 |
| `--output-dir <dir>` | 指定输出目录 |
| `--create-dirs` | 配合 `-o` 自动创建目录层级 |
| `--no-clobber` | 不覆盖已存在文件，改用 `file.1`、`file.2`… |
| `-R, --remote-time` | 用远端文件的时间戳 |
| `--remove-on-error` | 出错时删除已写出的部分文件 |
| `--xattr` | 把 URL 写入扩展属性 `xdg.origin.url` |
| `-D, --dump-header <file>` | 把响应头写到文件 |
| `-i, --include` | 输出中包含响应头 |
| `-I, --head` | 只取头部（HTTP HEAD） |
| `-s, --silent` | 静默（不显示进度与错误） |
| `-S, --show-error` | 配合 `-s`，出错时仍显示错误 |
| `--no-progress-meter` | 关进度条但保留告警信息 |
| `-#, --progress-bar` | 用简单进度条代替详细进度表 |
| `-w, --write-out <format>` | 传输结束后按格式输出变量（如 `%{http_code}`） |
| `-v, --verbose` | 详细调试输出 |
| `--trace <file>` | 完整追踪（含十六进制） |
| `--trace-ascii <file>` | 只追踪 ASCII 部分 |
| `--trace-time` / `--trace-ids` | 追踪行加时间戳 / 加传输与连接 ID |
| `-N, --no-buffer` | 禁用输出缓冲 |
| `--stderr <file>` | 把 stderr 重定向到文件 |
| `-M, --manual` | 显示完整帮助 |
| `-h, --help <category>` | 分类帮助 |
| `-V, --version` | 显示版本 |

#### HTTP 方法与数据

| 选项 | 说明 |
|---|---|
| `-d, --data <data>` | POST 表单数据（`application/x-www-form-urlencoded`） |
| `--data-ascii <data>` | `-d` 的别名 |
| `--data-binary <data>` | 原样 POST，保留换行 |
| `--data-raw <data>` | 同 `-d` 但不解析 `@` |
| `--data-urlencode <data>` | POST 前做 URL 编码 |
| `--json <data>` | 快捷设置 `-d` + JSON 两个头 |
| `-F, --form <name=content>` | `multipart/form-data` 表单（可传文件 `@file`） |
| `--form-string <name=string>` | 同 `-F`，但不解析 `@`/`<` |
| `--form-escape` | 表单字段名用反斜杠转义而非百分号编码 |
| `-G, --get` | 把 `-d` 数据改成 GET 查询串 |
| `-X, --request <method>` | 指定 HTTP 方法 |
| `--request-target <path>` | 用替代的 request target |
| `-T, --upload-file <file>` | 上传文件 |
| `--url-query <data>` | 往 URL query 追加数据 |
| `-e, --referer <URL>` | 设置 Referer（`;auto` 自动跟随） |
| `-A, --user-agent <name>` | 设置 User-Agent |
| `-H, --header <header>` | 追加/替换请求头（可多次） |
| `-b, --cookie <data\|file>` | 发送 Cookie |
| `-c, --cookie-jar <file>` | 保存 Cookie 到 Netscape 格式文件 |
| `-j, --junk-session-cookies` | 丢弃 session cookie |
| `-L, --location` | 跟随重定向 |
| `--location-trusted` | 跟随重定向且向新主机发送凭据 |
| `--max-redirs <num>` | 最大重定向次数（默认 50，`-1` 无限） |
| `--post301` / `--post302` / `--post303` | 重定向后保持 POST 而非转 GET |
| `-r, --range <range>` | 请求字节范围（断点/分段下载） |
| `-z, --time-cond <time>` | 按时间条件请求（只下更新的） |
| `--etag-save <file>` / `--etag-compare <file>` | ETag 缓存 |
| `--compressed` | 请求并自动解压 |
| `--tr-encoding` | 请求压缩 Transfer-Encoding |
| `--raw` | 禁用 HTTP 内容解码 |
| `--ignore-content-length` | 忽略 Content-Length |
| `--http0.9` / `-0, --http1.0` / `--http1.1` / `--http2` / `--http2-prior-knowledge` / `--http3` / `--http3-only` | 指定 HTTP 版本 |
| `--url <url>` | 在配置文件里指定 URL |
| `-:, --next` | 分隔多次操作（各自独立选项） |
| `-Z, --parallel` | 并行传输 |
| `--parallel-max <num>` | 并行上限（默认 50） |
| `--parallel-immediate` | 并行时优先多开连接 |
| `--rate <rate>` | 限制传输启动频率 |

#### 认证

| 选项 | 说明 |
|---|---|
| `-u, --user <user:password>` | 服务器认证用户名密码 |
| `--basic` / `--digest` / `--ntlm` / `--ntlm-wb` / `--negotiate` / `--anyauth` | 认证方式 |
| `-n, --netrc` | 从 `~/.netrc` 读凭据 |
| `--netrc-file <file>` / `--netrc-optional` | 指定 netrc 文件 / 可选 netrc |
| `--oauth2-bearer <token>` | OAuth 2.0 Bearer Token |
| `--aws-sigv4 <provider>` | AWS V4 签名认证 |
| `--delegation <LEVEL>` | GSS/Kerberos 委派级别 |
| `--krb <level>` | Kerberos 认证 |
| `--service-name <name>` | SPNEGO 服务名 |
| `-U, --proxy-user <user:password>` | 代理认证 |
| `--proxy-basic` / `--proxy-digest` / `--proxy-ntlm` / `--proxy-negotiate` / `--proxy-anyauth` | 代理认证方式 |
| `--proxy-service-name <name>` | 代理 SPNEGO 服务名 |
| `--sasl-authzid <identity>` / `--sasl-ir` | SASL 授权身份 / 初始响应 |
| `--tlsuser <name>` / `--tlspassword <str>` / `--tlsauthtype <type>` | TLS-SRP 认证 |

#### 连接 / 超时 / 重试

| 选项 | 说明 |
|---|---|
| `--connect-timeout <sec>` | 连接阶段超时 |
| `-m, --max-time <sec>` | 单次传输总超时 |
| `--keepalive-time <sec>` | TCP keepalive 间隔 |
| `--no-keepalive` | 禁用 keepalive |
| `-Y, --speed-limit <speed>` | 低于此速率（配合 `-y`）则中止 |
| `-y, --speed-time <sec>` | 低速持续秒数 |
| `--limit-rate <speed>` | 限制传输速率 |
| `--max-filesize <bytes>` | 文件超过此大小则不下 |
| `--retry <num>` | 瞬时错误重试次数 |
| `--retry-delay <sec>` | 重试间隔 |
| `--retry-max-time <sec>` | 重试总时限 |
| `--retry-all-errors` | 任何错误都重试 |
| `--retry-connrefused` | 把连接被拒当瞬时错误 |
| `--happy-eyeballs-timeout-ms <ms>` | IPv6 先跑多少毫秒再并发 IPv4 |
| `--tcp-fastopen` / `--tcp-nodelay` | TCP Fast Open / 禁用 Nagle |
| `-4, --ipv4` / `-6, --ipv6` | 只用 IPv4 / IPv6 |
| `--interface <name>` | 指定出口接口 |
| `--local-port <num/range>` | 指定本地端口 |
| `--resolve <host:port:addr>` | 命令行版 `/etc/hosts` |
| `--connect-to <H1:P1:H2:P2>` | 把连接改到另一台 |
| `-C, --continue-at <offset>` | 断点续传（`-C -` 自动定位） |
| `--unix-socket <path>` / `--abstract-unix-socket <path>` | 走 Unix 域套接字 |
| `--path-as-is` | 不规整 URL 里的 `/../` |

#### 代理

| 选项 | 说明 |
|---|---|
| `-x, --proxy [proto://]host[:port]` | 指定代理 |
| `--preproxy [proto://]host[:port]` | 先连 SOCKS 再连 HTTP 代理 |
| `-p, --proxytunnel` | 通过代理建 CONNECT 隧道 |
| `--noproxy <list>` | 不走代理的主机列表 |
| `--proxy1.0 <host[:port]>` | 用 HTTP 1.0 代理 |
| `--socks4` / `--socks4a` / `--socks5` / `--socks5-hostname` | SOCKS 代理（`-hostname` 表示由代理解析） |
| `--socks5-basic` / `--socks5-gssapi` | SOCKS5 认证方式 |
| `--proxy-header <header>` | 只发给代理的请求头 |
| `--proxy-http2` | 与 HTTPS 代理协商 HTTP/2 |
| `--proxy-insecure` | 不校验代理证书 |
| `--proxy-cacert` / `--proxy-capath` / `--proxy-cert` / `--proxy-key` / `--proxy-pass` | 代理侧 TLS 证书相关 |
| `--proxy-ciphers` / `--proxy-tls13-ciphers` | 代理侧密码套件 |
| `--proxy-pinnedpubkey` / `--proxy-crlfile` | 代理侧公钥固定 / CRL |

#### TLS / SSL

| 选项 | 说明 |
|---|---|
| `-k, --insecure` | 跳过证书校验（危险） |
| `--cacert <file>` / `--capath <dir>` / `--ca-native` | CA 证书来源 |
| `-E, --cert <cert[:passwd]>` / `--cert-type <type>` | 客户端证书 |
| `--cert-status` | 用 OCSP stapling 校验证书状态 |
| `--key <key>` / `--key-type <type>` / `--pass <phrase>` | 私钥 |
| `--pubkey <key>` | SSH 公钥文件 |
| `--ciphers <list>` / `--tls13-ciphers <list>` / `--curves <list>` | 密码套件 / 曲线 |
| `--crlfile <file>` | 证书吊销列表 |
| `--pinnedpubkey <hashes>` | 固定公钥 |
| `--tls-max <VERSION>` | 最高 TLS 版本 |
| `--tlsv1.0` / `--tlsv1.1` / `--tlsv1.2` / `--tlsv1.3` / `-1, --tlsv1` | 最低 TLS 版本 |
| `--ssl-allow-beast` / `--ssl-no-revoke` / `--ssl-revoke-best-effort` | 放宽 SSL 校验 |
| `--false-start` | TLS false start |
| `--no-alpn` / `--no-npn` / `--no-sessionid` | 关闭 ALPN / NPN / 会话 ID 缓存 |
| `--engine <name>` | OpenSSL 引擎 |
| `--hostpubmd5` / `--hostpubsha256` | 校验远端主机公钥（SFTP/SCP） |

#### DNS / DoH

| 选项 | 说明 |
|---|---|
| `--dns-interface <iface>` | DNS 请求走指定接口 |
| `--dns-ipv4-addr <addr>` / `--dns-ipv6-addr <addr>` | DNS 请求绑定源地址 |
| `--dns-servers <addrs>` | 指定 DNS 服务器 |
| `--doh-url <URL>` | 用 DNS-over-HTTPS 解析 |
| `--doh-insecure` / `--doh-cert-status` | DoH 的证书控制 |

#### 协议 / FTP / 其他

| 选项 | 说明 |
|---|---|
| `--proto <protocols>` | 限制可用协议 |
| `--proto-default <protocol>` | URL 无 scheme 时的默认协议 |
| `--proto-redir <protocols>` | 重定向时允许的协议 |
| `--ftp-account` / `--ftp-alternative-to-user` | FTP 账户信息 |
| `--ftp-create-dirs` / `--ftp-method` / `--ftp-pasv` / `-P, --ftp-port` | FTP 目录与模式 |
| `--ftp-pret` / `--ftp-skip-pasv-ip` | FTP 兼容性开关 |
| `--ftp-ssl-ccc` / `--ftp-ssl-ccc-mode` / `--ftp-ssl-control` | FTP over SSL |
| `-Q, --quote <command>` | 向 FTP/SFTP 发任意命令 |
| `-l, --list-only` | 只列目录（FTP） |
| `-B, --use-ascii` | ASCII 传输模式 |
| `-a, --append` | FTP/SFTP 上传时追加 |
| `--create-file-mode <mode>` | 远端新建文件权限 |
| `--compressed-ssh` | 启用 SSH 压缩 |
| `--tftp-blksize` / `--tftp-no-options` | TFTP 选项 |
| `-t, --telnet-option <opt=val>` | TELNET 选项 |
| `-K, --config <file>` | 从配置文件读参数 |
| `-q, --disable` | 作为首个参数时忽略 curlrc |
| `-g, --globoff` | 关闭 URL 通配解析 |
| `--variable` / `--expand-url` | 变量替换 |
| `--libcurl <file>` | 生成等效的 libcurl C 代码 |
| `--hsts <file>` / `--alt-svc <file>` | HSTS / Alt-Svc 缓存 |
| `--expect100-timeout <sec>` | 等 100-continue 的时限 |
| `--haproxy-protocol` / `--haproxy-clientip` | HAProxy PROXY 协议头 |
| `--disallow-username-in-url` | URL 含用户名则退出 |
| `--suppress-connect-headers` | 不输出代理 CONNECT 响应头 |

### 示例

```bash
curl -I https://example.com                     # 只看响应头
curl -s -o /dev/null -w '%{http_code}\n' url    # 只取状态码
curl -L -o file.zip https://example.com/f.zip   # 跟随重定向下载
curl -C - -O https://example.com/big.iso        # 断点续传
curl -X POST -H 'Content-Type: application/json' \
     -d '{"a":1}' https://api.example.com/x     # JSON POST
curl --json '{"a":1}' https://api.example.com/x # 等价简写
curl -F 'file=@a.png' -F 'name=bob' url         # 表单上传
curl -u user:pass https://example.com           # Basic 认证
curl -x socks5h://127.0.0.1:1080 https://x.com  # SOCKS5 代理
curl --resolve example.com:443:1.2.3.4 https://example.com  # 强制解析
curl -s --max-time 5 --retry 3 --retry-connrefused url
curl -w '\n%{http_code} %{time_total}s\n' -o /dev/null -s url
```

### 易踩坑

- **`-I` 发的是 HEAD 请求**，有些服务对 HEAD 返回 405，想验证 GET 可读性要用 `-o /dev/null -D -`。
- **`-X POST` 不会自动带 body**，还需要 `-d`。而 `-d` 本身已经隐含 POST，再写 `-X POST` 属多余。
- `-d @file` 会**读取文件内容**并做换行处理；要原样上传二进制用 `--data-binary @file`。
- `-k` 跳过证书校验，生产脚本里出现 `-k` 通常意味着有更该修的根因。
- **`--max-time` 是总时限，`--connect-timeout` 只是连接阶段**；慢速大文件要用 `-m` 调大。
- `-L` 跟随时**默认不向新主机发凭据**（防泄漏），确需用 `--location-trusted`，但这会把凭据发给任意重定向目标。
- `-o` 与 `-O` 混用时，`-o` 只影响紧跟其后的那个 URL。
- curl 退出码即错误码（如 6=解析失败、7=连接失败、22=HTTP 错误配合 `-f`、28=超时），脚本里应判 `$?`。

### macOS 差异

macOS 自带的是 **Apple 定制版 curl**，缺少部分特性（如 HTTP/3 依赖、某些 TLS 后端差异）。`--compressed`、`-k`、`-L` 等核心选项一致。若需完整特性建议 `brew install curl` 并用 `/opt/homebrew/opt/curl/bin/curl`。

---

## 4. wget — 下载 / 递归镜像

GNU Wget 是「非交互式、擅长递归抓取与镜像」的下载器。手册共 **204 个选项**。

### 基本语法

```
wget [option]... [URL]...
```

### 选项分组

#### 启动 / 日志 / 输入

| 选项 | 说明 |
|---|---|
| `-V, --version` | 显示版本 |
| `-h, --help` | 显示帮助 |
| `-b, --background` | 启动后立即转后台 |
| `-e, --execute <command>` | 执行一条 `.wgetrc` 命令 |
| `-o, --output-file=<log>` | 日志写到文件 |
| `-a, --append-output=<log>` | 日志追加到文件 |
| `-d, --debug` | 调试输出 |
| `-q, --quiet` | 关闭所有输出 |
| `-v, --verbose` | 详细输出（默认） |
| `-nv, --no-verbose` | 关详细但不完全静默 |
| `--report-speed=type` | 带宽按 type 输出（仅 `bits`） |
| `-i, --input-file=<file>` | 从文件读 URL（`-` 为 stdin） |
| `-F, --force-html` | 输入按 HTML 解析 |
| `-B, --base=<URL>` | 解析相对链接的基准 URL |
| `--config=<FILE>` | 指定启动配置文件 |
| `--rejected-log=<log>` | 记录被拒 URL |
| `--input-metalink=<file>` | 从本地 Metalink 文件下载 |
| `--metalink-over-http` / `--metalink-index` / `--preferred-location` | Metalink over HTTP |
| `--keep-badhash` | 保留哈希不匹配的 Metalink 文件 |
| `--xattr` | 用扩展属性保存原始 URL |

#### 下载控制

| 选项 | 说明 |
|---|---|
| `--bind-address=ADDRESS` | 客户端绑定源地址 |
| `--bind-dns-address=ADDRESS` | DNS 请求绑定地址（libcares） |
| `--dns-servers=ADDRESSES` | 指定 DNS 服务器（libcares） |
| `-t, --tries=number` | 重试次数（默认 20，`0`/`inf` 无限） |
| `-O, --output-document=file` | 所有文档合并写到 file（`-` 为 stdout） |
| `-nc, --no-clobber` | 不覆盖已存在文件 |
| `--backups=backups` | 覆盖前备份为 `.1`、`.2`… |
| `--no-netrc` | 不读 `.netrc` |
| `-c, --continue` | 断点续传 |
| `--start-pos=OFFSET` | 从指定偏移开始下载 |
| `--progress=type` | 进度显示类型：`dot` 或 `bar` |
| `--show-progress` | 任何 verbosity 下都显示进度条 |
| `-N, --timestamping` | 时间戳模式（只下更新的） |
| `--no-if-modified-since` | 不发送 If-Modified-Since |
| `--no-use-server-timestamps` | 不用服务器时间戳 |
| `-S, --server-response` | 打印服务器响应头 |
| `--spider` | 只检查不下载 |
| `-T, --timeout=seconds` | 总超时（同时设置下面三个） |
| `--dns-timeout=seconds` | DNS 查询超时 |
| `--connect-timeout=seconds` | 连接超时 |
| `--read-timeout=seconds` | 读超时（空闲时间） |
| `--limit-rate=amount` | 限速 |
| `-w, --wait=seconds` | 两次抓取之间等待 |
| `--waitretry=seconds` | 重试之间的线性退避上限 |
| `--random-wait` | 等待时间在 0.5~1.5 倍间随机 |
| `--no-proxy` | 不使用代理 |
| `-Q, --quota=quota` | 下载配额（单文件不受限） |
| `--no-dns-cache` | 关闭 DNS 缓存 |
| `--restrict-file-names=modes` | 本地文件名转义规则 |
| `-4, --inet4-only` / `-6, --inet6-only` | 只用 IPv4 / IPv6 |
| `--prefer-family=none/IPv4/IPv6` | 优先地址族 |
| `--retry-connrefused` | 把连接被拒当瞬时错误 |
| `--user=user` / `--password=password` | FTP+HTTP 通用凭据 |
| `--ask-password` | 每次连接询问密码 |
| `--use-askpass=command` | 用指定命令询问密码 |
| `--no-iri` / `--local-encoding` / `--remote-encoding` | 国际化 URI 支持 |
| `--unlink` | 覆盖前先 unlink（硬链接场景） |
| `--retry-on-host-error` / `--retry-on-http-error=code` | 特定错误重试 |

#### 目录 / 文件命名

| 选项 | 说明 |
|---|---|
| `-nd, --no-directories` | 递归时不建目录层级 |
| `-x, --force-directories` | 强制建目录层级 |
| `-nH, --no-host-directories` | 不生成主机名前缀目录 |
| `--protocol-directories` | 用协议名作目录组件 |
| `--cut-dirs=number` | 忽略前 N 级目录 |
| `-P, --directory-prefix=prefix` | 保存目录前缀 |
| `--default-page=name` | 默认页文件名（默认 index.html） |
| `-E, --adjust-extension` | 给非 `.html` 后缀的 HTML 加 `.html` |

#### HTTP 相关

| 选项 | 说明 |
|---|---|
| `--http-user` / `--http-password` | HTTP 凭据 |
| `--no-http-keep-alive` | 关闭 keep-alive |
| `--no-cache` | 禁用服务端缓存 |
| `--no-cookies` | 禁用 Cookie |
| `--load-cookies file` / `--save-cookies file` | 载入/保存 Cookie |
| `--keep-session-cookies` | 保存 session cookie |
| `--ignore-length` | 忽略错误的 Content-Length |
| `--header=header-line` | 追加请求头（可多次） |
| `--compression=type` | 压缩类型：`auto`/`gzip`/`none` |
| `--max-redirect=number` | 最大重定向数（默认 20） |
| `--proxy-user` / `--proxy-password` | 代理凭据 |
| `--referer=url` | 设置 Referer |
| `--save-headers` | 保存响应头到文件 |
| `-U, --user-agent=agent-string` | 设置 User-Agent |
| `--post-data=string` / `--post-file=file` | POST 数据 |
| `--method=HTTP-Method` | 指定 HTTP 方法 |
| `--body-data` / `--body-file` | 配合 `--method` 的请求体 |
| `--content-disposition` | 支持 Content-Disposition 文件名（实验性） |
| `--content-on-error` | 出错时仍保存内容 |
| `--trust-server-names` | 重定向后用重定向 URL 命名 |
| `--auth-no-challenge` | 主动发送 Basic 凭据 |
| `--secure-protocol=protocol` | 安全协议（`auto`/`TLSv1_2`…/`PFS`） |
| `--https-only` | 递归只跟随 HTTPS |
| `--ciphers` | 密码套件字符串 |
| `--no-check-certificate` | 不校验服务器证书（危险） |
| `--certificate` / `--certificate-type` / `--private-key` / `--private-key-type` | 客户端证书 |
| `--ca-certificate` / `--ca-directory` / `--crl-file` | CA / CRL |
| `--pinnedpubkey` | 固定公钥 |
| `--random-file` / `--egd-file` | 随机源 |
| `--no-hsts` / `--hsts-file=file` | HSTS 控制 |
| `--warc-file` / `--warc-header` / `--warc-max-size` / `--warc-cdx` / `--warc-dedup` | WARC 归档 |

#### FTP 相关

| 选项 | 说明 |
|---|---|
| `--ftp-user` / `--ftp-password` | FTP 凭据 |
| `--no-remove-listing` | 保留 `.listing` 文件 |
| `--no-glob` | 关闭 FTP 通配 |
| `--no-passive-ftp` | 禁用被动模式 |
| `--preserve-permissions` | 保留远端权限 |
| `--retr-symlinks` | 递归时跟随符号链接 |
| `--ftps-implicit` / `--no-ftps-resume-ssl` / `--ftps-clear-data-connection` / `--ftps-fallback-to-ftp` | FTPS 控制 |

#### 递归 / 镜像

| 选项 | 说明 |
|---|---|
| `-r, --recursive` | 递归抓取（默认最大深度 5） |
| `-l, --level=depth` | 最大递归深度（`0`/`inf` 无限） |
| `--delete-after` | 下载后删除每个文件（预取场景） |
| `-k, --convert-links` | 下载后把链接转成本地可看 |
| `--convert-file-only` | 只转换文件名部分 |
| `-K, --backup-converted` | 转换前备份 `.orig` |
| `-m, --mirror` | 镜像（等价 `-r -N -l inf --no-remove-listing`） |
| `-p, --page-requisites` | 下载页面所需的内嵌资源 |
| `--strict-comments` | 严格解析 HTML 注释 |
| `-A, --accept acclist` / `-R, --reject rejlist` | 按后缀/模式接受/拒绝 |
| `--accept-regex` / `--reject-regex` / `--regex-type` | 按正则接受/拒绝 |
| `-D, --domains=list` / `--exclude-domains list` | 限定/排除域名 |
| `--follow-ftp` | 跟随 HTML 里的 FTP 链接 |
| `--follow-tags=list` / `--ignore-tags=list` | 指定/忽略 HTML 标签 |
| `--ignore-case` | 匹配忽略大小写 |
| `-H, --span-hosts` | 跨主机递归 |
| `-L, --relative` | 只跟随相对链接 |
| `-I, --include-directories=list` / `-X, --exclude-directories=list` | 包含/排除目录 |
| `-np, --no-parent` | 不向父目录上溯 |

### 示例

```bash
wget https://example.com/file.iso              # 下载单文件
wget -c https://example.com/big.iso            # 断点续传
wget -q -O - https://example.com/x | head      # 输出到 stdout
wget --spider -i urls.txt                      # 批量检查 URL 存活
wget -r -np -nH --cut-dirs=2 -l 3 https://site/docs/   # 递归抓文档
wget -m -k -p https://site/page.html           # 镜像单页及其资源
wget -r -A '*.pdf' -np https://site/files/     # 只抓 PDF
wget --limit-rate=200k -t 3 -T 30 url          # 限速 + 重试 + 超时
wget --header='Referer: https://a/' url        # 自定义请求头
wget --post-data='a=1&b=2' https://api/x       # POST
```

### 易踩坑

- **`-O` 会把多个 URL 的内容合并到一个文件**，不是「重命名」，多 URL 场景慎用。
- `-r` 默认深度只有 **5**，想抓更深要显式 `-l`。
- `-m`（镜像）会开启无限深度 + 时间戳 + 保留 FTP 列表，容易把整个站拖下来，务必配合 `-np`、`-A`/`-R`、`--domains` 收窄。
- `-k`（convert-links）在**下载完成后**才转换，中途 Ctrl-C 会留下未转换的页面。
- `--no-check-certificate` 会跳过证书校验（危险）。
- `-b` 后台模式的输出默认进 `wget-log`，不在终端可见。
- 递归抓取对目标站是压力，**默认没有速率限制**，请自觉 `-w` 或 `--limit-rate`。

### macOS 差异

macOS **不自带 wget**（自带 `curl`）。需 `brew install wget`。Homebrew 版是标准 GNU Wget，选项与 Linux 一致。

---

## 5. ss — 查看端口/连接（替代 netstat）

`ss`（socket statistics）用 netlink 直接从内核读 socket 信息，比 netstat 遍历 `/proc` 快得多，是 netstat 的现代替代。

### 基本语法

```
ss [options] [ FILTER ]
```

### 选项（手册共 46 项，全列）

| 选项 | 说明 |
|---|---|
| `-h, --help` | 显示选项摘要 |
| `-V, --version` | 版本信息 |
| `-H, --no-header` | 不显示表头 |
| `-Q, --no-queues` | 不显示收发队列列 |
| `-O, --oneline` | 每个 socket 一行 |
| `-n, --numeric` | 不解析服务名，显示精确带宽值 |
| `-r, --resolve` | 解析数字地址/端口 |
| `-a, --all` | 显示监听与非监听（已建立）socket |
| `-l, --listening` | 只显示监听 socket（默认不显示） |
| `-B, --bound-inactive` | 只显示已绑定但未激活的 TCP socket |
| `-o, --options` | 显示定时器信息 |
| `-e, --extended` | 显示详细 socket 信息（uid/inode/cookie） |
| `-m, --memory` | 显示 socket 内存占用 |
| `-p, --processes` | 显示使用该 socket 的进程 |
| `-T, --threads` | 显示线程（隐含 `-p`） |
| `-i, --info` | 显示内部 TCP 信息（ts/sack/ecn/cong 等） |
| `--tos` | 显示 ToS 与优先级 |
| `--cgroup` | 显示 cgroup 信息 |
| `--tipcinfo` | 显示 TIPC 内部信息 |
| `-K, --kill` | 尝试强制关闭 socket（仅 IPv4/IPv6） |
| `-s, --summary` | 打印汇总统计（不解析 socket 列表） |
| `-E, --events` | 持续显示被销毁的 socket |
| `-Z, --context` | 同 `-p` 但显示进程安全上下文 |
| `-z, --contexts` | 同 `-Z` 但显示 socket 上下文 |
| `-N NSNAME, --net=NSNAME` | 切换到指定网络命名空间 |
| `-b, --bpf` | 显示 socket 的经典 BPF 过滤器（需管理员） |
| `-4, --ipv4` | 只显示 IPv4 socket（`-f inet` 别名） |
| `-6, --ipv6` | 只显示 IPv6 socket（`-f inet6` 别名） |
| `-0, --packet` | 显示 PACKET socket（`-f link` 别名） |
| `-t, --tcp` | 显示 TCP socket |
| `-u, --udp` | 显示 UDP socket |
| `-d, --dccp` | 显示 DCCP socket |
| `-w, --raw` | 显示 RAW socket |
| `-x, --unix` | 显示 Unix 域 socket（`-f unix` 别名） |
| `-S, --sctp` | 显示 SCTP socket |
| `--tipc` / `--vsock` / `--xdp` | 显示 TIPC / vsock / XDP socket |
| `-M, --mptcp` | 显示 MPTCP socket |
| `--inet-sockopt` | 显示 inet socket 选项 |
| `-f FAMILY, --family=FAMILY` | 按地址族显示（unix/inet/inet6/link/netlink/vsock/tipc/xdp） |
| `-A QUERY, --query=QUERY` | 要 dump 的 socket 表（逗号分隔，可前缀 `!` 排除） |
| `-D FILE, --diag=FILE` | 不显示，只把 TCP socket 原始信息 dump 到 FILE |
| `-F FILE, --filter=FILE` | 从文件读过滤条件（每行一个选项） |
| `--bpf-maps` / `--bpf-map-id=MAP_ID` | 美化打印 BPF socket 本地数据 |

### 过滤器（FILTER）

`ss` 的过滤表达式用状态名或字段条件，是它比 netstat 强的地方：

```bash
ss -t state established          # 只看已建立的 TCP
ss -t state listening            # 只看监听
ss -tn state time-wait           # TIME_WAIT
ss -tn sport = :22               # 源端口 22
ss -tn dport = :443              # 目标端口 443
ss -tn dst 10.0.0.1              # 目标地址
ss -tn '( dport = :80 or dport = :443 )'
```

TCP 状态名（`ss -t` 的 `state`）：`established`、`syn-sent`、`syn-recv`、`fin-wait-1`、`fin-wait-2`、`time-wait`、`closed`、`close-wait`、`last-ack`、`listening`、`closing`。
聚合写法：`all`、`connected`（除 listening/closed 外全部）、`synchronized`（除 syn-sent 外）。

### 示例

```bash
ss -tulnp                        # 最常用：TCP+UDP 监听 + 进程 + 数字
ss -tnp                          # 所有 TCP 连接 + 进程
ss -s                            # 汇总统计
ss -tn state time-wait | wc -l   # 数 TIME_WAIT 数量
ss -tlnp 'sport = :8080'         # 谁占了 8080
ss -o state established         # 含定时器
ss -i                            # 含拥塞/窗口内部信息
ss -K dst 1.2.3.4 dport = :80    # 强制关闭到某目标的连接
```

### 易踩坑

- **不加 `-a`/`-l` 时，监听 socket 默认不显示**，所以 `ss -t` 看不到监听端口，要用 `ss -tl`。
- `-p` 看进程需要 **root**（或 `CAP_NET_ADMIN`），普通用户看不到别人的进程。
- `-n` 不加会做服务名反解，输出慢且端口显示成 `ssh`/`https` 而非数字；排障建议总是加 `-n`。
- `-K`（kill socket）会直接掐断连接，属于破坏性操作，慎用。
- `ss` 显示的内存/队列字段（`Recv-Q`/`Send-Q`）在**监听** socket 上含义特殊：`Recv-Q` 是 accept 队列积压，`Send-Q` 是 backlog 上限。

### macOS 差异

**macOS 没有 `ss`**。等价手段：

```bash
netstat -an | grep LISTEN        # 看监听
lsof -nP -iTCP -sTCP:LISTEN      # 带进程的监听列表
lsof -nP -i:8080                 # 谁占了 8080
```

---

## 6. netstat — 网络状态（旧，已废弃）

net-tools 套件成员，已被 `ss` + `ip` 取代，但仍在老系统与脚本中广泛存在。

### 基本语法

```
netstat [address_family_options] [--tcp|-t] [--udp|-u] ...
```

### 选项（手册共 21 项，全列）

| 选项 | 说明 |
|---|---|
| `-r, --route` | 显示内核路由表 |
| `-g, --groups` | 显示 IPv4/IPv6 组播组成员 |
| `-i, --interfaces` | 显示接口及收发错误计数（等价 `ifconfig -s`） |
| `-M, --masquerade` | 显示被 masquerade 的连接 |
| `-s, --statistics` | 各协议汇总（SNMP）统计 |
| `-v, --verbose` | 详细输出 |
| `-W, --wide` | 不截断地址 |
| `-n, --numeric` | 显示数字地址，不解析 |
| `--numeric-hosts` | 只不解析主机名 |
| `--numeric-ports` | 只不解析端口名 |
| `--numeric-users` | 只不解析用户 ID |
| `-A Family, --protocol=Family` | 指定地址族（inet/inet6/unix/ipx/ax25/netrom/econet/ddp/bluetooth） |
| `-c, --continuous` | 每秒持续打印 |
| `-e, --extend` | 显示额外信息（可重复） |
| `-o, --timers` | 显示网络定时器 |
| `-p, --program` | 显示 PID 与程序名（不完全可信） |
| `-l, --listening` | 只显示监听 socket |
| `-a, --all` | 显示监听与非监听 socket |
| `-F` | 从 FIB 打印路由信息（默认） |
| `-C` | 从路由缓存打印（内核 3.6 后已移除缓存） |
| `-t/-u/-w/-x` | 分别显示 TCP/UDP/RAW/Unix socket |

### 示例

```bash
netstat -tulnp                   # 监听端口 + 进程（最常见用法）
netstat -an | grep :80           # 所有 80 端口连接
netstat -rn                      # 路由表
netstat -i                       # 接口统计
netstat -s                       # 协议统计
netstat -anp | grep ESTABLISHED  # 已建立连接
netstat -c -t                    # 每秒刷新 TCP
```

### 易踩坑

- **netstat 已废弃**，新系统可能默认未安装（`net-tools` 包）。排障请优先 `ss`。
- `-p` 的进程信息**不可信**：无权限或 socket 已移交内核时显示 `-`；进程退出但 socket 未关闭时也会误导。
- `-n` 不加会做大量 DNS 反解，在反向解析慢的环境下会卡很久。
- 大量连接时 netstat 遍历 `/proc/net/*` 很慢，这正是 `ss` 取代它的原因。
- `-C`（路由缓存）在 Linux 3.6+ 已无意义。

### macOS 差异

**macOS 自带 netstat**，但它与 Linux 版**选项不完全一致**：macOS 用 `-b`/`-d`/`-m`/`-g`/`-s`，**没有 `-t`/`-u`/`-p`**。看监听要 `netstat -an | grep LISTEN`，看进程要 `lsof -i`。macOS 的 `netstat -rn` 输出格式也与 Linux 不同。

---

## 7. tcpdump — 抓包

libpcap 的命令行前端，网络排障的「显微镜」。手册共 **95 项**。

### 基本语法

```
tcpdump [ -AbdDefhHIJKlLnNOpqStuUvxX# ] [ -B buffer_size ]
        [ -c count ] [ -C file_size ] [ -G rotate_seconds ] [ -F file ]
        [ -i interface ] [ -j tstamp_type ] [ -m module ] [ -M secret ]
        [ -Q|-P in|out|inout ] [ -r file ] [ -s snaplen ] [ -T type ]
        [ -V file ] [ -w file ] [ -W filecount ] [ -y datalinktype ]
        [ -z postrotate-command ] [ -Z user ] [ expression ]
```

### 选项（手册共 95 项，全列）

| 选项 | 说明 |
|---|---|
| `-A` | 每个包以 ASCII 打印（去链路层头） |
| `-b` | BGP 的 AS 号用 asdot 表示 |
| `-B buffer_size` | 内核捕获缓冲区大小（KiB） |
| `--buffer-size=buffer_size` | 同上（长选项） |
| `-c count` | 收到 count 个包后退出 |
| `--count` | 读文件时只打印包数 |
| `-C file_size` | 写文件超过 file_size（百万字节）则轮转 |
| `-d` | 以人类可读形式 dump 编译后的过滤代码 |
| `-dd` | 以 C 数组形式 dump 过滤代码 |
| `-ddd` | 以十进制数 dump 过滤代码 |
| `-D, --list-interfaces` | 列出可抓包的接口 |
| `-e` | 打印链路层头（MAC 地址） |
| `-E` | 用 spi@ipaddr 解密 IPsec ESP |
| `-f` | 外部 IPv4 地址用数字表示 |
| `-F file` | 从文件读过滤表达式 |
| `-g, --ip-oneline` | 详细模式下 IP 头后不换行 |
| `-G rotate_seconds` | 每 N 秒轮转 `-w` 文件（文件名含 strftime 格式） |
| `-h, --help` | 打印版本与用法 |
| `--version` | 打印版本 |
| `-H` | 尝试检测 802.11s mesh 头 |
| `-i interface` | 指定抓包接口 |
| `--interface=interface` | 同上（长选项） |
| `-I, --monitor-mode` | 接口进入 monitor 模式（仅 802.11） |
| `--immediate-mode` | 立即模式：包一到就交给 tcpdump |
| `-j tstamp_type` | 设置时间戳类型 |
| `--time-stamp-type=` | 同上（长选项） |
| `-J, --list-time-stamp-types` | 列出支持的时间戳类型 |
| `--time-stamp-precision=` | 设置时间戳精度 |
| `--micro` / `--nano` | 微秒 / 纳秒精度 |
| `-K, --dont-verify-checksums` | 不校验 IP/TCP/UDP 校验和 |
| `-l` | 行缓冲 stdout |
| `-L, --list-data-link-types` | 列出数据链路类型 |
| `--lengths` | 打印捕获长度与原始长度 |
| `-m module` | 加载 SMI MIB 模块 |
| `-M secret` | TCP-MD5（RFC2385）共享密钥 |
| `-n` | 不把地址/端口转成名字 |
| `-N` | 不打印主机名的域名限定 |
| `-#, --number` | 行首打印包序号 |
| `-O, --no-optimize` | 不运行过滤代码优化器 |
| `-p, --no-promiscuous-mode` | 不进入混杂模式 |
| `--print` | 即使 `-w` 存盘也打印解析结果 |
| `--print-sampling=nth` | 每第 nth 个包打印（隐含 `--print` 与 `-S`） |
| `-Q direction, --direction=` | 抓包方向：`in`/`out`/`inout` |
| `-q` | 快速输出，减少协议信息 |
| `-r file` | 从文件读包 |
| `-S, --absolute-tcp-sequence-numbers` | 打印绝对 TCP 序号 |
| `-s snaplen, --snapshot-length=` | 每包抓 snaplen 字节（默认 262144） |
| `--skip count` | 跳过前 count 个包 |
| `-T type` | 把选中的包按指定类型解释（`aodv`/`carp`/`domain`/`pgm`/`ptp`…） |
| `-t` | 不打印时间戳 |
| `-tt` | 打印 unix 时间戳 |
| `-ttt` | 打印与上一行的差值 |
| `-tttt` | 打印日期 + 时分秒 |
| `-ttttt` | 打印与第一行的差值 |
| `-u` | 打印未解码的 NFS 句柄 |
| `-U, --packet-buffered` | 包缓冲输出 |
| `-v` | 稍详细的输出 |
| `-vv` | 更详细 |
| `-vvv` | 最详细 |
| `-V file` | 从文件读文件名列表 |
| `-w file` | 把原始包写文件（不解析） |
| `-W filecount` | 配合 `-C`/`-G` 限制文件数量并循环覆盖 |
| `-x` | 打印包头外，再以十六进制打印数据（去链路层头） |
| `-xx` | 同上但含链路层头 |
| `-X` | 十六进制 + ASCII（去链路层头） |
| `-XX` | 十六进制 + ASCII（含链路层头） |
| `-y datalinktype, --linktype=` | 设置数据链路类型 |
| `-z postrotate-command` | 轮转后对文件执行命令（如 `-z gzip`） |
| `-Z user, --relinquish-privileges=` | 打开设备后降权到 user |

### 过滤表达式（BPF 语法）

```
tcpdump -i eth0 port 80
tcpdump -i eth0 host 10.0.0.1
tcpdump -i eth0 src 10.0.0.1 and dst port 443
tcpdump -i eth0 'tcp[tcpflags] & (tcp-syn|tcp-ack) != 0'   # SYN/ACK
tcpdump -i eth0 'tcp[13] == 2'                             # 只看 SYN
tcpdump -i eth0 -nn 'udp port 53'
tcpdump -i eth0 'not port 22'
tcpdump -i eth0 'ether host 00:11:22:33:44:55'
```

### 示例

```bash
tcpdump -i eth0 -nn port 80                  # 抓 80 端口
tcpdump -i any -nn -c 100                    # 任意接口抓 100 个
tcpdump -i eth0 -w cap.pcap                  # 存盘（推荐）
tcpdump -r cap.pcap -nn | less               # 读盘分析
tcpdump -i eth0 -A 'port 80 and host 1.2.3.4'  # 看 HTTP 明文
tcpdump -i eth0 -s 0 -w cap.pcap -C 100 -W 5   # 抓全包、每 100MB 轮转、留 5 个
tcpdump -i eth0 -G 3600 -w '%Y%m%d-%H.pcap'    # 每小时一个文件
tcpdump -i eth0 -c 10 -e -nn 'arp'           # 抓 ARP，带 MAC
```

### 易踩坑

- **`-s` 默认截断长度**：老版本默认 68 字节，会截断应用层数据。要抓全包用 `-s 0`（新版默认已是 262144，但写 `-s 0` 更保险）。
- **抓包需 root 或 `CAP_NET_RAW`**，普通用户会报权限错误。
- `-i any` 在部分接口类型下抓不到链路层头，`-e` 可能无效。
- 生产环境抓包**务必 `-w` 存盘再分析**，直接在终端解析会丢包（尤其高流量时）。
- `-w` 写文件是**缓冲**的，抓完要用 Ctrl-C 正常退出让缓冲区落盘，否则文件可能不完整；实时消费加 `-U`。
- 过滤表达式**一定要加引号**，否则 `>`、`|`、`(` 会被 shell 解释。
- `-A`/`-x`/`-X` 与 `-w` 同用时，存的是原始包，解析输出只在 `--print` 下才有。

### macOS 差异

macOS 自带 tcpdump（基于 BSD 版），核心选项一致，但**接口名是 `en0`/`en1`/`lo0`** 而非 `eth0`。`-I`（monitor 模式）在 macOS 上支持得更好。安装 Homebrew 版可获取更新的 libpcap。

---

## 8. nc — 网络调试 / 端口扫描 / 传数据

`nc`（netcat）是「网络瑞士军刀」。此处以 **OpenBSD netcat** 为准（Linux 发行版常用，也是 macOS 自带版本）。

### 基本语法

```
nc [-46bCDdFhklNnrStUuvZz] [-I length] [-i interval] [-M ttl] [-m minttl]
   [-O length] [-P proxy_username] [-p source_port] [-q seconds] [-s sourceaddr]
   [-T keyword] [-V rtable] [-W recvlimit] [-w timeout] [-X proxy_protocol]
   [-x proxy_address[:port]] [destination] [port]
```

### 选项（全列）

| 选项 | 说明 |
|---|---|
| `-4` | 只用 IPv4 |
| `-6` | 只用 IPv6 |
| `-b` | 允许广播 |
| `-C` | 发送时把 LF 转成 CRLF |
| `-D` | 开启 socket 调试 |
| `-d` | 不从 stdin 读 |
| `-F` | 把第一个连上的 socket 用 `sendmsg` 传给 stdout 后退出（配合 `-X`） |
| `-h` | 打印帮助 |
| `-I length` | 设置 TCP 接收缓冲区大小 |
| `-i interval` | 每行收发之间休眠 interval 秒；多端口连接之间也延迟 |
| `-k` | 连接结束后继续监听（需 `-l`） |
| `-l` | 监听模式（不能与 `-x`/`-z` 同用；`-w` 超时被忽略） |
| `-M ttl` | 设置出包 TTL / hop limit |
| `-m minttl` | 丢弃 TTL 低于 minttl 的入包 |
| `-N` | 输入 EOF 后 `shutdown()` socket |
| `-n` | 不做域名解析 |
| `-O length` | 设置 TCP 发送缓冲区大小 |
| `-P proxy_username` | 向需要认证的代理提供的用户名（仅 HTTP CONNECT） |
| `-p source_port` | 指定源端口 |
| `-q seconds` | stdin EOF 后等 N 秒再退出；负数表示永久等；非负隐含 `-N` |
| `-r` | 源/目的端口随机而非顺序 |
| `-S` | 启用 RFC2385 TCP MD5 签名 |
| `-s sourceaddr` | 指定源地址 |
| `-T keyword` | 设置 IPv4 TOS / IPv6 traffic class |
| `-t` | 对 RFC854 DO/WILL 回 DON'T/WON'T（用于脚本化 telnet 会话） |
| `-U` | 用 Unix 域 socket（`@` 开头为抽象命名空间） |
| `-u` | 用 UDP |
| `-V rtable` | 设置路由表 |
| `-v` | 更详细输出 |
| `-W recvlimit` | 收到 recvlimit 个包后终止 |
| `-w timeout` | 连接建立或空闲超时秒数（对 `-l` 无效） |
| `-X proxy_protocol` | 代理协议：`4`/`4A`/`5`/`connect`（默认 SOCKS5） |
| `-x proxy_address[:port]` | 经代理连接（不能与 `-lsuU` 同用） |
| `-Z` | DCCP 模式 |
| `-z` | 只扫描不发送数据（不能与 `-l` 同用） |

### 示例

```bash
nc -zv host 2181                # 测端口是否开
nc -zv host 20-30               # 扫端口范围
nc -zv -w 2 host 80 443 8080    # 批量测端口，2s 超时
nc -l 1234                      # 监听 1234
nc -N 127.0.0.1 1234            # 连到上面
nc -l -u 1234                   # UDP 监听
nc -lk 1234                     # 持续监听
nc host 80                      # 手打 HTTP 请求
printf 'GET / HTTP/1.0\r\n\r\n' | nc host 80
tar czf - dir | nc -l 9999      # 传目录（接收端 nc host 9999 | tar xzf -）
nc -U /tmp/mysock               # Unix 域 socket
nc -X connect -x proxy:3128 host 443   # 经 HTTP 代理
```

### 易踩坑

- **`-l` 与 `-w` 的语义**：`-w` 对监听模式无效，监听会一直等。
- 默认 `nc` 在 stdin EOF 后**不立即退出**，可能挂住。用 `-N`（或 `-q 0`）强制 EOF 后关闭。
- 不同实现的 `nc`（OpenBSD / GNU netcat / nmap-ncat）**选项差异极大**，脚本里最好显式判断版本。例如 GNU netcat 的 `-l -p` 用法与 OpenBSD 的 `nc -l 1234` 不同。
- `-z` 扫描属于主动探测，会触发目标 IDS/审计日志，**未经授权不要对他人主机扫描**。
- `-k` 必须配合 `-l`，否则无意义。
- `nc` 传输**无加密、无校验**，跨公网传数据请走 `scp`/`rsync`/`ssh`。

### macOS 差异

macOS 自带的是 **OpenBSD netcat**，选项与本节一致。但**监听时 `-l` 后不能带端口**（macOS 老版需 `nc -l 1234`，部分版本要求 `-p`）。Linux 上则可能是 GNU netcat 或 nmap-ncat，用法不同。

---

## 9. telnet — 端口测试（明文）

古老的远程登录协议，明文传输。现代主要用途是**测端口连通性**（`telnet host port`）。

### 基本语法

```
telnet [OPTION...] [HOST [PORT]]
```

### 选项（GNU inetutils 2.8，全列）

| 选项 | 说明 |
|---|---|
| `-4, --ipv4` | 只用 IPv4 |
| `-6, --ipv6` | 只用 IPv6 |
| `-8, --binary` | 8 位数据传输 |
| `-a, --login` | 尝试自动登录 |
| `-b, --bind=ADDRESS` | 绑定本地地址 |
| `-c, --no-rc` | 不读用户 `.telnetrc` |
| `-d, --debug` | 开启调试 |
| `-e, --escape=CHAR` | 指定转义字符 |
| `-E, --no-escape` | 不使用转义字符 |
| `-K, --no-login` | 不自动登录 |
| `-l, --user=USER` | 以 USER 自动登录 |
| `-L, --binary-output` | 输出用 8 位传输 |
| `-n, --trace=FILE` | 把追踪信息写入 FILE |
| `-r, --rlogin` | 使用类似 rlogin 的界面 |
| `-?, --help` | 显示帮助 |
| `--usage` | 简短用法 |
| `-V, --version` | 显示版本 |

### 示例

```bash
telnet host 22                  # 测 22 端口（能连上会看到 SSH banner）
telnet host 80                  # 测 80
telnet 10.0.0.1 3306            # 测 MySQL 端口
```

### 易踩坑

- **telnet 是明文协议**，登录凭据会裸奔在网络上。**绝不要用它登录生产服务器**，登录用 `ssh`。
- 现代用途只剩「测端口通不通」。连上后看到 banner 即端口开放；`Connection refused` 表示端口关；一直挂着表示被防火墙丢包。
- **退出方式**：先按 `Ctrl-]` 进命令模式，再输 `quit` 回车（直接 `Ctrl-C` 在部分实现里不生效）。
- 很多系统默认**不装 telnet**（因安全原因被移除），测端口更推荐 `nc -zv` 或 `curl -v telnet://host:port`。
- 部分发行版的 `telnet` 是 BSD 版，选项与 GNU 版不同。

### macOS 差异

macOS 自带 telnet（BSD 版）。macOS 新版已移除，需 `brew install telnet`。选项与 GNU 版有差异。

---

## 10. dig / nslookup — DNS 查询

`dig`（BIND 9 的 DNS 工具箱，输出详细、脚本友好）与 `nslookup`（老式交互式查询器，BIND 9 仍附带）。

### dig 基本语法

```
dig [@server] [-b address] [-c class] [-f filename] [-k filename] [-m]
    [-p port#] [-q name] [-t type] [-v] [-x addr] [-y [hmac:]name:key]
    [ [-4] | [-6] ] [name] [type] [class] [queryopt...]
```

### dig 命令行选项（全列）

| 选项 | 说明 |
|---|---|
| `-4` | 只用 IPv4 |
| `-6` | 只用 IPv6 |
| `-b address[#port]` | 绑定源地址（可带端口） |
| `-c class` | 查询类别（IN/CH/HS…） |
| `-f file` | 从文件批量读查询（批处理模式） |
| `-h` | 用法摘要 |
| `-k keyfile` | TSIG 密钥文件 |
| `-m` | 内存调试 |
| `-p port` | 查询非标准端口 |
| `-q name` | 指定查询域名 |
| `-r` | 不读 `~/.digrc` |
| `-t type` | 查询类型（A/AAAA/MX/NS/SOA/TXT…） |
| `-u` | 查询耗时用微秒显示 |
| `-v` | 版本号 |
| `-x addr` | 反向查询（PTR） |
| `-y [hmac:]name:key` | TSIG 密钥（命令行明文，不推荐） |

### dig 的 `+queryopt` 查询选项（常用，全表见手册）

| 选项 | 说明 |
|---|---|
| `+[no]short` | 只输出结果，不显示头部/统计 |
| `+[no]answer` | 显示/隐藏 answer 段 |
| `+[no]authority` | 显示/隐藏 authority 段 |
| `+[no]additional` | 显示/隐藏 additional 段 |
| `+[no]comments` | 显示/隐藏注释行 |
| `+[no]stats` | 显示/隐藏统计 |
| `+[no]question` | 显示/隐藏问题段 |
| `+[no]cmd` | 显示/隐藏初始注释 |
| `+[no]all` | 一次控制所有显示段 |
| `+[no]recurse`（`+[no]rec`） | 是否请求递归 |
| `+[no]tcp`（`+[no]vc`） | 用 TCP 而非 UDP |
| `+[no]dnssec`（`+[no]do`） | 请求 DNSSEC 记录 |
| `+[no]trace` | 从根开始逐级追踪 |
| `+[no]search` | 使用搜索列表 |
| `+[no]aaflag` / `+[no]adflag` / `+[no]cdflag` / `+[no]raflag` / `+[no]rdflag` / `+[no]tcflag` / `+[no]zflag` / `+[no]coflag` | 各 DNS 标志位 |
| `+bufsize[=B]` / `+[no]edns[=#]` / `+[no]ednsflags[=#]` / `+[no]ednsopt[=code[:value]]` / `+[no]ednsnegotiation` | EDNS 控制 |
| `+[no]cookie[=####]` / `+[no]showbadcookie` / `+[no]badcookie` | DNS Cookie |
| `+[no]besteffort` / `+[no]fail` | 畸形消息处理 |
| `+[no]multiline` / `+[no]rrcomments` | 多行输出 / 记录注释 |
| `+[no]identify` | 显示应答服务器地址 |
| `+[no]nsid` / `+[no]nssearch` | 名称服务器 ID / 搜索 |
| `+[no]subnet=addr` | EDNS Client Subnet |
| `+[no]keepopen` / `+[no]keepalive` | 保持连接 |
| `+[no]tls` / `+[no]https` / `+[no]http` | DoT / DoH / DoH(GET) |
| `+[no]proxy=src_addr[#src_port]` | 代理 |
| `+[no]timeout=N` / `+[no]tries=N` / `+[no]retry=N` | 超时/重试 |
| `+[no]ttlid` / `+[no]ttlunits` | TTL 显示 |
| `+[no]yaml` / `+[no]split` / `+[no]padding[=N]` | YAML 输出 / 拆分 / 填充 |
| `+[no]idn` / `+[no]ignore` / `+[no]crypto` / `+[no]defname` / `+[no]domain=NAME` / `+[no]ndots=N` / `+[no]onesoa` / `+[no]opcode=CODE` / `+[no]qid=N` / `+[no]qr` / `+[no]class` / `+[no]header` / `+[no]unknownformat` / `+[no]fuzztime[=value]` | 其他 |

### nslookup 用法

nslookup 有交互与非交互两种模式：

```
nslookup [-option] [name | -] [server]
```

非交互（最常用）：

```bash
nslookup baidu.com                 # 用默认 DNS 查 A（并显示 AAAA）
nslookup baidu.com 8.8.8.8         # 指定 DNS 服务器
nslookup -query=mx baidu.com       # 指定查询类型
nslookup -query=ns -timeout=10 baidu.com
nslookup -version                  # 显示版本
```

交互模式命令：

| 命令 | 说明 |
|---|---|
| `host [server]` | 查询主机信息 |
| `server domain` / `lserver domain` | 切换默认服务器 |
| `exit` | 退出 |
| `set keyword[=value]` | 修改查询状态（见下） |
| `root` / `finger` / `ls` / `view` / `help` / `?` | **均未实现**（BIND 9 版） |

`set` 的合法关键字（BIND 9 手册原文）：

| 关键字 | 说明 |
|---|---|
| `all` | 打印当前所有常用选项值 |
| `class=value`（缩写 `cl`） | 查询类别：IN/CH/HS/ANY，默认 IN |
| `nodebug`（缩写 `[no]deb`） | 是否显示完整响应包 |
| `nod2` | 调试模式开关，默认 nod2 |
| `domain=name` | 设置搜索列表 |
| `nosearch` / `search` | 是否自动补全搜索域，默认 search |
| `port=value`（缩写 `po`） | DNS 端口，默认 53 |
| `querytype=value` / `type=value`（缩写 `q`/`ty`） | 查询类型，默认先 A 后 AAAA |
| `norecurse` / `recurse`（缩写 `[no]rec`） | 是否请求递归，默认 recurse |
| `ndots=number` | 几个点后停止搜索 |
| `retry=number` | 重试次数 |
| `timeout=number` | 初始超时秒数 |
| `novc` / `vc` | 是否总用虚电路（TCP），默认 novc |
| `nofail` / `fail` | 遇 SERVFAIL/referral 是换服务器还是终止，默认 nofail |

### 示例

```bash
dig baidu.com                      # A 记录
dig +short baidu.com               # 只要结果
dig MX baidu.com                   # 邮件记录
dig NS baidu.com                   # 权威服务器
dig TXT baidu.com                  # TXT
dig -x 8.8.8.8                     # 反向解析
dig @8.8.8.8 baidu.com +noall +answer
dig +trace baidu.com               # 逐级追踪
dig +dnssec +short baidu.com       # DNSSEC
dig +tcp baidu.com                 # 强制 TCP
dig -f queries.txt +short          # 批量查询
dig +short -t AAAA baidu.com
nslookup -query=mx baidu.com 1.1.1.1
```

### 易踩坑

- **`dig` 默认无 scheme，`dig` 不带参数时查的是 `.` 的 NS**，不是本机。
- `dig` 的**退出码**：0=成功，9=无应答（如 NXDOMAIN 也可能返回 0），脚本判定要结合 `+short` 是否为空。
- `+trace` 会直接向根/各级服务器发查询，绕过本机递归解析器，**输出量大且耗时**。
- `nslookup` 的**输出格式随版本变化**（BIND 9.9 前后不同），不适合脚本解析；脚本请用 `dig +short`。
- `-y` 在命令行明文传 TSIG 密钥，会被 `ps` 看到，应改用 `-k` 密钥文件。
- 反向查询（`-x`）只对 PTR 记录有效，很多 IP 无 PTR 会返回 NXDOMAIN，属正常。
- **DNS 缓存**：改了记录但没生效，先确认是不是本地/上游缓存（`dig` 直接问权威服务器可绕过）。

### macOS 差异

macOS 自带 `dig`/`nslookup`（BIND 版）与 `host`。另外可用 `dscacheutil -q host -a name X` 与 `scutil --dns` 查看系统 DNS 配置。macOS 的解析器由 mDNSResponder 管理，`/etc/resolv.conf` 不一定反映真实配置。

---

## 11. traceroute — 路由追踪

利用逐步增大 TTL 让沿途路由器回 ICMP 超时，从而绘出到目标的路径。

### 基本语法

```
traceroute [options] host [packet_len]
```

### 选项（手册共 34 项，全列）

| 选项 | 说明 |
|---|---|
| `--help` | 打印帮助 |
| `-4` / `-6` | 强制 IPv4 / IPv6 |
| `-I, --icmp` | 用 ICMP ECHO 探测 |
| `-T, --tcp` | 用 TCP SYN 探测 |
| `-d, --debug` | socket 层调试 |
| `-F, --dont-fragment` | 不分片探测包（IPv4 还会设 DF 位） |
| `-f first_ttl, --first=first_ttl` | 起始 TTL，默认 1 |
| `-g gateway, --gateway=gateway` | 源路由经过指定网关 |
| `-i interface, --interface=interface` | 指定发送接口 |
| `-m max_ttl, --max-hops=max_ttl` | 最大跳数，默认 30 |
| `-N squeries, --sim-queries=squeries` | 并发探测包数，默认 16 |
| `-n` | 不把 IP 反解成主机名 |
| `-p port, --port=port` | 目的端口基准（UDP 递增 / TCP 固定） |
| `-t tos, --tos=tos` | IPv4 TOS / IPv6 Traffic Class |
| `-l flow_label, --flowlabel=flow_label` | IPv6 flow label |
| `-w max[,here,near], --wait=max[,here,near]` | 等待响应时间（默认 5.0s） |
| `-q nqueries, --queries=nqueries` | 每跳探测包数，默认 3 |
| `-r` | 绕过路由表直接发给直连主机 |
| `-s source_addr, --source=source_addr` | 指定源地址 |
| `-z sendwait, --sendwait=sendwait` | 探测最小间隔（>10 为毫秒，否则为秒） |
| `-e, --extensions` | 显示 ICMP 扩展（RFC4884） |
| `-A, --as-path-lookups` | 做 AS 路径查询 |
| `-V, --version` | 版本 |
| `--sport=port` | 指定源端口（隐含 `-N 1 -w 5`） |
| `--fwmark=mark` | 设置防火墙标记（内核 2.6.25+） |
| `-M method, --module=name` | 指定追踪方法（`default`/`icmp`/`tcp`…） |
| `-O option, --options=options` | 方法专用选项（`-O help` 查看） |
| `-U, --udp` | 用 UDP（固定目的端口，默认 53） |
| `-UL` | 用 UDPLITE（默认端口 53） |
| `-D, --dccp` | 用 DCCP |
| `-P protocol, --protocol=protocol` | 用指定协议原始包（默认 253） |
| `--mtu` | 发现路径 MTU（隐含 `-F -N 1`） |
| `--back` | 打印反向跳数 |

### 示例

```bash
traceroute baidu.com              # 默认 UDP 探测
traceroute -I baidu.com           # ICMP 探测（更易穿防火墙）
traceroute -T -p 443 baidu.com    # TCP SYN 到 443（穿防火墙最好）
traceroute -n -m 15 baidu.com     # 不反解、最多 15 跳
traceroute -q 1 -w 2 baidu.com    # 每跳 1 包、2s 超时（快速）
traceroute --mtu baidu.com        # 找路径 MTU
traceroute -A baidu.com           # 显示 AS 号
traceroute -4 -s 192.168.1.10 host
```

### 易踩坑

- **很多防火墙丢 UDP**，默认 UDP 模式常显示 `* * *`。换 `-I`（ICMP）或 `-T`（TCP SYN）往往能穿透。
- `* * *` **不代表故障**，可能只是某跳路由器不响应探测（常见于骨干网）。
- `-T` 探测 TCP SYN，目标端口要选开放端口（如 `-p 443`），否则到最后一跳也显示不通。
- 每跳默认发 **3 个包**（`-q 3`），输出里每行最多 3 个时延。
- `--mtu` 会显著变慢（逐跳试不同包长）。
- `-g`（源路由）在现代网络几乎全被禁用。
- 网络路径**动态变化**，两次 traceroute 结果不同是正常的。

### macOS 差异

macOS 自带 `traceroute`（BSD 版），默认用 **UDP**，选项子集不同：**没有 `-A`、`-D`、`-e`、`-M`、`-O`、`-P`、`-UL`、`--back`、`--fwmark`、`--mtu`、`--sport`**。macOS 的 `-I`/`-T` 可用。macOS 还有 `mtr` 需自行安装。

---

## 12. scp — 远程复制

基于 SSH 的文件复制（OpenSSH 9.0 起默认改用 SFTP 协议传输）。

### 基本语法

```
scp [-346ABCOpqRrsTv] [-c cipher] [-D sftp_server_path] [-F ssh_config]
    [-i identity_file] [-J destination] [-l limit] [-o ssh_option]
    [-P port] [-S program] [-X sftp_option] source ... target
```

### 选项（手册共 23 项，全列）

| 选项 | 说明 |
|---|---|
| `-3` | 两台远端之间经本地中转（默认直接互传） |
| `-4` | 只用 IPv4 |
| `-6` | 只用 IPv6 |
| `-A` | 允许转发 ssh-agent |
| `-B` | 批处理模式（不询问密码） |
| `-C` | 启用压缩 |
| `-c cipher` | 选择加密算法 |
| `-D sftp_server_path` | 直连本地 SFTP 服务器程序（调试用） |
| `-F ssh_config` | 指定 ssh 配置文件 |
| `-i identity_file` | 指定私钥 |
| `-J destination` | 经跳板机连接 |
| `-l limit` | 限速（Kbit/s） |
| `-O` | 用旧 SCP 协议（非 SFTP） |
| `-o ssh_option` | 传 ssh 配置项 |
| `-P port` | 远端端口（大写 P，因 `-p` 已被占用） |
| `-p` | 保留修改/访问时间与权限位 |
| `-q` | 安静模式 |
| `-R` | 两台远端之间由源主机发起连接执行 |
| `-r` | 递归复制目录（会跟随符号链接） |
| `-S program` | 指定加密连接程序 |
| `-T` | 关闭严格文件名检查 |
| `-v` | 详细调试输出 |
| `-X sftp_option` | 指定 SFTP 协议选项（`nrequests=`/`buffer=`…） |

### 示例

```bash
scp a.txt user@host:/tmp/                 # 上传单文件
scp user@host:/tmp/a.txt ./               # 下载单文件
scp -r dir/ user@host:/data/              # 递归上传目录
scp -P 2222 -i ~/.ssh/id_ed25519 a.txt user@host:/tmp/
scp -C -l 8000 big.iso user@host:/data/   # 压缩 + 限速 8Mbit/s
scp -J jump@bastion a.txt user@host:/tmp/ # 经跳板机
scp -3 host1:/a host2:/b                  # 两远端互传（经本地）
scp -O legacy@old-host:/f ./              # 兼容只支持 SCP 的老服务器
```

### 易踩坑

- **`-P` 是大写**（端口），小写 `-p` 是「保留时间戳」。这是最常见的笔误。
- OpenSSH 9.0+ **默认用 SFTP 协议**，某些老服务器不支持 SFTP 需加 `-O` 回退到 SCP 协议。
- `-r` 会**跟随符号链接**，可能把链接指向的大目录一并复制。
- 远端路径里的 `~` 在 `-O` 模式下对老服务器可能不展开，需写绝对路径。
- **`scp` 不支持断点续传**，大文件传输中断要重头来；需要续传请用 `rsync`。
- `scp` 通配符由**远端 shell** 展开（新 SFTP 模式下行为有变化），含空格文件名要加引号。
- 目标路径末尾有无 `/` 语义不同：`dir` 与 `dir/` 结果可能不一样。

### macOS 差异

macOS 自带 OpenSSH 版 scp，用法一致。新版 macOS 同样默认 SFTP 协议。

---

## 13. rsync — 增量同步

rsync 用「滚动校验和」算法只传差异部分，是备份与镜像的首选。手册共 **337 个选项**。

### 基本语法

```
rsync [OPTION]... SRC [SRC]... DEST
rsync [OPTION]... SRC [SRC]... [USER@]HOST:DEST       # 经 ssh
rsync [OPTION]... [USER@]HOST:SRC [DEST]
rsync [OPTION]... rsync://[USER@]HOST[:PORT]/SRC [DEST]  # daemon 模式
```

### 选项分组

#### 通用 / 输出

| 选项 | 说明 |
|---|---|
| `-v, --verbose` | 增加详细度 |
| `--info=FLAGS` | 细粒度信息详细度 |
| `--debug=FLAGS` | 细粒度调试详细度 |
| `--stderr=e\|a\|c` | stderr 输出模式 |
| `-q, --quiet` | 抑制非错误信息 |
| `--no-motd` | 抑制 daemon 模式 MOTD |
| `--stats` | 显示传输统计 |
| `-h, --human-readable` | 数字人类可读 |
| `--progress` | 显示进度 |
| `-P` | 等价 `--partial --progress` |
| `-i, --itemize-changes` | 输出每个更新的变更摘要 |
| `--out-format=FORMAT` | 自定义输出格式 |
| `--log-file=FILE` / `--log-file-format=FMT` | 日志 |
| `-8, --8-bit-output` | 高字节不转义 |
| `--outbuf=N\|L\|B` | 输出缓冲：无/行/块 |

#### 传输行为

| 选项 | 说明 |
|---|---|
| `-a, --archive` | 归档模式 = `-rlptgoD` |
| `-r, --recursive` | 递归目录 |
| `-R, --relative` | 使用相对路径名 |
| `--no-implied-dirs` | 不发送 `--relative` 隐含目录 |
| `-d, --dirs` | 传目录但不递归 |
| `--mkpath` | 创建目标缺失的路径组件 |
| `-n, --dry-run` | 演练，不做实际改动 |
| `-W, --whole-file` | 整文件复制（不用增量算法） |
| `-c, --checksum` | 按校验和而非时间+大小判断 |
| `-u, --update` | 跳过接收端更新的文件 |
| `--inplace` | 原地更新目标文件 |
| `--append` | 追加到较短文件 |
| `--append-verify` | `--append` + 校验已有数据 |
| `-I, --ignore-times` | 不跳过大小时间相同的文件 |
| `--size-only` | 只按大小判断 |
| `--modify-window=NUM` | 修改时间比较精度 |
| `-x, --one-file-system` | 不跨文件系统 |
| `-B, --block-size=SIZE` | 固定校验块大小 |
| `--max-size=SIZE` / `--min-size=SIZE` | 大小过滤 |
| `--partial` / `--partial-dir=DIR` | 保留部分传输的文件 |
| `--delay-updates` | 全部更新最后落位 |
| `--timeout=SECONDS` | I/O 超时 |
| `--contimeout=SECONDS` | daemon 连接超时 |
| `--stop-after=MINS` / `--stop-at=y-m-dTh:m` | 定时停止 |
| `--fsync` | 每个文件写后 fsync |
| `--sparse, -S` | 把连续 0 转成稀疏块 |
| `--preallocate` | 写前预分配目标文件 |
| `--bwlimit=RATE` | 限制带宽 |
| `--max-alloc=SIZE` | 内存分配上限 |
| `--protocol=NUM` | 强制旧协议版本 |

#### 权限 / 属性

| 选项 | 说明 |
|---|---|
| `-p, --perms` | 保留权限 |
| `-E, --executability` | 保留可执行位 |
| `--chmod=CHMOD` | 调整权限 |
| `-t, --times` | 保留修改时间 |
| `-U, --atimes` | 保留访问时间 |
| `-N, --crtimes` | 保留创建时间 |
| `-O, --omit-dir-times` | 目录不保留时间 |
| `-J, --omit-link-times` | 符号链接不保留时间 |
| `--open-noatime` | 打开文件不改 atime |
| `-o, --owner` | 保留属主（需 root） |
| `-g, --group` | 保留属组 |
| `--devices` / `--specials` / `-D` | 保留设备/特殊文件（`-D` = 两者） |
| `--copy-devices` / `--write-devices` | 复制/写入设备内容 |
| `-A, --acls` | 保留 ACL（隐含 `--perms`） |
| `-X, --xattrs` | 保留扩展属性 |
| `--super` / `--fake-super` | 超级用户活动 / 用 xattr 模拟特权属性 |
| `--numeric-ids` | 不做 uid/gid 名称映射 |
| `--usermap` / `--groupmap` / `--chown=USER:GROUP` | 用户/组映射 |
| `--copy-as=USER[:GROUP]` | 以指定用户/组复制 |

#### 链接处理

| 选项 | 说明 |
|---|---|
| `-l, --links` | 符号链接当链接复制 |
| `-L, --copy-links` | 符号链接转成引用目标 |
| `--copy-unsafe-links` | 只转换「不安全」链接 |
| `--safe-links` | 忽略指向树外的链接 |
| `--munge-links` | 破坏链接使其不可用（安全） |
| `-k, --copy-dirlinks` | 指向目录的链接转成目录 |
| `-K, --keep-dirlinks` | 接收端符号链接目录当目录对待 |
| `-H, --hard-links` | 保留硬链接 |

#### 备份 / 删除 / 比较

| 选项 | 说明 |
|---|---|
| `-b, --backup` | 备份被覆盖文件 |
| `--backup-dir=DIR` | 备份到 DIR 层级 |
| `--suffix=SUFFIX` | 备份后缀（默认 `~`） |
| `--delete` | **删除目标端多余文件** |
| `--del` | `--delete-during` 的别名 |
| `--delete-before` / `--delete-during` / `--delete-delay` / `--delete-after` | 删除时机 |
| `--delete-excluded` | 连被排除的文件也删 |
| `--max-delete=NUM` | 最多删 NUM 个 |
| `--force` | 强制删除非空目录 |
| `--ignore-errors` | 即使有 I/O 错误也删 |
| `--ignore-missing-args` / `--delete-missing-args` | 忽略/删除缺失的源参数 |
| `--existing` / `--ignore-existing` | 跳过新建 / 跳过已存在 |
| `--remove-source-files` | 同步后删除源文件 |
| `--prune-empty-dirs, -m` | 剪掉空目录链 |
| `--compare-dest=DIR` / `--copy-dest=DIR` / `--link-dest=DIR` | 与 DIR 比较 / 复制 / 硬链接 |
| `--fuzzy, -y` | 找相似文件作为基准 |

#### 过滤 / 压缩 / 远程

| 选项 | 说明 |
|---|---|
| `-z, --compress` | 传输时压缩 |
| `--compress-choice=STR` | 压缩算法 |
| `--compress-level=NUM` / `--compress-threads=NUM` | 压缩级别/线程 |
| `--skip-compress=LIST` | 跳过后缀列表 |
| `-C, --cvs-exclude` | 用 CVS 规则自动忽略 |
| `-f, --filter=RULE` | 加过滤规则 |
| `-F` | 等价 `--filter='dir-merge /.rsync-filter'`（可重复） |
| `--exclude=PATTERN` / `--exclude-from=FILE` | 排除 |
| `--include=PATTERN` / `--include-from=FILE` | 包含 |
| `--files-from=FILE` | 从文件读源文件列表 |
| `-0, --from0` | `*-from`/filter 文件用 NUL 分隔 |
| `-e, --rsh=COMMAND` | 指定远程 shell |
| `--rsync-path=PROGRAM` | 指定远端 rsync 程序 |
| `--old-args` / `-s, --secluded-args` / `--trust-sender` | 参数传递安全控制 |
| `--blocking-io` | 远程 shell 用阻塞 I/O |
| `--address=ADDRESS` / `--port=PORT` / `--sockopts=OPTIONS` | 绑定地址 / 端口 / TCP 选项 |
| `-M, --remote-option=OPT` | 只发给远端的选项 |
| `--password-file=FILE` | daemon 密码文件 |
| `--early-input=FILE` | daemon early exec 输入 |
| `--list-only` | 只列文件不复制 |
| `--write-batch=FILE` / `--only-write-batch=FILE` / `--read-batch=FILE` | 批处理更新 |
| `--iconv=CONVERT_SPEC` | 文件名字符集转换 |
| `--checksum-seed=NUM` | 校验种子 |
| `-4, --ipv4` / `-6, --ipv6` | 优先 IPv4 / IPv6 |
| `--daemon` | 以 daemon 模式运行 |

### 示例

```bash
rsync -av src/ dst/                        # 本地同步（注意结尾斜杠）
rsync -avz src/ user@host:/data/           # 经 ssh 同步并压缩
rsync -avz --delete src/ user@host:/data/  # 目标端删除多余文件（危险）
rsync -avzn --delete src/ host:/data/      # 先演练（-n）确认删什么
rsync -avP big.iso host:/data/             # 带进度、可续传
rsync -av --exclude='*.log' --exclude='.git/' src/ dst/
rsync -av --link-dest=/backup/last /src/ /backup/now/   # 硬链接增量备份
rsync -av --bwlimit=5000 --timeout=60 src/ host:/data/
rsync -av --files-from=list.txt ./ host:/data/
rsync -av -e 'ssh -p 2222' src/ host:/data/
rsync -av --remove-source-files src/ host:/data/        # 传完删源
```

### 易踩坑

- **结尾斜杠决定语义**：`rsync -a src dst` 会把 `src` **目录本身**放进 `dst`（变成 `dst/src/...`）；`rsync -a src/ dst` 才是把 `src` **内容**同步到 `dst`。这是最经典的坑。
- **`--delete` 是 destructive**：会把目标端「源里没有的文件」删掉。**用 `--delete` 前必须先用 `-n`（dry-run）确认**。方向写反（`rsync -a --delete /empty/ /data/`）会清空数据。
- `-a` 隐含 `-rlptgoD`，但**不含 `-A`/`-X`/`-U`/`-N`/`-H`**（ACL、xattr、atime、crtime、硬链接需显式加）。
- `-z` 压缩在**快速局域网内反而拖慢**（CPU 成为瓶颈），仅在慢速链路上划算。
- 远端源路径含通配符时，**由远端 shell 展开**；`--files-from` 则由 rsync 自己处理，语义不同。
- `--partial`（或 `-P`）才会保留中断的部分文件，默认中断会删掉。
- 目标为 FAT/exFAT 等不支持权限的文件系统时，`-p` 会失败或告警，可加 `--no-perms`。
- `--inplace` 在传输中断时会留下损坏的目标文件，除非配合 `--partial`。

### macOS 差异

**macOS 自带的是 rsync 2.6.9（2006 年）**，缺少 `--info`、`--debug`、`-N`(crtimes)、`--mkpath`、`--stop-after` 等大量新选项，且**不支持 `-X`/`-A` 的现代语义**。备份关键数据建议 `brew install rsync` 并调用 `/opt/homebrew/bin/rsync`。此外 macOS 文件系统默认大小写不敏感，`--delete` 时要注意大小写差异可能误删。

---

## 14. ssh — 远程登录 / 隧道

OpenSSH 客户端。手册共 **52 项**。

### 基本语法

```
ssh [-46AaCfGgKkMNnqsTtVvXxYy] [-B bind_interface] [-b bind_address]
    [-c cipher_spec] [-D [bind_address:]port] [-E log_file] [-e escape_char]
    [-F configfile] [-I pkcs11] [-i identity_file] [-J destination]
    [-L address] [-l login_name] [-m mac_spec] [-O ctl_cmd] [-o option]
    [-P tag] [-p port] [-Q query_option] [-R address] [-S ctl_path]
    [-W host:port] [-w local_tun[:remote_tun]] destination [command]
```

### 选项（手册共 52 项，全列）

| 选项 | 说明 |
|---|---|
| `-4` | 只用 IPv4 |
| `-6` | 只用 IPv6 |
| `-A` | 启用 ssh-agent 转发（有安全风险） |
| `-a` | 禁用 ssh-agent 转发 |
| `-B bind_interface` | 连接前绑定到指定接口地址 |
| `-b bind_address` | 用本机指定源地址 |
| `-C` | 请求压缩 |
| `-c cipher_spec` | 加密算法列表 |
| `-D [bind_address:]port` | 动态端口转发（本地 SOCKS 代理） |
| `-E log_file` | 调试日志写到文件 |
| `-e escape_char` | 转义字符（默认 `~`） |
| `-F configfile` | 指定配置文件 |
| `-f` | 执行命令前转后台（隐含 `-n`） |
| `-G` | 打印求值后的配置并退出 |
| `-g` | 允许远端连本地转发端口 |
| `-I pkcs11` | 指定 PKCS#11 共享库 |
| `-i identity_file` | 指定私钥 |
| `-J destination` | 经跳板机（ProxyJump） |
| `-K` | 启用 GSSAPI 认证与凭据转发 |
| `-k` | 禁用 GSSAPI 凭据转发 |
| `-L [bind:]port:host:hostport` | 本地端口转发 |
| `-l login_name` | 远端登录用户名 |
| `-M` | master 模式（连接复用） |
| `-m mac_spec` | MAC 算法列表 |
| `-N` | 不执行远端命令（仅转发） |
| `-n` | 把 stdin 重定向到 /dev/null |
| `-O ctl_cmd` | 控制复用 master（`check`/`forward`/`cancel`/`proxy`/`exit`） |
| `-o option` | 传配置项 |
| `-P tag` | 指定 tag 名 |
| `-p port` | 远端端口 |
| `-Q query_option` | 查询支持的算法（`cipher`/`mac`/`kex`/`key`…） |
| `-q` | 安静模式 |
| `-R [bind:]port:host:hostport` | 远程端口转发 |
| `-S ctl_path` | 连接复用控制 socket 路径 |
| `-s` | 请求子系统（如 `sftp`） |
| `-T` | 禁用伪终端分配 |
| `-t` | 强制分配伪终端（可重复） |
| `-V` | 显示版本 |
| `-v` | 详细调试（可重复，最多 3 次） |
| `-W host:port` | 把 stdin/stdout 转发到 host:port（隐含 `-N -T`） |
| `-w local_tun[:remote_tun]` | 隧道设备转发（tun） |
| `-X` | 启用 X11 转发 |
| `-x` | 禁用 X11 转发 |
| `-Y` | 启用可信 X11 转发 |
| `-y` | 日志走 syslog |

### 端口转发三种模式

| 模式 | 选项 | 用途 |
|---|---|---|
| 本地转发 | `-L 8080:localhost:80` | 把本地 8080 转到远端的 80 |
| 远程转发 | `-R 8080:localhost:80` | 把远端 8080 转到本地 80 |
| 动态转发 | `-D 1080` | 本地 SOCKS5 代理，流量走远端出 |

### 示例

```bash
ssh user@host                       # 登录
ssh -p 2222 user@host               # 指定端口
ssh -i ~/.ssh/id_ed25519 user@host  # 指定私钥
ssh user@host 'uptime; df -h'       # 执行远程命令
ssh -J jump@bastion user@host       # 经跳板机
ssh -L 8080:db.internal:3306 user@host   # 本地转发（隧道访问内网 DB）
ssh -R 9090:localhost:3000 user@host     # 远程转发
ssh -D 1080 user@host                    # SOCKS5 动态代理
ssh -N -L 8080:localhost:80 user@host    # 只转发不开 shell
ssh -o StrictHostKeyChecking=no user@host
ssh -v user@host                    # 调试
ssh -Q cipher                       # 查支持的加密算法
ssh -G user@host                    # 查看生效配置
```

### 易踩坑

- **`-L` 默认只监听 127.0.0.1**，要让别人连需 `-g` 或写 `-L 0.0.0.0:8080:...`（有暴露风险）。
- `-R` 远程转发默认也只绑远端回环，要让外部访问需远端 `sshd_config` 里 `GatewayPorts yes`。
- **`-A`（agent 转发）有风险**：远端 root 可通过转发的 socket 借用你的密钥，别转发到不可信主机。
- `-D` 动态转发**不加密出口流量**，只是让流量从远端出去，SOCKS 本身不认证。
- `-N` 与 `-f` 常配合做后台隧道，但 `-f` 要求能非交互认证（否则会卡在密码提示）。
- 连接复用（`-M`/`-S`/`ControlMaster`）能大幅加速多次连接，但控制 socket 权限要设好，否则等于给了别人免密通道。
- **`StrictHostKeyChecking=no`** 会跳过主机指纹校验，削弱中间人防护，仅测试环境用。
- 默认 `~/.ssh/known_hosts` 记录指纹，服务器重装后指纹变了会拒绝连接，需 `ssh-keygen -R host` 清除旧记录。

### macOS 差异

macOS 自带 OpenSSH 客户端（较新版本）。用法一致。注意 macOS 的 ssh-agent 集成方式不同（需 `ssh-add --apple-use-keychain`）。

---

## 附录 A：网络排障套路速查

| 现象 | 排查顺序 |
|---|---|
| 服务连不上 | `ping` 通不通 → `nc -zv host port` 端口开没开 → `ss -tlnp` 本地监听没 → `curl -v` 应用层报什么 |
| 域名解析异常 | `dig +short name` → `dig @8.8.8.8 name`（换 DNS 对比）→ `dig +trace name`（逐级）→ 查 `/etc/resolv.conf` |
| 连接慢/丢包 | `ping -c 100`（看丢包率与抖动）→ `traceroute -T -p 443`（看哪一跳丢）→ `mtr`（持续） |
| 端口占用 | `ss -tlnp \| grep :8080` → `lsof -i:8080` |
| 连接数暴增 | `ss -s` → `ss -tn state time-wait \| wc -l` → `ss -tnp \| head` |
| 抓包分析 | `tcpdump -i any -nn -w cap.pcap` → `tcpdump -r cap.pcap -nn -A` |
| 隧道/跳板 | `ssh -J jump host`（跳板）→ `ssh -L`（本地转发）→ `ssh -D`（SOCKS） |
| 大文件传输 | `rsync -avP`（可续传）> `scp`（不可续传） |

## 附录 B：危险操作清单

| 命令 | 危险点 | 安全做法 |
|---|---|---|
| `ip addr flush dev eth0` | 清空网卡地址，远程必断 | 先确认有带外/控制台通道 |
| `ip link set eth0 down` | 关闭网卡，远程必断 | 同上 |
| `ip route del default` | 删默认路由，外网全断 | 先记录原路由 |
| `nc -z host 1-65535` | 全端口扫描，触发 IDS | 仅对自有资产扫描 |
| `nc -l 0.0.0.0 1234` | 对外监听，可能被利用 | 绑定回环或加防火墙 |
| `rsync --delete src/ dst/` | 删除目标端多余文件 | **先 `-n` 演练**，确认方向 |
| `rsync -a --delete /empty/ /data/` | 清空 /data | 永远先 dry-run |
| `scp -r` 覆盖远端 | 覆盖远端同名文件 | 先确认目标路径 |
| `ssh -A user@untrusted` | agent 转发被滥用 | 不转发到不可信主机 |
| `ssh -L 0.0.0.0:port:...` | 隧道端口暴露公网 | 默认绑 127.0.0.1 |
| `curl -k` | 跳过证书校验，易被 MITM | 修根因，勿长期 `-k` |
| `wget -m` 无限制 | 可能拖垮目标站 | 加 `-np`/`-A`/`--limit-rate` |
| `tcpdump -w` 不退出 | 文件写满磁盘 | 加 `-c`/`-C`/`-W` |
| `kill -9`（对 sshd） | 掐断自己的登录会话 | 确认不是当前会话 |

## 附录 C：macOS ↔ Linux 命令对照

| 用途 | Linux | macOS |
|---|---|---|
| 查看 IP | `ip addr` | `ifconfig` |
| 查看路由 | `ip route` / `ip -s link` | `netstat -rn` |
| 查看 ARP | `ip neigh` | `arp -an` |
| 查看端口/连接 | `ss -tulnp` | `netstat -an` / `lsof -nP -iTCP -sTCP:LISTEN` |
| 谁占用端口 | `ss -tlnp` / `lsof -i:PORT` | `lsof -nP -i:PORT` |
| DNS 查询 | `dig` / `nslookup` | `dig` / `nslookup` / `dscacheutil` / `scutil --dns` |
| 抓包 | `tcpdump -i eth0` | `tcpdump -i en0` |
| 路由追踪 | `traceroute` / `mtr` | `traceroute`（需装 mtr） |
| 下载 | `wget` | `curl -O`（无 wget） |
| 端口测试 | `nc -zv` / `telnet` | `nc -zv` / `nc -G` |
| 增量同步 | `rsync`（新版） | `rsync`（2.6.9，建议 brew 装新版） |
| 服务管理 | `systemctl` | `launchctl` / `brew services` |
