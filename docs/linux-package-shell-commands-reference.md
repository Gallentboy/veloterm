# Linux 包管理 / 权限用户 / 归档压缩 / Shell 与环境 命令参考

> 覆盖 `apt-apt-get / yum-dnf / rpm / dpkg / chmod / chown / sudo / su / useradd-passwd / tar / gzip-gunzip / zip-unzip / xz / echo / export / env / which-whereis / history / alias / crontab / date / watch / xargs / tee / screen-tmux` 共 **25 条命令**。
>
> **来源**：apt 2.x（apt/apt-get）、DNF 4.x（dnf/yum，man7 上的 `yum.8` 已改为「redirecting to DNF Command Reference」）、rpm 4.x（112 项）、dpkg 1.21+（58 项）、GNU coreutils 9.11（chmod/chown/echo/env/date/tee）、shadow-utils 4.x（useradd/passwd）、sudo 1.9.x、util-linux 2.40（su）、GNU tar 1.35（157 项）、GNU gzip 1.13、Info-ZIP 3.0（zip 181 项 / unzip 45 项）、xz 5.4+（54 项）、GNU findutils 4.9（xargs）、procps-ng（watch）、cronie（crontab/crontab.5）、GNU Bash 5.x（export/history/alias 为 shell 内建）、GNU screen 4.x、tmux 3.x（152 个子命令）。
>
> **BSD/macOS 差异在每节末尾单列**。本批 macOS 差异极大：**macOS 没有 apt/yum/dnf/rpm/dpkg**（用 Homebrew），**没有 `useradd`**（用 `dscl`/`sysadminctl`），**`tar` 是 bsdtar 而非 GNU tar**，**`date` 是 BSD 版**（`-d` 语义完全不同）。
>
> 不确定的地方一律查了手册，未凭记忆推断。

## 速查表

| 命令 | 作用 | 危险级别 | 本参考中的参数模板 |
|---|---|---|---|
| `apt` / `apt-get` | Debian/Ubuntu 包管理 | **confirm** | `apt {{command}} {{packages}}` |
| `yum` / `dnf` | RHEL/CentOS 包管理 | **confirm** | `dnf {{command}} {{packages}}` |
| `rpm` | RPM 包查询/安装 | **confirm** | `rpm {{options}} {{package}}` |
| `dpkg` | Debian 包查询/安装 | **confirm** | `dpkg {{options}} {{package}}` |
| `chmod` | 改权限 | **confirm** | `chmod {{options}} {{mode}} {{file}}` |
| `chown` | 改属主/属组 | **confirm** | `chown {{options}} {{owner}} {{file}}` |
| `sudo` | 以其他身份执行 | **confirm** | `sudo {{options}} {{command}}` |
| `su` | 切换用户 | **confirm** | `su {{options}} {{user}}` |
| `useradd` / `passwd` | 用户管理 | **confirm** | `useradd {{options}} {{user}}` |
| `tar` | 打包/解包 | **confirm** | `tar {{options}} {{file}}` |
| `gzip` / `gunzip` | gz 压缩/解压 | **confirm** | `gzip {{options}} {{file}}` |
| `zip` / `unzip` | zip 压缩/解压 | **confirm** | `zip {{options}} {{archive}} {{files}}` |
| `xz` | xz 压缩/解压 | **confirm** | `xz {{options}} {{file}}` |
| `echo` | 输出 | safe | `echo {{options}} {{string}}` |
| `export` | 设/导出环境变量 | safe | `export {{name}}={{value}}` |
| `env` | 查看/临时改环境 | safe | `env {{options}} {{name}}={{value}} {{command}}` |
| `which` / `whereis` | 查命令路径 | safe | `which {{options}} {{command}}` |
| `history` | 历史命令 | safe | `history {{options}}` |
| `alias` | 别名 | safe | `alias {{name}}='{{command}}'` |
| `crontab` | 定时任务 | **confirm** | `crontab {{options}}` |
| `date` | 时间 | safe | `date {{options}} {{format}}` |
| `watch` | 定时执行 | safe | `watch {{options}} {{command}}` |
| `xargs` | 参数传递 | **confirm** | `xargs {{options}} {{command}}` |
| `tee` | 分流输出 | **confirm** | `tee {{options}} {{file}}` |
| `screen` / `tmux` | 会话保持 | safe | `tmux {{options}} {{command}}` |

> 本批 **confirm 占绝大多数（18/25）**，因为包管理、权限、归档、定时任务都属「会改变系统状态」的操作。**没有标 destructive** 的，但下列子操作实为 destructive，已在各节标注：
> `rm` 类语义的 `apt purge`、`dnf remove`、`rpm -e`、`dpkg -P`；`chmod -R 777 /`、`chown -R` 到错误路径；`crontab -r`（无确认删除）；`tar -x` 覆盖；`xargs rm`；`tee` 覆盖；`> file` 重定向截断。
> `sudo` / `su` 本身不破坏数据，但**提权后一条命令的破坏力被放大**，故标 confirm。

---

## 1. apt / apt-get — Debian/Ubuntu 包管理

`apt` 是面向终端用户的**高层**前端（有彩色输出与进度条，**输出格式随版本变化，不适合脚本**）；`apt-get` 是**低层**、输出稳定、**适合脚本**的老牌工具。二者共享同一套包数据库。

### 基本语法

```
apt     [-h] [-o=config] [-c=config] [-t=release] [-a=arch] {command} ...
apt-get [-h] [-o=config] [-c=config] [-t=release] [-a=arch] {command} ...
```

### 子命令

| 子命令 | 说明 |
|---|---|
| `update` | 从配置的源下载包索引（**不改动已装包**） |
| `upgrade` | 升级所有已装包；**不删除**任何包，需删包时该包跳过 |
| `full-upgrade`（apt-get 里叫 `dist-upgrade`） | 升级并可**移除**阻碍整体升级的包 |
| `install` | 安装/升级指定包 |
| `reinstall` | 重装已装且已是最新的包 |
| `remove` | 卸载包，**保留**用户修改过的配置文件 |
| `purge` | 卸载并**连配置一起删** |
| `autoremove`（别名 `auto-remove`） | 删除不再被依赖的自动安装包 |
| `satisfy` | 满足依赖表达式（同 Build-Depends 语法） |
| `why` / `why-not` | 解释某包为何被装 / 为何装不上 |
| `search` | 按正则搜索可用包 |
| `show` | 显示包详情（依赖、大小、描述） |
| `list` | 列包（`--installed`/`--upgradable`/`--all-versions`） |
| `edit-sources` | 用编辑器改 `sources.list`（带基本校验） |
| `source` | 下载源码包 |
| `build-dep` | 安装构建依赖 |
| `download` | 只下载 .deb 不安装 |
| `changelog` | 查看包变更日志 |
| `clean` | 清空 `/var/cache/apt/archives` |
| `autoclean`（别名 `auto-clean`） | 只清过期的 .deb |
| `check` | 检查依赖完整性 |
| `indextargets` | 列出索引目标 |
| `dselect-upgrade` | 按 dselect 选择升级 |
| `showsrc` / `depends` / `rdepends` / `policy` | 见 apt-cache(8) |

### 常用选项

| 选项 | 说明 |
|---|---|
| `-y, --yes, --assume-yes` | 所有提示自动答 yes（**脚本必用**） |
| `--assume-no` | 所有提示自动答 no |
| `-d, --download-only` | 只下载不安装 |
| `-f, --fix-broken` | 尝试修复依赖损坏的系统 |
| `-m, --ignore-missing, --fix-missing` | 忽略取不到的包 |
| `--no-download` | 禁止下载（配合 `--ignore-missing` 只用本地缓存） |
| `-q, --quiet` | 安静（`-qq` 更安静，适合日志） |
| `-s, --simulate, --dry-run, --just-print, --no-act` | **演练**，不做实际改动 |
| `-V, --verbose-versions` | 显示完整版本号 |
| `-t, --target-release, --default-release` | 指定默认发行版/固定优先级 |
| `-a, --host-architecture` | 指定架构（源码编译/交叉构建） |
| `-P, --build-profiles` | 指定构建 profile |
| `-b, --compile, --build` | 下载后编译源码包 |
| `-S, --snapshot` | 选择快照仓库（如 `-S 20220102T030405Z`） |
| `-U, --update` | 执行指定命令前先跑 `update` |
| `--no-install-recommends` | 不把推荐包当依赖安装 |
| `--install-suggests` | 把建议包当依赖安装 |
| `--no-show-upgraded` | 不显示将要升级的包列表 |
| `--no-list-columns` | 包列表不分栏 |
| `--comment` | 给 `history.log` 加 Comment 字段 |
| `--ignore-hold` | 忽略包的 hold |
| `--with-new-pkgs` | 配合 upgrade 允许装新包 |
| `--no-upgrade` | 配合 install 不升级命令行上的包 |
| `--only-upgrade` | 只升级已装的，不装新的 |
| `--allow-downgrades` | **危险**：允许降级且不提示 |
| `--allow-remove-essential` | **危险**：允许删 essential 包且不提示 |
| `--allow-change-held-packages` | **危险**：允许改被 hold 的包且不提示 |
| `--force-yes` | **危险且已废弃**：任何潜在有害操作都不提示 |
| `--print-uris` | 只打印 URI 不下载 |
| `--purge` | 把本要 remove 的改为 purge |
| `--reinstall` | 重装 |
| `--trivial-only` | 只做「显然安全」的操作 |
| `--mark-auto` | 装完把新装包标为自动安装 |
| `--no-remove` | 若有包将被移除则立即中止 |
| `--auto-remove, --autoremove` | 顺带自动清理 |
| `--only-source` | 源码名不映射到二进制表 |
| `--diff-only` / `--dsc-only` / `--tar-only` | 源码包只下指定部分 |
| `--arch-only` / `--indep-only` | 只处理架构相关/无关的构建依赖 |
| `--allow-unauthenticated` | 允许未认证的包且不提示 |
| `--allow-insecure-repositories` | 允许 update 取不可验证的数据 |
| `--allow-releaseinfo-change` | 允许源改了发行信息后继续 |
| `--show-progress` | 显示友好进度 |
| `--with-source=FILE` | 把某文件加为元数据源 |
| `-eany, --error-on=any` | update 时任何错误都失败 |
| `-o, --option` | 设置任意配置项（`-o Foo::Bar=bar`） |
| `-c, --config-file` | 指定配置文件 |
| `--cli-version` | 请求特定 CLI 版本（MAJOR.MINOR） |
| `--no-color` / `--color` | 开关彩色 |
| `--audit` | 显示 audit/notice 消息 |
| `-h, --help` / `-v, --version` | 帮助 / 版本 |

### 版本/发行版选择语法

```bash
apt install nginx=1.24.0-1        # 指定版本（=）
apt install nginx/stable          # 指定发行版/代号（/）
apt install pkg+                  # 强制安装
apt remove pkg-                   # 强制移除
apt install ./local.deb           # 安装本地 deb（路径带 ./）
```

### 示例

```bash
sudo apt update && sudo apt upgrade -y
sudo apt install -y nginx curl
sudo apt install --no-install-recommends -y nginx
sudo apt full-upgrade -y                      # 可移除包的完整升级
sudo apt remove nginx && sudo apt purge nginx
sudo apt autoremove --purge -y
apt list --upgradable                          # 看有哪些可升级
apt list --installed | grep nginx
apt search '^nginx' && apt show nginx
apt-get -s dist-upgrade                        # 演练完整升级
apt-get install -y --only-upgrade nginx
apt-cache policy nginx                         # 看候选版本与来源
```

### 易踩坑

- **`apt` 的输出格式不保证稳定**（官方明说 CLI 面向终端用户、行为可能随版本变），**脚本里请用 `apt-get`**。
- **`apt upgrade` 不会删包**，需要删包才能升级的会被跳过；要完整升级用 `apt full-upgrade`（= `apt-get dist-upgrade`）。**full-upgrade 可能移除包，跑之前先 `-s` 演练**。
- `update` 只是刷新索引，**不会升级任何包**；很多人以为 `apt update` 就是升级。
- `remove` 保留配置文件，`purge` 才删干净。反复 `install`/`remove` 会留下残留配置。
- `autoremove` 会删掉「曾被当依赖装上、现在没人依赖」的包，**可能包含你后来喜欢上的工具**；用 `apt-mark manual <pkg>` 可保命。
- `apt install ./foo.deb` 才会走本地安装；写成 `apt install foo.deb` 会被当成包名去仓库找。
- `--force-yes` 已废弃，改用 `--allow-downgrades` / `--allow-remove-essential` / `--allow-change-held-packages` 三个更细的开关，且都**危险**。
- 包锁冲突（`Could not get lock /var/lib/dpkg/lock-frontend`）说明有另一个 apt/dpkg 在跑或上次中断，**不要暴力删锁文件**，先确认进程。

### macOS 差异

**macOS 没有 apt/apt-get**。对应工具是 **Homebrew**（`brew install/upgrade/uninstall`）或 MacPorts。若在 macOS 上用 Docker/UTM 里的 Debian 容器，则与 Linux 一致。

---

## 2. yum / dnf — RHEL/CentOS 包管理

`dnf` 是 `yum` 的下一代实现，**RHEL 8+ / Fedora 里 `yum` 就是 `dnf` 的软链接**（man7.org 上 `yum.8` 的内容已变成「redirecting to DNF Command Reference」）。二者 CLI 高度兼容。

### 基本语法

```
dnf [options] <command> [<args>...]
```

### 子命令（30 个，全列）

| 子命令 | 说明 |
|---|---|
| `install` | 安装包 |
| `reinstall` | 重装 |
| `remove` | 卸载 |
| `upgrade` | 升级（可删包以完成升级） |
| `upgrade-minimal` | 只升级到最小必要版本 |
| `update` | `upgrade` 的别名 |
| `downgrade` | 降级 |
| `distro-sync` | 把已装包同步到仓库版本 |
| `swap` | 用一个包换掉另一个 |
| `autoremove` | 删除不再需要的依赖 |
| `check-update` | 检查是否有更新（退出码 100 表示有） |
| `check` | 检查依赖问题 |
| `info` | 显示包信息 |
| `list` | 列包 |
| `search` | 搜索 |
| `provides` | 查哪个包提供某文件/能力 |
| `deplist` | 列出依赖 |
| `repoquery` | 高级查询（大量子选项） |
| `repolist` | 列出仓库 |
| `repoinfo` | 仓库详情 |
| `repository-packages` | 按仓库操作包 |
| `group` | 包组管理 |
| `module` | 模块流管理 |
| `history` | 事务历史（可 undo/rollback） |
| `mark` | 标记包为 install/remove |
| `clean` | 清理缓存 |
| `makecache` | 预生成缓存 |
| `updateinfo` | 安全/缺陷公告 |
| `alias` | 命令别名 |
| `shell` | 交互 shell |
| `help` | 帮助 |

### 常用选项

| 选项 | 说明 |
|---|---|
| `-y, --assumeyes` | 全部自动答 yes |
| `--assumeno` | 全部自动答 no |
| `-q, --quiet` | 安静 |
| `-v, --verbose` | 详细 |
| `-C, --cacheonly` | 只用系统缓存，不更新 |
| `--refresh` | 先过期缓存再操作 |
| `--nogpgcheck` | 跳过 GPG 校验（**降低安全性**） |
| `--skip-broken` | 跳过依赖坏掉的包 |
| `--allowerasing` | 允许删除已装包以解依赖 |
| `-b, --best` / `--nobest` | 只用最优版本 / 允许次优 |
| `--downloadonly` | 只下载 |
| `--downloaddir=PATH, --destdir=PATH` | 指定下载目录 |
| `--setopt=OPT=VALUE` | 覆盖任意配置项 |
| `--enablerepo=REPOID` / `--disablerepo=REPOID` | 启用/禁用仓库 |
| `--enableplugin` / `--disableplugin` | 启用/禁用插件 |
| `-x, --exclude=SPEC` / `--excludepkgs=SPEC` | 排除包 |
| `--disableexcludes=[all\|main\|REPOID]` | 关闭排除规则 |
| `--installroot=PATH` | 装到指定根（做 chroot 镜像） |
| `--releasever=RELEASE` | 指定发行版版本 |
| `--showduplicates` | 显示所有版本 |
| `--security` | 只含安全更新 |
| `--bugfix` | 只含缺陷修复 |
| `--enhancement` / `--newpackage` | 只含增强/新包 |
| `--advisory=ID, --advisories=ID` | 按公告 ID |
| `--cve=CVES, --cves=CVES` | 按 CVE |
| `--sec-severity=SEV, --secseverity=SEV` | 按安全等级 |
| `--bz=BUGZILLA, --bzs=BUGZILLA` | 按 Bugzilla ID |
| `--obsoletes` | 处理过时包 |
| `--nodocs` | 不装文档 |
| `--noautoremove` | 不自动清理依赖 |
| `-4` / `-6` | 只用 IPv4 / IPv6 |
| `--forcearch=ARCH` | 强制架构 |
| `--comment=COMMENT` | 给事务加注释 |
| `-c FILE, --config=FILE` | 指定配置文件 |
| `-d N, --debuglevel=N` / `-e N, --errorlevel=N` | 调试/错误级别 |
| `--debugsolver` | 输出求解器详细过程 |
| `--noplugins` | 不加载插件 |
| `--repofrompath=REPO,PATH` | 临时加仓库 |
| `--repo=REPOID, --repoid=REPOID` | 只启用指定仓库 |
| `--rpmverbosity=NAME` | rpm 输出级别 |
| `-R MIN, --randomwait=MIN` | 随机等待上限（分钟） |
| `--disable` / `--enable, --set-disabled, --set-enabled` | 禁用/启用仓库（持久） |
| `--version` / `-h, --help, --help-cmd` | 版本 / 帮助 |

`list` / `repoquery` 的过滤选项：`-a/--all`、`--available`、`--installed`、`--upgrades`、`--extras`、`--duplicates`、`--unneeded`、`--userinstalled`、`--recent`、`--latest-limit N`、`-f/--file FILE`、`--arch`、`--whatprovides/--whatrequires/--whatdepends/--whatconflicts/--whatobsoletes/--whatrecommends/--whatsuggests/--whatsupplements/--whatenhances`、`--alldeps`、`--exactdeps`、`--srpm`、`-i/--info`、`-l/--list`、`--changelogs`、`--depends`、`--requires`、`--provides`、`--obsoletes`、`--location`、`--tree`、`--nvr`、`--nevra`、`--envra`、`--qf/--queryformat`、`--recursive`、`--resolve`。

### 示例

```bash
sudo dnf install -y nginx
sudo dnf upgrade --refresh -y
sudo dnf remove nginx
sudo dnf search nginx
dnf list installed | grep nginx
dnf provides /usr/sbin/nginx
sudo dnf group install -y 'Development Tools'
sudo dnf history                              # 事务历史
sudo dnf history undo last                    # 撤销上一次事务
sudo dnf repoquery --requires nginx
sudo dnf updateinfo list security
sudo dnf install --downloadonly --downloaddir=/tmp/pkgs nginx
sudo dnf distro-sync -y
```

### 易踩坑

- **RHEL 8+ 上 `yum` 就是 `dnf`**，写 `yum` 能跑但实际执行的是 dnf；新脚本建议直接写 `dnf`。
- `dnf upgrade` 与 `apt upgrade` 语义**不同**：dnf 的 upgrade **允许移除阻碍升级的包**（类似 apt 的 full-upgrade），而 apt upgrade 不删包。
- `--skip-broken` 会让升级「部分成功」，容易留下不一致状态，非必要不用。
- `--nogpgcheck` 跳过签名校验，**不要在生产用**。
- `dnf history undo` 只对**能安全回滚**的事务有效，涉及删除的事务回滚可能失败。
- `--refresh` 每次都会重新拉元数据，CI 里高频调用会拖慢并加重镜像源负担。
- 缓存问题（元数据过期/损坏）先 `dnf clean all && dnf makecache`。

### macOS 差异

**macOS 没有 yum/dnf**。用 Homebrew 或 MacPorts。在 CentOS/RHEL 容器里则与 Linux 一致。

---

## 3. rpm — RPM 包

底层 RPM 包工具，**不做依赖求解**（依赖要自己装）。手册共 **112 项**。

### 基本语法

```
rpm [OPTION...] PACKAGE_FILE ...
rpm [OPTION...] -q [select-options] [query-options]
```

### 主要模式

| 选项 | 说明 |
|---|---|
| `-i, --install` | 安装新包（不处理升级场景） |
| `-U, --upgrade` | 安装或升级（旧的/被废弃的会被移除） |
| `-F, --freshen` | 只升级已装过旧版本的包 |
| `--reinstall` | 重装 |
| `-e, --erase` | 卸载 |
| `-q, --query` | 查询 |
| `-V, --verify` | 校验已装文件与包元数据是否一致 |
| `-K, --checksig, --import` | 校验/导入签名（见 rpmkeys(8)） |
| `--restore` | 恢复已装包的文件元数据（时间戳/属主/权限/能力） |
| `--querytags` | 列出可用 querytag |
| `--initdb` / `--rebuilddb` / `--verifydb` | 数据库初始化/重建/校验（见 rpmdb(8)） |
| `--addsign` / `--delsign` / `--resign` | 签名操作（见 rpmsign(1)） |
| `--specfile` | 见 rpmspec(1) |

### 查询选择器（`-q` 配合）

| 选项 | 说明 |
|---|---|
| `-a, --all` | 所有已装包（可加 `tag=pattern` 收窄） |
| `-f, --file FILE` | 某文件属于哪个包 |
| `-p, --package PKG` | 查询未安装的包文件 |
| `-g, --group GROUP` | 按组查询 |
| `--path PATH` | 拥有该路径的包（不论是否已装） |
| `--whatprovides CAP` | 哪个包提供该能力 |
| `--whatrequires CAP` | 哪个包依赖该能力 |
| `--whatconflicts` / `--whatobsoletes` / `--whatrecommends` / `--whatsuggests` / `--whatsupplements` / `--whatenhances` | 相应的反向查询 |
| `--triggeredby PKG` | 由哪些包触发 |
| `--tid TID` | 按事务 ID 查 |
| `--querybynumber HDRNUM` | 直接查数据库条目（调试用） |
| `--dupes` | 列出重复安装的包 |
| `--nomanifest` / `--noglob` | 不把非包文件当 manifest / 不做 glob |

### 查询输出选项

| 选项 | 说明 |
|---|---|
| `-i, --info` | 包信息（名称/版本/描述） |
| `-l, --list` | 列出包内文件 |
| `-R, --requires` | 列出依赖 |
| `--provides` | 列出提供的能力 |
| `--obsoletes` / `--conflicts` / `--recommends` / `--suggests` / `--supplements` / `--enhances` | 相应的能力列表 |
| `--scripts` | 列出安装/卸载脚本 |
| `--triggers, --triggerscripts` | 列出触发器脚本 |
| `--filetriggers` | 列出文件触发器 |
| `--changelog` / `--changes` | 变更记录（后者带完整时间戳） |
| `-s, --state` | 文件状态（normal/not installed/replaced，隐含 `-l`） |
| `--dump` | 文件详情（隐含 `-l`） |
| `--filesbypkg` | 按包分组列文件 |
| `--last` | 按安装时间排序 |
| `--xml` | 以 XML 输出头信息 |
| `-c, --configfiles` / `-d, --docfiles` / `-L, --licensefiles` / `-A, --artifactfiles` | 只列配置/文档/许可/产物文件 |
| `--noartifact` / `--noconfig` / `--noghost` | 排除产物/配置/幽灵文件 |
| `--filecaps` / `--fileclass` / `--filecolor` / `--fileprovide` / `--filerequire` | 文件能力/分类/颜色/提供/依赖 |
| `--qf FMT, --queryformat FMT` | 自定义输出格式（见 rpm-queryformat(7)） |

### 安装/卸载修饰符

| 选项 | 说明 |
|---|---|
| `-h, --hash` | 解包时打 50 个 `#`（配合 `-v` 更好看） |
| `-v, --verbose` | 详细 |
| `--force` | 等价 `--replacepkgs --replacefiles --oldpackage` |
| `--replacepkgs` | 已装也强装 |
| `--replacefiles` | 覆盖其他包的文件 |
| `--oldpackage` | 允许用旧版替换新版 |
| `--nodeps` | 不检查依赖 |
| `--test` | 只演练不实际执行 |
| `--justdb` | 只改数据库不动文件系统 |
| `--nodb` | 只动文件系统不改数据库 |
| `--noscripts` | 不执行任何脚本（等价一串 `--nopre/--nopost/...`） |
| `--nopre` / `--nopost` / `--nopreun` / `--nopostun` / `--nopretrans` / `--noposttrans` / `--nopreuntrans` / `--nopostuntrans` | 不执行指定类型的脚本 |
| `--notriggers` / `--notriggerin` / `--notriggerun` / `--notriggerprein` / `--notriggerpostun` | 不执行触发器 |
| `--allmatches` | 移除所有匹配版本的包 |
| `--excludedocs` / `--includedocs` | 不装/装文档 |
| `--excludeconfigs, --noconfigs` | 不装配置文件 |
| `--excludepath=OLDPATH` | 不装以 OLDPATH 开头的文件 |
| `--excludeartifacts` | 不装产物文件（如 build-id 链接） |
| `--allfiles` | 装上所有 missingok 文件 |
| `--prefix=NEWPATH` / `--relocate=OLD=NEW` / `--badreloc` | 可重定位包改路径 |
| `--ignoresize` / `--ignorearch` / `--ignoreos` | 忽略空间/架构/OS 检查 |
| `--noverify` | 安装前不校验包文件 |
| `--nocontexts` / `--nocaps` | 不设 SELinux 上下文 / 文件能力 |
| `--nosysusers` | 不创建 rpm-sysusers |
| `--noorder` | 不按依赖重排 |
| `--noplugins` | 不加载插件 |
| `--deploops` | 把依赖环打印为警告 |
| `--percent` | 打印百分比（供外部工具调用） |
| `--ignore-failed-read` 等见手册 | — |

### 校验修饰符（`-V` 配合）

`--nodeps`、`--nodigest`、`--nofiles`、`--noscripts`、`--nosignature`、`--nolinkto`、`--nofiledigest`（旧名 `--nomd5`）、`--nosize`、`--nomtime`、`--nomode`、`--nordev`、`--nouser`、`--nogroup`、`--nocaps`。

`-V` 输出为 9 个字符，依次对应：`S`(size) `M`(mode) `5`(digest) `D`(device) `L`(link) `U`(user) `G`(group) `T`(mtime) `P`(caps)。`.` 表示通过，`?` 表示无法检查。

### 示例

```bash
rpm -qa | grep nginx                     # 已装的 nginx 包
rpm -qi nginx                            # 包信息
rpm -ql nginx                            # 包内文件
rpm -qc nginx                            # 只列配置文件
rpm -qf /usr/sbin/nginx                  # 该文件属于哪个包
rpm -qp --requires pkg.rpm               # 未安装包的依赖
rpm -q --whatprovides /usr/bin/python3
rpm -q --changelog nginx | head -30
rpm -V nginx                             # 校验文件是否被改
sudo rpm -ivh pkg.rpm                    # 安装（带 hash + verbose）
sudo rpm -Uvh pkg.rpm                    # 升级
sudo rpm -e nginx                        # 卸载
rpm -qp --queryformat '%{NAME} %{VERSION}\n' pkg.rpm
sudo rpm --import RPM-GPG-KEY
rpm -K pkg.rpm                           # 校验签名
```

### 易踩坑

- **rpm 不做依赖求解**。`rpm -ivh` 缺依赖会失败，生产上应用 `dnf`/`yum` 装包，`rpm` 只用来查询和排障。
- `rpm -U` 与 `rpm -i` 的区别：`-i` 装第二份副本（可能冲突），`-U` 才是正常升级。
- `rpm -V` 的 9 个字符要按位读，**没有输出表示全部通过**。
- `rpm -qa | grep` 在包多时较慢，可用 `rpm -qa --qf '%{NAME}\n'`。
- `--force` 是三个危险开关的合并（`--replacepkgs --replacefiles --oldpackage`），会掩盖真正的冲突。
- `--nodeps` 装出来的包**依赖是坏的**，除非你非常清楚在做什么。
- 数据库损坏时用 `rpm --rebuilddb`，但**先备份 `/var/lib/rpm`**。

### macOS 差异

**macOS 没有 rpm**（除非装了 rpm 二进制）。macOS 的包格式是 `.pkg`，用 `installer` / `pkgutil` 管理。

---

## 4. dpkg — Debian 包

Debian 底层包工具（同 rpm 之于 RPM），**不做依赖求解**。手册共 **58 项**。

### 基本语法

```
dpkg [option...] command
```

### 主要操作

| 选项 | 说明 |
|---|---|
| `-i, --install PKG.deb` | 安装（`-R` 时参数应为目录） |
| `--unpack PKG.deb` | 只解包不配置 |
| `--configure PKG\|-a\|--pending` | 配置已解包未配置的包（`-a` 全部） |
| `--triggers-only` | 只处理触发器 |
| `-r, --remove PKG` | 卸载（保留 conffiles） |
| `-P, --purge PKG` | 彻底清除（含 conffiles） |
| `-V, --verify [PKG]` | 校验已装文件完整性 |
| `-C, --audit [PKG]` | 数据库一致性检查 |
| `--get-selections [PATTERN]` | 导出选择状态 |
| `--set-selections` | 从 stdin 设选择状态 |
| `--clear-selections` | 把所有非必要包标为 deinstall |
| `--update-avail` / `--merge-avail` | 更新可用包信息 |
| `-A, --record-avail PKG.deb` | 记录包的可用信息 |
| `--forget-old-unavail` | 已废弃，现为空操作 |
| `--clear-avail` | 清空可用信息 |
| `--yet-to-unpack` | 找出选了但没装的包 |
| `--predep-package` | 打印 pre-dependency 目标包 |
| `--add-architecture ARCH` / `--remove-architecture ARCH` | 增删可安装架构 |
| `--print-architecture` | 打印本机包架构（如 `amd64`） |
| `--print-foreign-architectures` | 打印额外允许的架构 |
| `--assert-feature` / `--assert-help` | 断言 dpkg 支持某特性 |
| `--validate-thing STRING` | 校验字符串语法 |
| `--compare-versions v1 op v2` | 比较版本号（op 为二元运算符） |
| `--version` / `-?, --help` | 版本 / 帮助 |
| `--force-help` / `-Dh, --debug=help` | 强制选项/调试选项帮助 |

### 通用修饰符

| 选项 | 说明 |
|---|---|
| `--force-things` / `--no-force-things, --refuse-things` | 强制/拒绝某些行为（things 为逗号分隔列表） |
| `--ignore-depends=PKG,...` | 忽略指定包的依赖检查 |
| `--no-act, --dry-run, --simulate` | 演练，不写任何改动 |
| `-R, --recursive` | 递归处理目录下所有 `*.deb` |
| `-G` | 已装更新版本时不装（`--refuse-downgrade` 别名） |
| `-B, --auto-deconfigure` | 删包时自动反配置依赖它的包 |
| `--abort-after=N` | 出错多少次后中止（默认 50） |
| `-Doctal, --debug=octal` | 打开调试（位或组合） |
| `--admindir=DIR` | 设置管理目录 |
| `--instdir=DIR` | 设置安装目录 |
| `--root=DIR` | 设置根目录（同时改 instdir 与 admindir） |
| `-O, --selected-only` | 只处理被选中安装的包 |
| `-E, --skip-same-version` | 同版本同架构则跳过 |
| `--pre-invoke=CMD` / `--post-invoke=CMD` | dpkg 运行前后执行的钩子 |
| `--path-exclude=GLOB` / `--path-include=GLOB` | 路径过滤 |
| `--verify-format=FMT` | `--verify` 输出格式（目前只支持 `rpm`） |

### 配套命令（dpkg 家族）

```bash
dpkg -l                  # = dpkg-query -l，列出已装包
dpkg -L nginx            # = dpkg-query -L，列出包内文件
dpkg -S /usr/sbin/nginx  # = dpkg-query -S，查文件属于哪个包
dpkg -s nginx            # = dpkg-query -s，包状态详情
dpkg-deb -c pkg.deb      # 列出 deb 内文件
dpkg-deb -I pkg.deb      # 查看 deb 元信息
dpkg-deb -x pkg.deb DIR  # 解包到目录（不安装）
dpkg --configure -a      # 配置所有未配置的包（修复中断）
```

### 示例

```bash
dpkg -l | grep nginx
dpkg -L nginx | head
dpkg -S /usr/bin/curl
dpkg -s nginx | grep -E 'Version|Status'
sudo dpkg -i ./pkg.deb
sudo dpkg -r nginx && sudo dpkg -P nginx
sudo dpkg --configure -a
dpkg --get-selections > pkgs.txt
sudo dpkg --set-selections < pkgs.txt
dpkg --compare-versions 1.2.3 gt 1.2.2 && echo newer
dpkg -l | awk '/^ii/{print $2}'
```

### 易踩坑

- **dpkg 不做依赖求解**。`dpkg -i` 报依赖错误时，用 `sudo apt-get -f install` 补依赖。
- `dpkg -l` 输出首两列是状态：`ii`=已装且已配置，`rc`=已删但残留配置，`un`=未装。**`rc` 状态的包要用 `dpkg -P` 清掉**。
- `dpkg --configure -a` 是修复「装到一半中断」的标准手段。
- `dpkg -i` 覆盖安装不会自动升级依赖。
- `-P`（purge）会**删掉配置文件**，误用会丢配置。
- `dpkg -S` 只查已装包；查未装文件用 `apt-file search`。
- 架构不匹配（如 i386 包装到 amd64）会失败，需先 `dpkg --add-architecture`。

### macOS 差异

**macOS 没有 dpkg**。包管理用 Homebrew/MacPorts，系统级 `.pkg` 用 `installer`/`pkgutil`。

---

## 5. chmod — 改权限

修改文件/目录的权限位（mode）。权限分三组：**属主(u)**、**属组(g)**、**其他人(o)**，每组有 **读(r=4)**、**写(w=2)**、**执行(x=1)**。

### 基本语法

```
chmod [OPTION]... MODE[,MODE]... FILE...
chmod [OPTION]... OCTAL-MODE FILE...
chmod [OPTION]... --reference=RFILE FILE...
```

### 数字（八进制）模式

| 数字 | 含义 |
|---|---|
| `4` | 读 r |
| `2` | 写 w |
| `1` | 执行 x |
| `0` | 无权限 |

三组叠加：`755` = `rwxr-xr-x`（属主全权、其他人读+执行）；`644` = `rw-r--r--`；`600` = `rw-------`。

**特殊位（第四位）**：

| 前缀 | 名称 | 作用 |
|---|---|---|
| `4xxx` | setuid (SUID) | 执行时以**文件属主**身份运行 |
| `2xxx` | setgid (SGID) | 执行时以**文件属组**身份运行；对目录表示新文件继承该组 |
| `1xxx` | sticky | 目录内文件只能被属主/目录属主/root 删除（如 `/tmp`） |

例：`4755` setuid、`2775` setgid 目录、`1777` sticky 目录。

### 符号模式

格式：`[ugoa...][+-=][rwxXst...]`

| 符号 | 含义 |
|---|---|
| `u` / `g` / `o` / `a` | 属主 / 属组 / 其他人 / 全部（`a` = ugo） |
| `+` | 增加权限 |
| `-` | 移除权限 |
| `=` | 精确设置（清掉其余位） |
| `r` `w` `x` | 读 / 写 / 执行 |
| `X` | 仅当已是目录或已有执行位时才加 x（批量给目录加执行很有用） |
| `s` | setuid/setgid |
| `t` | sticky |
| `u` `g` `o` | 复制对应组的现有权限 |

例：`chmod u+x a.sh`、`chmod go-w file`、`chmod a=r file`、`chmod g=u file`、`chmod -R a+rX dir`。

### 选项（全列，15 项）

| 选项 | 说明 |
|---|---|
| `-c, --changes` | 类似 verbose，但只报告**发生了改变**的 |
| `-f, --silent, --quiet` | 抑制大部分错误信息 |
| `-v, --verbose` | 对每个处理过的文件输出诊断 |
| `--dereference` | 作用于符号链接**指向的目标**（而非链接本身） |
| `-h, --no-dereference` | 作用于符号链接本身 |
| `--no-preserve-root` | 不特殊对待 `/`（默认） |
| `--preserve-root` | 拒绝在 `/` 上递归操作 |
| `--reference=RFILE` | 用 RFILE 的 mode，而非指定 MODE |
| `-R, --recursive` | 递归修改 |
| `-H` | 配合 `-R`：命令行参数里的符号链接到目录则跟随 |
| `-L` | 配合 `-R`：跟随遇到的**每一个**符号链接到目录 |
| `-P` | 配合 `-R`：不跟随任何符号链接（默认） |
| `--help` | 帮助 |
| `--version` | 版本 |

> `-H`/`-L`/`-P` 只在同时给了 `-R` 时有效；同时给多个时**只有最后一个生效**。

### 示例

```bash
chmod 755 a.sh                    # rwxr-xr-x
chmod 644 a.txt                   # rw-r--r--
chmod 600 ~/.ssh/id_ed25519       # 私钥必须 600
chmod u+x,g-w a.sh
chmod -R a+rX /var/www            # 目录给 x、文件给 r（X 的妙用）
chmod 1777 /tmp                   # sticky 目录
chmod 4755 /usr/bin/suidbin       # setuid
chmod --reference=ref.txt a.txt   # 抄 ref.txt 的权限
chmod -c 644 *.txt                # 只报告真正改动的
```

### 易踩坑

- **`chmod -R 777 /` 是经典灾难**，会毁掉系统；`--preserve-root` 可防手滑（GNU 默认是 `--no-preserve-root`）。
- **SSH 私钥权限必须是 `600`**，太开放（如 644）sshd 会直接拒绝使用。
- 目录的 `x` 位含义是「可进入/可遍历」，不是「可执行」；只给 `r` 不给 `x` 会连 `ls` 都列不了。
- `X` 与 `x` 的区别：`X` 只在「是目录」或「已有执行位」时才加，**批量处理混合树时用 `X`**。
- `chmod` 默认**不跟随**符号链接（`-R` 时），要改链接目标用 `--dereference`。
- `= ` 会**清掉**该组所有其他权限位，例如 `chmod a=r file` 会同时去掉 w 和 x。
- 特殊位在 `ls -l` 里显示为：`rws`/`rwS`（setuid）、`rws`（setgid，属组位）、`rwt`/`rwT`（sticky）。大写表示**对应执行位缺失**。
- umask 只影响**新建**文件，不影响 chmod。

### macOS 差异

macOS 的 `chmod` 是 BSD 版，**选项更少**：没有 `-c`、`--changes`、`--reference`、`--preserve-root`、`--dereference`（BSD 用 `-h` 与默认行为区分）。符号模式与八进制模式一致。macOS 还支持 ACL，用 `chmod -a`/`-e`（BSD 专有），Linux 上要用 `setfacl`。

---

## 6. chown — 改属主/属组

修改文件/目录的属主（user）与属组（group）。

### 基本语法

```
chown [OPTION]... [OWNER][:[GROUP]] FILE...
chown [OPTION]... --reference=RFILE FILE...
```

### 属主写法

| 写法 | 效果 |
|---|---|
| `user` | 只改属主 |
| `user:group` | 同时改属主与属组 |
| `user:` | 改属主，并把属组设为该用户的登录组 |
| `:group` | 只改属组 |
| `user.group` | 同 `user:group`（点号写法，注意用户名含点时易歧义） |
| `--from=OLDUSER:OLDGROUP` | 只有当前属主/属组匹配时才改 |

### 选项（全列，16 项）

| 选项 | 说明 |
|---|---|
| `-c, --changes` | 只报告发生了改变的 |
| `-f, --silent, --quiet` | 抑制大部分错误 |
| `-v, --verbose` | 输出诊断 |
| `--dereference` | 作用于符号链接的目标（**默认**） |
| `-h, --no-dereference` | 作用于符号链接本身（仅在支持改链接属主的系统上有效） |
| `--from=CUR_OWNER:CUR_GROUP` | 当前属主/属组匹配时才改（两者可省略其一） |
| `--no-preserve-root` | 不特殊对待 `/`（默认） |
| `--preserve-root` | 拒绝在 `/` 上递归 |
| `--reference=RFILE` | 用 RFILE 的属主/属组 |
| `-R, --recursive` | 递归 |
| `-H` | 命令行参数里的符号链接到目录则跟随 |
| `-L` | 跟随遇到的每个符号链接到目录 |
| `-P` | 不跟随任何符号链接（默认） |
| `--help` | 帮助 |
| `--version` | 版本 |

> 配套命令 `chgrp GROUP FILE...` 只改属组，选项与 chown 基本一致。

### 示例

```bash
sudo chown tom a.txt                     # 只改属主
sudo chown tom:dev a.txt                 # 属主 + 属组
sudo chown :www-data /var/www            # 只改属组
sudo chown -R www-data:www-data /var/www # 递归
sudo chown --reference=/etc/nginx b.conf # 抄属主属组
sudo chown --from=root:root -R /data     # 只改当前是 root:root 的
chown -h tom link                        # 改链接本身的属主
chgrp -R dev /srv/project
```

### 易踩坑

- **只有 root（或有 `CAP_CHOWN`）才能把文件改给别人**；普通用户只能改「自己拥有的文件」的属组（且必须是自己所属的组）。
- `-R` 加错路径是**高危操作**（如 `chown -R user /`），会破坏系统。GNU 的 `--preserve-root` 默认**不开启**。
- `chown user.group` 与 `chown user:group` 等价，但**用户名里含 `.` 时（如 AD 域账号 `DOMAIN.user`）点号写法会歧义**，推荐始终用冒号。
- `chown` 默认**跟随**符号链接（与 `chmod` 相反！），要改链接本身用 `-h`。
- 属主改动**不受 umask 影响**。
- 改属主后，原属主的 setuid/setgid 位会被清除（安全设计）。
- 复制文件时用 `cp -p` / `rsync -a` 保留属主，比事后 `chown` 更稳妥。

### macOS 差异

macOS 的 `chown` 是 BSD 版，选项更少（无 `--reference`、`--preserve-root`、`--from` 的部分形式）。**BSD 的 `-R` 默认不跟随符号链接**且 `-h` 语义略有差异。macOS 支持 `chown -R user:group` 与 ACL（`chmod -e`/`-a`）。

---

## 7. sudo — 以其他身份执行

以另一个用户（默认 root）的身份执行命令，按 `/etc/sudoers` 策略授权，并**缓存凭据**（默认 5 分钟）。

### 基本语法

```
sudo [options] [command]
sudoedit [options] file...
```

### 选项（手册共 29 项，全列）

| 选项 | 说明 |
|---|---|
| `-A, --askpass` | 用（图形）助手程序读密码 |
| `-B, --bell` | 密码提示时响铃 |
| `-b, --background` | 命令在后台运行 |
| `-C num, --close-from=num` | 执行前关闭 ≥num 的文件描述符（不得 <3） |
| `-D dir, --chdir=dir` | 在指定目录运行命令 |
| `-E, --preserve-env` | 保留现有环境变量 |
| `--preserve-env=list` | 只保留列表中（逗号分隔）的环境变量 |
| `-e, --edit` | 编辑文件而非执行命令（= sudoedit） |
| `-g group, --group=group` | 指定主组 |
| `-H, --set-home` | 把 HOME 设为目标用户的家目录 |
| `-h, --help` | 帮助 |
| `-h host, --host=host` | 在指定主机执行（sudoers 插件不支持） |
| `-i, --login` | 以登录 shell 运行（读 `.profile`/`.bash_profile`） |
| `-K, --remove-timestamp` | 清除该用户**所有**缓存的凭据 |
| `-k, --reset-timestamp` | 使当前会话的缓存凭据失效 |
| `-l, --list` | 列出当前用户（或 `-U` 指定用户）的权限 |
| `-N, --no-update` | 不更新缓存凭据 |
| `-n, --non-interactive` | 不提示任何输入（需密码则报错退出） |
| `-P, --preserve-groups` | 保留调用者的组向量 |
| `-p prompt, --prompt=prompt` | 自定义密码提示（支持 `%H` 等转义） |
| `-R dir, --chroot=dir` | 在指定 chroot 里执行 |
| `-S, --stdin` | 从 stdin 读密码（提示写 stderr） |
| `-s, --shell` | 用 SHELL 指定的 shell 运行 |
| `-U user, --other-user=user` | 配合 `-l` 列出指定用户的权限 |
| `-T timeout, --command-timeout=timeout` | 给命令设超时 |
| `-u user, --user=user` | 以指定用户运行（默认 root；`#uid` 写数字） |
| `-V, --version` | 版本 |
| `-v, --validate` | 刷新缓存凭据（默认延长 5 分钟，不执行命令） |
| `--` | 选项结束，后续参数传给命令 |

### sudoers 要点

- 编辑必须用 **`visudo`**（会做语法检查并加锁），直接改 `/etc/sudoers` 出错会导致 sudo 全废。
- 免密：`tom ALL=(ALL) NOPASSWD: /usr/bin/systemctl restart nginx`
- 按组授权：`%wheel ALL=(ALL) ALL`（`wheel`/`sudo` 组成员）
- 包含目录：`#includedir /etc/sudoers.d`
- 常见别名：`User_Alias`、`Host_Alias`、`Runas_Alias`、`Cmnd_Alias`

### 示例

```bash
sudo systemctl restart nginx
sudo -u www-data id
sudo -i                                  # 登录 shell（root）
sudo -s                                  # root shell
sudo -H -u postgres psql                 # 指定用户 + 设 HOME
sudo -E env | grep PATH                  # 保留环境
sudo -l                                  # 看自己能 sudo 什么
sudo -k && sudo whoami                   # 清缓存后重新认证
sudo -n true && echo cached              # 非交互探测缓存
sudo -b tcpdump -i eth0 -w /tmp/cap.pcap # 后台运行
sudoedit /etc/nginx/nginx.conf           # 安全编辑
sudo -u deploy rsync -av src/ host:/dst/
```

### 易踩坑

- **`sudo cmd > file` 里的重定向是以「当前用户」身份执行的**，`sudo echo x > /root/f` 会失败。要写 root 文件用 `sudo tee`（`echo x | sudo tee /root/f`）或 `sudo sh -c 'echo x > /root/f'`。
- `sudo -i` / `sudo -s` 会开一个 root shell，**PATH 与环境与 `su -` 不同**；`sudo -i` 读 root 的 profile。
- 凭据默认缓存 5 分钟（`timestamp_timeout`）；`sudo -k` 立即失效。
- **不要用 `sudo` 跑编辑器/`vim`**，普通用户可通过 shell 逃逸提权；用 `sudoedit`。
- `-E` 保留环境可能把 `LD_PRELOAD`/`LD_LIBRARY_PATH` 带进去，**是提权风险**，sudoers 里默认有 `env_reset` 与 `secure_path`。
- `visudo -c` 可做语法检查；改坏 sudoers 后可用单用户模式或 `pkexec` 救回。
- `sudo` 与 `su` 的认证对象不同：`sudo` 验**自己的**密码，`su` 验**目标用户的**密码（root→任意用户不需密码）。

### macOS 差异

macOS 自带 `sudo`（版本较新），用法一致。macOS 用 `%admin`/`%wheel` 组，且部分系统服务走 `launchd` 而非 systemd，`sudo` 的典型用途是 `sudo launchctl`。

---

## 8. su — 切换用户

切换到另一个用户（默认 root），或以其身份执行命令。

### 基本语法

```
su [options] [-] [user [argument...]]
```

### 选项（手册共 14 项，全列）

| 选项 | 说明 |
|---|---|
| `-c command, --command=command` | 传给 shell 执行（会创建新会话，见 `setsid(2)`） |
| `-f, --fast` | 给 shell 传 `-f`（csh 相关） |
| `-g group, --group=group` | 指定主组（**仅 root 可用**） |
| `-G group, --supp-group=group` | 指定附加组（**仅 root**；首个附加组在无 `-g` 时兼作主组） |
| `-, -l, --login` | 以登录 shell 启动（环境类似真实登录） |
| `-m, -p, --preserve-environment` | 保留整个环境（不设 HOME/SHELL/USER/LOGNAME）；与 `--login` 同用时被忽略 |
| `-P, --pty` | 为会话创建伪终端（更安全，不共享原终端） |
| `-s shell, --shell=shell` | 用指定 shell |
| `--session-command=command` | 同 `-c` 但不创建新会话（**不推荐**） |
| `-T, --no-pty` | 不创建伪终端（与 `-P` 相反） |
| `-w list, --whitelist-environment=list` | `--login` 清环境时保留列表中的变量 |
| `-h, --help` | 帮助 |
| `-V, --version` | 版本 |
| （无） | 密码延迟、PATH 由 `/etc/login.defs` 的 `ENV_PATH`/`ENV_SUPATH` 定义 |

### `su` 与 `su -` 的区别

| 命令 | 环境 | 当前目录 |
|---|---|---|
| `su user` | **保留**原环境变量（PATH/HOME 等基本不变） | 不变 |
| `su - user`（或 `su -l user`） | **重置**为登录环境（读目标用户的 profile） | 切到目标用户家目录 |

### 示例

```bash
su - root                              # 完整登录到 root
su                                     # 不写用户默认 root
su - tom                               # 登录 tom
su -c 'systemctl restart nginx' root   # 以 root 执行一条命令
su -s /bin/bash - tom                  # 指定 shell
su -w DISPLAY - tom                    # 登录时保留 DISPLAY
su -P - tom                            # 独立伪终端（更安全）
sudo su -                              # 先 sudo 再 su（等价 sudo -i）
```

### 易踩坑

- **`su` 验的是目标用户的密码**；root 切到任何用户不需密码。`sudo` 验的是自己的密码。二者认证模型不同。
- **`su -` 的 `-` 不能省**：少了它 PATH/HOME 是原用户的，命令可能找不到或写错地方。
- `su -c 'cmd'` 里的引号很重要，`su -c cmd` 会被 shell 拆词。
- `su` 是**旧的提权方式**，审计与细粒度授权差；现代发行版推荐 `sudo`（可记录、可限定命令）。
- 部分发行版默认**不允许 root 直接 `su`**（`pam_wheel` 限制），只有 `wheel` 组成员可用 `su`。
- `su` 的历史漏洞（如 TIOCSTI 终端注入）使 `-P`（独立 pty）更安全，但并非所有场景可用。
- `su -` 与 `su -l` 等价；`--session-command` 官方明确**不推荐**（不创建新会话，易被信号影响）。

### macOS 差异

macOS 自带 `su`（BSD 版），选项更少。macOS 默认 **root 账号未启用**，需 `sudo su -` 或 `dsenableroot` 启用。macOS 更常用 `sudo -i`。

---

## 9. useradd / passwd — 用户管理

`useradd` 创建用户（shadow-utils），`passwd` 管理密码。配套还有 `usermod`、`userdel`、`groupadd`、`groupdel`、`chage`。

### useradd 基本语法

```
useradd [options] LOGIN
useradd -D [options]
```

### useradd 选项（手册共 33 项，全列）

| 选项 | 说明 |
|---|---|
| `-b, --base-dir BASE_DIR` | 家目录的基础目录（与用户名拼接） |
| `-c, --comment COMMENT` | 注释（通常是全名，写入 passwd 的 GECOS 字段） |
| `-d, --home-dir HOME_DIR` | 指定家目录 |
| `-D, --defaults` | 查看/修改默认值（见下） |
| `-e, --expiredate DATE` | 账号失效日期（`YYYY-MM-DD` 或自 1970 起的**天数**） |
| `-f, --inactive INACTIVE` | 密码过期后再过多少天锁号（`-1` 表示不锁） |
| `-F, --add-subids-for-system` | 系统账号也更新 `/etc/subuid`、`/etc/subgid` |
| `-g, --gid GROUP` | 主组（组名或已存在的 GID） |
| `-G, --groups G1[,G2,...]` | 附加组（逗号分隔，不能有空格） |
| `-h, --help` | 帮助 |
| `-k, --skel SKEL_DIR` | 骨架目录（家目录初始文件来源） |
| `-K, --key KEY=VALUE` | 覆盖 `/etc/login.defs` 默认值（如 `-K PASS_MAX_DAYS=-1`） |
| `-l, --no-log-init` | 不写 lastlog/faillog 数据库 |
| `-m, --create-home` | 创建家目录并复制骨架 |
| `-M, --no-create-home` | 不创建家目录（即使 login.defs 里 `CREATE_HOME=yes`） |
| `-N, --no-user-group` | 不创建同名组，用 `-g` 指定的组 |
| `-o, --non-unique` | 允许 UID 重复（**必须与 `-u` 同用**） |
| `-p, --password PASSWORD` | 初始密码，**必须是 crypt(3) 加密后的值** |
| `-r, --system` | 创建系统账号（无 aging 信息，UID 取 SYS_UID_MIN~SYS_UID_MAX） |
| `-R, --root CHROOT_DIR` | 在 chroot 里操作（不支持 SELinux） |
| `-P, --prefix PREFIX_DIR` | 在指定根下操作配置文件（不 chroot） |
| `-s, --shell SHELL` | 登录 shell |
| `-u, --uid UID` | 指定 UID（须唯一，除非 `-o`） |
| `-U, --user-group` | 创建同名组并把用户加入 |
| `-Z, --selinux-user SEUSER` | 指定 SELinux 用户 |
| `--selinux-range SERANGE` | 指定 SELinux MLS 范围 |
| `--badname` | 允许不符合标准的用户名 |
| （`-D` 子选项） | `-b`/`-e`/`-f`/`-g`/`-s` 在 `-D` 模式下改的是 `/etc/default/useradd` 的默认值 |

### passwd 选项（手册共 17 项，全列）

| 选项 | 说明 |
|---|---|
| `-a, --all` | 仅与 `-S` 同用，显示所有用户状态 |
| `-d, --delete` | 删除用户密码（置空，**账号变为无密码**） |
| `-e, --expire` | 立即令密码过期（强制下次登录改密） |
| `-h, --help` | 帮助 |
| `-i, --inactive INACTIVE` | 密码过期多少天后禁用账号 |
| `-k, --keep-tokens` | 只对已过期的认证令牌改密 |
| `-l, --lock` | 锁定账号（密码前加 `!`） |
| `-n, --mindays MIN_DAYS` | 两次改密的最小间隔天数 |
| `-q, --quiet` | 安静模式 |
| `-r, --repository REPOSITORY` | 在指定仓库改密 |
| `-R, --root CHROOT_DIR` | 在 chroot 里操作 |
| `-P, --prefix PREFIX_DIR` | 在指定根下操作 |
| `-S, --status` | 显示账号状态（7 个字段） |
| `-u, --unlock` | 解锁账号（恢复锁定前的密码） |
| `-w, --warndays WARN_DAYS` | 密码到期前多少天开始警告 |
| `-x, --maxdays MAX_DAYS` | 密码最长有效天数（`-1` 表示移除限制） |
| `-s, --stdin` | 从 stdin 读新密码（可管道） |

### 示例

```bash
sudo useradd -m -s /bin/bash tom                # 建用户 + 家目录 + bash
sudo useradd -r -s /sbin/nologin svcuser        # 系统账号、不可登录
sudo useradd -m -G sudo,docker -c "Tom A" tom   # 附加组 + 注释
sudo passwd tom                                  # 交互设密码
echo 'tom:NewPass123' | sudo chpasswd           # 非交互设密码（推荐）
sudo passwd -l tom                               # 锁定
sudo passwd -u tom                               # 解锁
sudo passwd -e tom                               # 强制下次改密
sudo passwd -S tom                               # 看状态
sudo passwd -n 7 -x 90 -w 14 tom                 # 改密策略
sudo usermod -aG docker tom                      # 追加组（-a 不能省！）
sudo userdel -r tom                              # 删用户 + 家目录
sudo chage -l tom                                # 看 aging 详情
sudo useradd -D                                   # 看默认值
sudo useradd -D -s /bin/bash                      # 改默认 shell
```

### 易踩坑

- **`usermod -G docker tom` 会覆盖** tom 的所有附加组，**必须写 `-aG`**（append）才追加。这是最经典的坑。
- **`useradd` 不会自动建家目录**（除非 `-m`），且默认 shell 可能不是 bash。
- `-p` 要的是 **crypt 加密后的密码**，直接写明文会得到一个「永远登不上」的账号；设密码用 `passwd` 或 `chpasswd`。
- `passwd -d` 把密码置空 = **账号无密码可登录**，是严重安全问题（尤其配合 sshd 的 `PermitEmptyPasswords`）。
- `passwd -l` 只是加 `!` 前缀，**不影响已建立的 SSH key 登录**；要彻底禁止登录还得改 shell 或锁账号。
- `userdel` 不加 `-r` 会**留下家目录与邮件**。
- `-G` 列表里**不能有空格**，`-G a, b` 会被 shell 拆成两个参数。
- `-e` 的日期可以是 `YYYY-MM-DD` 或**自 1970-01-01 起的天数**，两种写法容易混。
- 改 UID/GID 后要 `chown -R` 修正家目录属主，否则文件归属会变成数字 UID。

### macOS 差异

**macOS 没有 `useradd`/`passwd -l` 这套 shadow-utils**。对应工具：

| 用途 | macOS |
|---|---|
| 创建用户 | `sysadminctl -addUser` / `dscl . -create /Users/...` |
| 删除用户 | `sysadminctl -deleteUser` / `dscl . -delete` |
| 改密码 | `passwd`（有，但选项极少）或 `sysadminctl -resetPassword` |
| 改组 | `dseditgroup` |
| 查看 | `dscl . -read /Users/tom`、`id`、`dscacheutil` |

macOS 用 DirectoryService 而非 `/etc/passwd`/`/etc/shadow`，直接编辑 `/etc/passwd` 无效。

---

## 10. tar — 打包/解包

GNU tar，把多个文件打成 `.tar`，可配合压缩。手册共 **157 项**。

### 基本语法

```
tar [OPTION...] [FILE]...
```

### 三大模式

| 选项 | 说明 |
|---|---|
| `-c, --create` | 创建归档 |
| `-x, --extract, --get` | 解包 |
| `-t, --list` | 列出内容 |
| `-r, --append` | 追加文件到归档末尾 |
| `-u, --update` | 只追加比归档内更新的文件 |
| `-A, --catenate, --concatenate` | 把归档拼接到另一个归档 |
| `-d, --diff, --compare` | 比较归档与文件系统 |
| `--delete` | 从归档中删除成员（**不支持压缩归档**） |
| `--test-label` | 测试卷标 |

### 压缩相关

| 选项 | 说明 |
|---|---|
| `-z, --gzip` | gzip（`.tar.gz` / `.tgz`） |
| `-j, --bzip2` | bzip2（`.tar.bz2`） |
| `-J, --xz` | xz（`.tar.xz`） |
| `-Z, --compress` | compress（`.tar.Z`） |
| `--zstd` | zstd（`.tar.zst`） |
| `-a, --auto-compress` | 按扩展名自动选压缩 |
| `-I CMD, --use-compress-program=CMD` | 用外部程序压缩 |
| `--no-auto-compress` | 禁用按扩展名自动压缩 |

### 常用选项

| 选项 | 说明 |
|---|---|
| `-f FILE, --file=FILE` | 指定归档文件（**必须紧邻文件名**） |
| `-v, --verbose` | 详细 |
| `-C DIR, --directory=DIR` | 切换目录（可多次） |
| `-p, --preserve-permissions, --same-permissions` | 保留权限 |
| `-h, --dereference` | 跟随符号链接（打包其目标） |
| `-P, --absolute-names` | 保留绝对路径（**默认会剥掉前导 `/`**） |
| `--exclude=PATTERN` | 排除（可多次） |
| `-X FILE, --exclude-from=FILE` | 从文件读排除模式 |
| `-T FILE, --files-from=FILE` | 从文件读要打包/解包的清单 |
| `--wildcards` / `--no-wildcards` | 解包时是否用通配符匹配成员 |
| `--strip-components=N` | 解包时剥掉前 N 层目录 |
| `-O, --to-stdout` | 解到 stdout |
| `--transform=EXPR` / `--xform` | 用 sed 语法重命名成员 |
| `--numeric-owner` | 用数字 UID/GID（不查名称） |
| `--same-owner` / `--no-same-owner` | 解包时是否恢复属主 |
| `-k, --keep-old-files` | 解包时不覆盖已存在文件 |
| `--skip-old-files` | 静默跳过已存在文件 |
| `--overwrite` / `--overwrite-dir` | 覆盖文件 / 覆盖目录元数据（默认） |
| `-U, --unlink-first` | 覆盖前先删 |
| `--recursive-unlink` | 解包前递归删掉目录内容 |
| `--one-top-level[=DIR]` | 全部解到指定目录（或归档名目录） |
| `-S, --sparse` | 高效处理稀疏文件 |
| `--hole-detection=METHOD` | 稀疏检测方法（`seek`/`raw`） |
| `-W, --verify` | 写完归档后校验 |
| `--remove-files` | 打包后删源文件 |
| `--no-recursion` | 不递归目录 |
| `--ignore-failed-read` | 不可读文件不导致非零退出 |
| `--atime-preserve[=METHOD]` | 保留访问时间（`replace` 默认 / `system`） |
| `--occurrence[=N]` | 只处理第 N 次出现 |
| `--restrict` | 禁用部分潜在有害选项 |
| `--totals[=SIGNAL]` | 打印总字节数 |
| `--checkpoint[=N]` | 每 N 条记录打印进度 |
| `-G, --incremental` | 旧式增量备份 |
| `-g FILE, --listed-incremental=FILE` | 新式增量备份（快照文件） |
| `--level=N` | 增量备份级别（目前只有 `0` 有意义） |
| `--check-device` / `--no-check-device` | 增量时是否检查设备号 |
| `--show-defaults` | 显示内置默认值 |
| `-?, --help` / `--usage` / `--version` | 帮助 / 用法 / 版本 |

### 示例

```bash
tar czvf backup.tar.gz /var/www           # 打包 + gzip
tar xzvf backup.tar.gz                    # 解包 + gzip
tar xzvf backup.tar.gz -C /tmp            # 解到指定目录
tar tzf backup.tar.gz                     # 只看内容不解
tar czf b.tgz --exclude='*.log' app/      # 打包时排除
tar xzf a.tgz --strip-components=1        # 剥掉第一层目录
tar xzf a.tgz -O path/in/archive > out    # 只抽一个文件到 stdout
tar cf - dir | ssh host 'tar xf - -C /dst'  # 边打包边传输
tar --zstd -cf a.tar.zst dir              # zstd 压缩
tar -czf - . | pv | gzip > /dev/null      # 看进度
tar xjf a.tar.bz2                         # bzip2
tar xJf a.tar.xz                          # xz
tar tvf a.tar                             # 详细列出（不解）
```

### 易踩坑

- **`-f` 后面必须紧跟文件名**：`tar czf a.tgz dir` 对，`tar czfv a.tgz dir` **错**（`v` 被当成文件名）。
- **解包时 tar 会剥掉路径前导 `/`**，把绝对路径变成相对路径（安全设计）；要保留用 `-P`（危险，可能覆盖系统文件）。
- **`tar xf` 覆盖已存在文件是默认行为**，没有提示；先 `tar tf` 看清单。
- `tar xf` 不会自动创建 `-C` 指定的目录（GNU tar 较新版本会，但老版本不会）。
- 经典「tar 炸弹」：`tar` 归档里含 `../../etc/passwd` 这类路径，解包时会写到目标目录外。**解不可信归档前先 `tar tf` 检查路径**。
- **压缩归档不能追加**：`tar rf a.tgz x` 会失败，得先解开再重打。
- `--exclude` 的**模式匹配位置敏感**，`--exclude='*.log'` 与 `--exclude='dir/*.log'` 不同。
- `tar -z`/`-j`/`-J` 调用的压缩程序必须在 PATH 里。
- GNU tar 会自动识别压缩格式解压（即使没写 `-z`），但**创建时必须显式指定**。

### macOS 差异

**macOS 自带的是 bsdtar 而非 GNU tar**。差异：bsdtar **默认自动识别压缩**、支持 `-a` 自动压缩、`--strip-components` 可用，但**缺少 `--transform`、`--listed-incremental`、`-S` 的 GNU 语义**，且 `--exclude` 行为略有不同。要 GNU tar 需 `brew install gnu-tar`（命令为 `gtar`）。macOS 归档还涉及资源分支（`._*` 文件）与 `COPYFILE_DISABLE=1`。

---

## 11. gzip / gunzip — gz 压缩/解压

GNU gzip，用 LZ77 压缩单文件（**不能打包多个文件**，多文件请先 `tar`）。

### 基本语法

```
gzip   [ -acdfhklLnNrtvV19 ] [-S suffix] [ name ... ]
gunzip [ -acfhklLnNrtvV ]   [-S suffix] [ name ... ]
zcat   [ -fhLV ] [ name ... ]
```

### 选项（全列，22 项）

| 选项 | 说明 |
|---|---|
| `-a, --ascii` | ASCII 文本模式（仅部分非 Unix 系统支持） |
| `-c, --stdout, --to-stdout` | 输出到 stdout，保留原文件 |
| `-d, --decompress, --uncompress` | 解压 |
| `-f, --force` | 强制压缩/解压（即使有硬链接/目标已存在/数据非 gzip 格式） |
| `-h, --help` | 帮助 |
| `-k, --keep` | 保留输入文件 |
| `-l, --list` | 列出压缩信息（压缩后/原始大小/比率/原名） |
| `-L, --license` | 显示许可证 |
| `-n, --no-name` | 压缩时不保存原始文件名与时间戳 |
| `-N, --name` | 压缩时总是保存原始文件名与时间戳 |
| `-q, --quiet` | 抑制所有警告 |
| `-r, --recursive` | 递归处理目录 |
| `-S .suf, --suffix .suf` | 用指定后缀代替 `.gz` |
| `--synchronous` | 同步输出（崩溃时更不易丢数据，但更慢） |
| `-t, --test` | 校验压缩文件完整性 |
| `-v, --verbose` | 显示压缩率 |
| `-V, --version` | 版本 |
| `-1` … `-9, --fast, --best` | 压缩级别（`-1` 最快 / `-9` 最小，默认 `-6`） |
| `--rsyncable` | 让 rsync 能只传变化部分 |

`gunzip` = `gzip -d`；`zcat` = `gunzip -c`。

### 示例

```bash
gzip a.txt                       # 生成 a.txt.gz，删原文件
gzip -k a.txt                    # 保留原文件
gzip -9 -k a.txt                 # 最高压缩比
gzip -d a.txt.gz                 # 解压（= gunzip）
gzip -dc a.txt.gz                # 解压到 stdout（= zcat）
gunzip a.txt.gz
zcat a.txt.gz | head
gzip -l a.txt.gz                 # 看压缩率
gzip -t a.txt.gz                 # 校验完整性
gzip -r logs/                    # 递归压缩
tar czf - dir | gzip -9 > d.tgz  # 打包 + 高压缩
```

### 易踩坑

- **`gzip` 默认删除原文件**（生成 `.gz` 后原文件消失），要保留加 `-k`。
- **gzip 不能压缩多个文件成一个**（那是 `tar` 的活）；`gzip a b c` 会生成 `a.gz b.gz c.gz`。
- gzip **不压缩符号链接**（会跳过）。
- gzip 与 zip 不同：gzip 是「单文件流」，没有目录结构与多成员概念。
- `gunzip` 能识别 `.tgz`/`.taz` 并当作 `.tar.gz`/`.tar.Z`；也能解开 `zip` 的**单成员 deflate**，但多成员 zip 必须用 `unzip`。
- `.gz` 用 `file` 命令能看出是 gzip 数据；用 `zcat` 看内容比先解压更安全。
- 压缩已压缩文件（如 jpg/mp4）**基本没有收益**，反而浪费时间。

### macOS 差异

macOS 自带 gzip（BSD 版），核心选项一致。macOS 也有 `gunzip`/`zcat`。注意 macOS 上 `tar -z` 调用的是 bsdtar 内部的 gzip。

---

## 12. zip / unzip — zip 压缩/解压

Info-ZIP 工具。zip 支持**多文件 + 目录结构 + 单文件随机访问**（与 tar.gz 的流式不同），且跨平台（Windows 原生支持）。zip 手册共 **181 项**（含大量平台专有项）。

### zip 基本语法

```
zip [-options] [-b path] [-t mmddyyyy] [-n suffixes] [zipfile list] [-xi list]
```

### zip 常用选项

| 选项 | 说明 |
|---|---|
| `-r, --recurse-paths` | 递归打包目录 |
| `-R, --recurse-patterns` | 从当前目录递归匹配模式 |
| `-m, --move` | 打包后**删除**源文件 |
| `-d, --delete` | 从归档中删除条目 |
| `-u, --update` | 只更新更新的文件 |
| `-f, --freshen` | 只更新已存在的条目 |
| `-g, --grow` | 追加到已有归档 |
| `-FS, --filesync` | 同步归档与文件系统（删掉归档里多余项） |
| `-j, --junk-paths` | 只存文件名，丢掉路径 |
| `-p, --paths` | 保留相对路径（默认） |
| `-x files, --exclude files` | 排除文件 |
| `-i files, --include files` | 只包含指定文件 |
| `-e, --encrypt` | 交互式设密码加密 |
| `-P password, --password password` | 命令行给密码（**不安全**，会被 `ps` 看到） |
| `-y, --symlinks` | 把符号链接当链接存储 |
| `-X, --no-extra` | 不保存额外属性（uid/gid/时间） |
| `-0` … `-9, -#` | 压缩级别（`-0` 只存储不压缩） |
| `-Z cm, --compression-method cm` | 压缩方法（`store`/`deflate`） |
| `-n suffixes, --suffixes` | 指定后缀的文件不压缩（只存储） |
| `-q, --quiet` | 安静 |
| `-v, --verbose` | 详细 / 版本 |
| `-T, --test` | 测试新归档完整性 |
| `-TT cmd, --unzip-command cmd` | 指定测试用命令 |
| `-z, --archive-comment` | 为整个归档加注释 |
| `-c, --entry-comments` | 为每个条目加注释 |
| `-o, --latest-time` | 归档时间设为最旧条目时间 |
| `-D, --no-dir-entries` | 不创建目录条目 |
| `-A, --adjust-sfx` | 调整自解压归档 |
| `-J, --junk-sfx` | 去掉前置 SFX 数据 |
| `-O outfile, --output-file` | 输出到新文件而非原地更新 |
| `-s splitsize, --split-size` | 创建分卷归档 |
| `-sp, --split-pause` | 分卷暂停模式 |
| `-sb, --split-bell` / `-sv, --split-verbose` | 分卷响铃 / 详细 |
| `-t mmddyyyy, --from-date` | 只处理此日期之后修改的 |
| `-tt mmddyyyy, --before-date` | 只处理此日期之前修改的 |
| `-b path, --temp-path path` | 临时归档目录 |
| `-MM, --must-match` | 所有模式必须匹配到文件 |
| `-nw, --no-wild` | 不做内部通配处理 |
| `-@, --names-stdin` | 从 stdin 读文件名列表 |
| `-UN v, --unicode v` | Unicode 文件名处理策略 |
| `-U, --copy-entries` | 从另一归档复制条目 |
| `-la, --log-append` / `-lf, --logfile-path` / `-li, --log-info` | 日志控制 |
| `-db, --display-bytes` / `-dc, --display-counts` / `-dd, --display-dots` / `-ds size, --dot-size` / `-du, --display-usize` / `-dv, --display-volume` / `-dg, --display-globaldots` | 进度显示 |
| `-h, --help` / `-h2, --more-help` | 帮助 / 扩展帮助 |
| `-so, --show-options` | 显示本机编译支持的所有选项 |
| `-sf, --show-files` / `-su, --show-unicode` / `-sU, --show-just-unicode` | 显示将要处理的文件 |
| `-L, --license` | 许可证 |
| `-!` `--use-privileges`、`-$` `--volume-label`、`-k` `--DOS-names`、`-l` `--to-crlf`、`-ll` `--from-crlf`、`-S` `--system-hidden`、`-E` `--longnames`、`-ic` `--ignore-case`、`-ws` `--wild-stop-dirs`、`-DF` `--difference-archive`、`-FI` `--fifo`、`-RE` `--regex`、`-C`/`-C2`/`-C5` VMS 大小写、`-V`/`-VV`/`-w`/`-ww` VMS、`-B`/`-Bn` 非 Unix 平台项 | 平台专有 |

### unzip 选项（45 项）

| 选项 | 说明 |
|---|---|
| `-l` | 列出归档内容（简表） |
| `-v` | 列出（详细）/ 版本信息 |
| `-t` | 测试归档（校验 CRC） |
| `-p` | 解到 stdout（二进制，无提示） |
| `-c` | 解到 stdout 并打印文件名 |
| `-f` | freshen：只解已存在且更新的 |
| `-u` | update：解已存在且更新的，并创建新文件 |
| `-o` | 覆盖已存在文件**不提示**（危险） |
| `-n` | **从不覆盖**已存在文件 |
| `-j` | 丢掉路径，全部解到当前目录 |
| `-d DIR` | 解到指定目录 |
| `-x list` | 排除指定文件 |
| `-q` | 安静（`-qq` 更安静） |
| `-a` | 自动转换文本文件 |
| `-b` | 全部按二进制处理 |
| `-C` | 大小写不敏感匹配 |
| `-D` | 不恢复时间戳 |
| `-K` | 保留 SUID/SGID/sticky 位（默认清除） |
| `-L` | 大写文件名转小写 |
| `-M` | 分页输出 |
| `-P password` | 解密密码（**不安全**） |
| `-X` | 恢复属主/权限（Unix 下 UID/GID） |
| `-U` | 修改/禁用 UTF-8 处理 |
| `-W` | 通配符匹配时不跨目录 |
| `-:` | 允许解到当前目录之外 |
| `-^` | 允许文件名含控制字符 |
| `-$` | 恢复卷标 |
| `-Z` | zipinfo 模式（其后选项按 zipinfo 解释） |
| `-z` | 只显示归档注释 |
| `-B` | 覆盖前备份（需编译时启用） |
| `-2` `-s` `-S` `-V` `-Y` `-/` `-E` `-F` `-i` `-J` `-N` `-T` | VMS/MacOS/OS2/BeOS 等平台专有 |

### 示例

```bash
zip -r archive.zip dir/                  # 递归打包
zip -r -9 archive.zip dir/               # 最高压缩
zip -r archive.zip dir/ -x '*.log'       # 排除
zip -e secret.zip a.txt                  # 交互加密
zip -j flat.zip path/to/a.txt            # 丢掉路径
zip -u archive.zip newfile.txt           # 更新
zip -d archive.zip old.txt               # 从归档删除
zip -r -s 100m big.zip data/             # 分卷（每卷 100MB）
unzip -l archive.zip                     # 列出内容
unzip archive.zip                        # 解压
unzip -o archive.zip -d /tmp             # 覆盖解到 /tmp
unzip -n archive.zip                     # 不覆盖
unzip -j archive.zip -d /tmp             # 丢掉路径解压
unzip -p archive.zip file.txt > out.txt  # 单个文件到 stdout
unzip -t archive.zip                     # 测试完整性
unzip -O gbk archive.zip                 # 处理 GBK 文件名乱码
```

### 易踩坑

- **中文文件名乱码**：Windows 打的 zip 常用 GBK/CP936，Linux 的 unzip 默认按 UTF-8 解，会乱码。用 `unzip -O gbk`（需 unzip 编译时支持）或 `-U` 处理。
- **`unzip -o` 静默覆盖**，无提示，误用会覆盖重要文件；反之默认行为是**交互式询问**，在脚本里会卡住，需显式 `-o` 或 `-n`。
- **`zip -P` 与 `unzip -P` 的密码在命令行明文**，会被 `ps`/shell history 记录，**不安全**；zip 传统加密本身也很弱（ZipCrypto）。
- zip 的**加密不加密文件名**，且 zip 2.0 的传统加密可被已知明文攻击破解；要安全加密请用 7z/age/gpg。
- `zip -r` 会把**目录本身**打进去（`zip -r a.zip dir` 里含 `dir/...`），只想装内容要 `cd dir && zip -r ../a.zip .`。
- `-m`（move）打包后删源文件，误用会丢数据。
- zip **不能追加到压缩归档的中间**（与 tar 类似，但 zip 支持追加到末尾 `-g`）。
- zip 对**超过 4GB 的归档**需要 Zip64 支持（现代版本默认开启）。

### macOS 差异

macOS 自带 `zip`/`unzip`（Info-ZIP 移植版）。注意：macOS 的 zip 会默认包含 `._*` 资源分支文件，用 `zip -X` 或设 `COPYFILE_DISABLE=1` 可排除。macOS 的 `unzip` 通常**不支持 `-O`**（编码选项），处理中文乱码需装 GNU unzip 或用 Python/7z。

---

## 13. xz — xz 压缩/解压

用 LZMA2 算法，**压缩率通常优于 gzip/bzip2**，代价是更慢、更耗内存。手册共 **54 项**。

### 基本语法

```
xz [OPTION]... [FILE]...
```

### 操作模式

| 选项 | 说明 |
|---|---|
| `-z, --compress` | 压缩（默认操作） |
| `-d, --decompress, --uncompress` | 解压 |
| `-t, --test` | 校验完整性（等价 `-dc` 但丢弃输出） |
| `-l, --list` | 列出压缩文件信息 |

### 常用选项

| 选项 | 说明 |
|---|---|
| `-k, --keep` | 保留输入文件（5.2.6+ 对符号链接也生效） |
| `-f, --force` | 强制（覆盖已存在目标、处理非普通文件等） |
| `-c, --stdout, --to-stdout` | 输出到 stdout（隐含 `--keep`） |
| `--single-stream` | 只解第一个 .xz 流，忽略后续数据 |
| `--no-sparse` | 不创建稀疏文件 |
| `-S .suf, --suffix=.suf` | 指定后缀（默认 `.xz`/`.lzma`） |
| `--files[=file]` / `--files0[=file]` | 从文件读文件名（后者用 NUL 分隔） |
| `-F fmt, --format=fmt` | 格式：`auto`/`xz`/`lzma`/`raw` |
| `-C check, --check=check` | 完整性校验类型：`none`/`crc32`/`crc64`/`sha256` |
| `--ignore-check` | 解压时不校验完整性 |
| `-0` … `-9` | 预设级别（默认 `-6`） |
| `-e, --extreme` | 更慢但压缩率更好的变体 |
| `--fast` / `--best` | `-0` / `-9` 的别名（官方建议少用） |
| `--block-size=size` | 分块大小 |
| `--block-list=items` | 按间隔分块并指定过滤链 |
| `--flush-timeout=timeout` | 超时后强制 flush（流式场景） |
| `--no-sync` | 不 fsync（更快，掉电风险） |
| `--memlimit-compress=limit` / `--memlimit-decompress=limit` / `--memlimit-mt-decompress=limit` | 内存限制 |
| `-M limit, --memlimit=limit, --memory=limit` | 同时设三种内存限制 |
| `--no-adjust` | 内存不足时直接报错而不自动调整 |
| `-T threads, --threads=threads` | 线程数（`0` = 按 CPU 核数） |
| `--filters=chain` / `--filters1..9` / `--filters-help` | 自定义过滤链 |
| `--lzma1[=opts]` / `--lzma2[=opts]` | LZMA1/LZMA2 过滤 |
| `--x86` `--arm` `--armthumb` `--arm64` `--powerpc` `--ia64` `--sparc` `--riscv` | BCJ 分支/调用/跳转过滤器（可显著提升可执行文件压缩率） |
| `--delta[=opts]` | Delta 过滤 |
| `-q, --quiet` / `-v, --verbose` | 安静 / 详细（可重复） |
| `-Q, --no-warn` | 有警告也不把退出码设为 2 |
| `--robot` | 机器可解析输出 |
| `--info-memory` | 显示内存与线程信息 |
| `-h, --help` / `-H, --long-help` | 帮助 / 完整帮助 |
| `-V, --version` | 版本 |

### 示例

```bash
xz a.txt                       # 生成 a.txt.xz，删原文件
xz -k a.txt                    # 保留原文件
xz -9e -k big.iso              # 最高压缩比
xz -d a.txt.xz                 # 解压（= unxz）
xz -dc a.txt.xz | head         # 解到 stdout
xz -l a.txt.xz                 # 看信息
xz -t a.txt.xz                 # 校验
tar cJf a.tar.xz dir/          # 配合 tar
xz -T0 -9 data.bin             # 多线程压缩
xz -dc a.xz | less             # 不解压到磁盘直接看
unxz a.txt.xz                  # = xz -d
```

### 易踩坑

- **`xz` 默认删除原文件**，要保留加 `-k`。
- **xz 解压极耗内存**（高压缩级别可达数百 MB 甚至 GB），小内存 VPS 上解大 xz 文件可能 OOM；用 `--memlimit` 限制或选较低级别。
- `-9e` 压缩非常慢，**只在归档长期保存时值得**。
- xz **不打包多文件**（那是 `tar` 的活），只能压单文件/流。
- `.xz` 与 `.lzma` 是不同格式（`.lzma` 是旧格式，`-F lzma` 才生成）；`xz -d` 两者都能解。
- 压缩后的文件**不能用 `grep` 直接搜**，需 `xz -dc f.xz | grep`。
- `-T0` 多线程压缩在**小文件上反而更慢**（线程开销），大文件才划算。
- `--check=none` 会牺牲完整性校验，嵌入式/特定场景才用。

### macOS 差异

macOS 自带 `xz`（较新版本有）。用法一致。macOS 上 `tar -J` 调用的是 bsdtar 内部的 xz 支持。

---

## 14. echo — 输出

输出文本。**注意：`echo` 在 bash 里是内建命令**，行为受 shell 选项（`xpg_echo`）影响，且各实现差异大。

### 基本语法

```
echo [SHORT-OPTION]... [STRING]...
```

### 选项（全列，5 项）

| 选项 | 说明 |
|---|---|
| `-n` | 不输出末尾换行 |
| `-e` | 启用反斜杠转义解释 |
| `-E` | 禁用反斜杠转义解释（**默认**） |
| `--help` | 帮助 |
| `--version` | 版本 |

### `-e` 支持的转义

| 转义 | 含义 |
|---|---|
| `\a` | 响铃 (BEL) |
| `\b` | 退格 |
| `\c` | **不再输出后续内容**（且不加换行） |
| `\e` / `\E` | ESC |
| `\f` | 换页 |
| `\n` | 换行 |
| `\r` | 回车 |
| `\t` | 水平制表 |
| `\v` | 垂直制表 |
| `\\` | 反斜杠 |
| `\0nnn` | 八进制字符（`nnn` 为 0~3 位八进制） |
| `\xHH` | 十六进制字符 |
| `\uHHHH` | Unicode 码点（4 位十六进制） |
| `\UHHHHHHHH` | Unicode 码点（8 位十六进制） |

### 示例

```bash
echo hello                       # hello
echo -n hello                    # 不换行
echo -e 'a\tb\nc'                # 解释制表与换行
echo -e '\033[31mred\033[0m'     # ANSI 红字
echo $PATH                       # 输出变量
echo "$PATH"                     # 变量在双引号内展开
echo '$PATH'                     # 单引号：原样输出
echo *                           # 通配符展开（当前目录文件）
echo -e '\x41'                   # A
echo -e 'a\cb'                   # 只输出 a（\c 截断）
printf '%s\n' "$var"             # 更可移植的替代
echo "text" >> file              # 追加到文件
```

### 易踩坑

- **`echo` 的 `-e`/`-n` 不是 POSIX 标准**，不同 shell（dash/sh/bash/zsh）行为不同。**脚本里要可移植请用 `printf`**。
- `echo` 不引号时会被 shell 做**通配符展开与词分割**；`echo *` 会列出当前目录文件。
- 不加引号时多余空格会被压缩；`echo a  b` 输出 `a b`。
- `echo` 里 `-n`/`-e` 若出现在字符串位置（如 `echo -n`），可能被当成选项；要输出字面 `-n` 用 `echo -- -n` 或 `printf`。
- **`\c` 会截断后续所有输出**且不加换行，容易造成「输出莫名少了」。
- bash 的 `shopt -s xpg_echo` 会让内建 echo 默认解释转义，**与 `-E` 默认行为相反**，是跨平台脚本的常见陷阱。
- 想输出变量且保留原样（含空格/换行）**必须加双引号**：`echo "$var"`。

### macOS 差异

macOS 默认 shell 是 zsh，其内建 `echo` 默认**解释转义**（更接近 `-e` 行为），与 Linux 的 bash/dash 默认**不解释**相反。`/bin/echo` 是 BSD 版。跨平台脚本务必用 `printf`。

---

## 15. export — 设/导出环境变量

**bash 内建命令**，把变量标记为「导出」，使其对**后续启动的子进程**可见。

### 基本语法

```
export [-fn] [name[=value]] ...
export -p [-f]
```

### 选项（全列）

| 选项 | 说明 |
|---|---|
| `-f` | 操作的是**函数**而非变量 |
| `-n` | **取消导出**（移除导出属性），变量仍在但不再传给子进程 |
| `-p` | 以可重用为输入的形式显示所有已导出的名字 |

不给名字、或只给 `-p` 时，列出所有已导出变量。`-p` 与 `-f` 同用则显示已导出的函数。

### 示例

```bash
export JAVA_HOME=/usr/lib/jvm/java-21
export PATH="$JAVA_HOME/bin:$PATH"
export LANG=en_US.UTF-8
export -p | grep JAVA            # 查看已导出的
export -n SECRET                 # 取消导出（变量仍在）
export -f myfunc                 # 导出函数给子 shell
export -p -f                     # 列出已导出函数
export MYVAR=1 MYOTHER=2         # 一次导出多个
declare -x FOO=bar               # 等价写法
env | grep JAVA_HOME             # 验证是否导出
```

### 易踩坑

- **`export` 只在当前 shell 及其子进程有效**，不会影响父 shell；写进脚本的 export 在脚本退出后消失。要持久化需写进 `~/.bashrc`、`~/.profile` 或 `/etc/environment`、`/etc/profile.d/*.sh`。
- **`VAR=x` 不等于 `export VAR=x`**：前者只是 shell 变量，子进程看不到。
- `export VAR=$OTHER` 与 `export VAR="$OTHER"` 在含空格时不同，**始终加双引号**。
- `export PATH=...` 覆盖后可能**丢掉原有 PATH**，务必写成 `export PATH="new:$PATH"`。
- `-n` 只取消导出，**不删除变量**；删除用 `unset`。
- 交互式 shell 与脚本读的初始化文件不同（登录 shell 读 `.bash_profile`，非登录交互读 `.bashrc`），export 放错文件会「有时生效有时不」。
- `sudo` 默认**不继承**导出的环境（`env_reset`），要传需 `sudo -E` 或在 sudoers 里配 `env_keep`。

### macOS 差异

macOS 默认 shell 是 zsh，`export` 语义相同。但 macOS 的初始化文件不同：zsh 读 `~/.zshrc`/`~/.zprofile`；且 macOS 有 `launchctl setenv` 来给 GUI 应用设环境变量。系统级变量在 `/etc/paths`、`/etc/paths.d/`。

---

## 16. env — 查看/临时改环境

在修改后的环境中运行命令，或显示当前环境。手册 13 项。

### 基本语法

```
env [OPTION]... [-] [NAME=VALUE]... [COMMAND [ARG]...]
```

### 选项（全列，13 项）

| 选项 | 说明 |
|---|---|
| `-a ARG, --argv0=ARG` | 把 ARG 作为 COMMAND 的第 0 个参数 |
| `-i, --ignore-environment` | 从**空环境**开始 |
| `-0, --null` | 每行输出以 NUL 结尾（而非换行） |
| `-u NAME, --unset=NAME` | 从环境中移除变量 |
| `-C DIR, --chdir=DIR` | 切换工作目录到 DIR |
| `-S S, --split-string=S` | 按 shell 规则拆分 S（**shebang 行传多参数的关键**） |
| `--block-signal[=SIG]` | 向 COMMAND 屏蔽信号 |
| `--default-signal[=SIG]` | 重置信号处理为默认 |
| `--ignore-signal[=SIG]` | 让 COMMAND 忽略信号 |
| `--list-signal-handling` | 列出非默认的信号处理到 stderr |
| `-v, --debug` | 每个处理步骤打印详细信息 |
| `--help` | 帮助 |
| `--version` | 版本 |

### 示例

```bash
env                                  # 列出所有环境变量
env | grep -i proxy                  # 过滤
env | sort
env -i /bin/bash --noprofile --norc  # 干净 shell
env -i PATH=/usr/bin:/bin cmd        # 只给最小环境
env VAR=1 OTHER=2 cmd                # 临时设变量运行
env -u http_proxy curl url           # 临时移除某变量
env -C /tmp pwd                      # 临时切目录
env -0 | tr '\0' '\n'                # NUL 分隔转行
#!/usr/bin/env -S python3 -u         # shebang 传多参数（-S 的作用）
env -S 'bash -c "echo hi"'           # 用 -S 拆字符串
```

### 易踩坑

- **`env` 的临时变量只对该次命令有效**，不会留在当前 shell（那是 `export` 的活）。
- `env VAR=1` 只是**临时**设置；写成 `env VAR=1` 单跑没有意义（无 COMMAND 时只列出环境）。
- **shebang 行 `#!/usr/bin/env python3 -u` 在多数系统上不生效**（内核只取一个参数），需 `#!/usr/bin/env -S python3 -u`。
- `env -i` 从空环境开始，**PATH 也没了**，必须显式给 PATH 否则命令找不到。
- `env` 输出顺序不固定（哈希表顺序），要稳定输出用 `env | sort`。
- `env` 能看到的多为**已导出**的变量；shell 内部变量（未 export）看不到。
- `-u` 是「移除」，不是「设空值」；设空用 `VAR=`。
- `env` 与 `printenv` 区别：`env` 还能改环境并运行命令，`printenv` 只能打印。

### macOS 差异

macOS 自带 `env`（BSD 版），**没有 `-S`、`-C`、`-a`、信号相关选项**（较老版本）。macOS 上 `#!/usr/bin/env -S ...` 也常不生效，脚本宜用 `#!/usr/bin/env bash` 单参数形式。

---

## 17. which / whereis — 查命令路径

`which` 在 PATH 里找可执行文件；`whereis` 找二进制、手册、源码。

### which 基本语法

```
which [options] [--] programname [...]
```

### which 选项（全列）

| 选项 | 说明 |
|---|---|
| `-a, --all` | 打印 PATH 里**所有**匹配，而非只第一个 |
| `-i, --read-alias` | 从 stdin 读别名并报告匹配的（配合 `alias which='alias \| which -i'`） |
| `--skip-alias` | 忽略 `--read-alias` |
| `--read-functions` | 从 stdin 读 shell 函数定义并报告匹配 |
| `--skip-functions` | 忽略 `--read-functions` |
| `--skip-dot` | 跳过 PATH 中以点开头的目录 |
| `--skip-tilde` | 跳过以 `~` 开头的目录及 HOME 下的可执行文件 |
| `--show-dot` | PATH 中点开头的目录匹配时，打印 `./program` 而非完整路径 |
| `--show-tilde` | 目录匹配 HOME 时输出 `~`（root 调用时忽略） |
| `--tty-only` | 不在 tty 上时停止处理右侧选项 |
| `-v, -V, --version` | 版本 |
| `--help` | 帮助 |

返回值：失败参数的个数；没给 programname 时返回 `-1`。

### whereis 选项（全列，12 项）

| 选项 | 说明 |
|---|---|
| `-b` | 只搜二进制 |
| `-m` | 只搜手册 |
| `-s` | 只搜源码 |
| `-u` | 只显示「不寻常」的条目（某类型条目数不等于 1） |
| `-B list` | 限定二进制搜索目录（空格分隔） |
| `-M list` | 限定手册搜索目录 |
| `-S list` | 限定源码搜索目录 |
| `-f` | 结束目录列表，开始文件名（**用了 `-B/-M/-S` 就必须加**） |
| `-l` | 输出实际使用的查找路径 |
| `-g` | 把后续名字当 glob 模式（只比对 basename） |
| `-h, --help` | 帮助 |
| `-V, --version` | 版本 |

### 示例

```bash
which java                       # 找 java 的路径
which -a python3                 # 所有 python3
which -a ls                      # 可能有别名与 /bin/ls 两个
whereis nginx                    # 二进制 + 手册
whereis -b -m python3
whereis -l nginx                 # 看它搜了哪些目录
type -a ls                       # bash 内建：区分别名/函数/内建/文件
command -v python3               # 最可移植的「命令在哪」
type -p python3                  # 只输出路径
```

### 易踩坑

- **`which` 不认 shell 内建与别名**（`which cd` 可能找不到，因为 `cd` 是内建）。查「shell 会怎么解释这个命令」用 **`type -a`**。
- `which` 只按 **PATH** 找，不查函数；`type` 才全面。
- `which` 在不同发行版是**不同实现**（debianutils 的 `which` vs GNU which），选项有差异。
- **脚本里判断命令是否存在，最可移植的是 `command -v`**，而不是 `which`（后者可能未安装）。
- `whereis` 用的是**硬编码/预建数据库**路径，不查 PATH，所以可能与 `which` 结果不同。
- `whereis` 用了 `-B`/`-M`/`-S` 后**必须用 `-f`**，否则把文件名当成目录列表。
- `which` 的返回值是「失败个数」，多个参数时非零不代表全失败。

### macOS 差异

macOS 自带 `which`（BSD/旧版）与 `whereis`，但 **macOS 的 `which` 不支持 `--read-alias` 等 GNU 选项**。macOS 推荐用 `type`/`command -v`。注意 macOS 的 `which` 不查 shell 函数。

---

## 18. history — 历史命令

**bash 内建命令**，查看/操作命令历史。

### 基本语法

```
history [n]
history -c
history -d offset
history -d start-end
history -anrw [filename]
history -p arg [arg ...]
history -s arg [arg ...]
```

### 选项（全列）

| 选项 | 说明 |
|---|---|
| `history [n]` | 列出最近 n 条（不带 n 列全部）；带 `*` 前缀的是被修改过的 |
| `-c` | **清空**历史列表 |
| `-d offset` | 删除第 offset 条（负数从末尾倒数，`-1` 是当前这条） |
| `-d start-end` | 删除区间（闭区间） |
| `-a` | 把**本次会话新产生**的历史追加到历史文件 |
| `-n` | 读入历史文件中**本次会话尚未读过**的新行 |
| `-r` | 读历史文件并追加到当前列表 |
| `-w` | 把当前列表**覆盖写**到历史文件 |
| `-p arg...` | 对参数做历史替换并输出（不存入列表） |
| `-s arg...` | 把参数作为**单条**存入历史列表 |

指定 `filename` 则用该文件代替 `$HISTFILE`。

### 相关环境变量

| 变量 | 作用 |
|---|---|
| `HISTSIZE` | 内存中保留的历史条数（`0` 表示不记录） |
| `HISTFILESIZE` | 历史文件保留的条数 |
| `HISTFILE` | 历史文件路径（默认 `~/.bash_history`） |
| `HISTCONTROL` | `ignorespace`（空格开头不记）/`ignoredups`（重复不记）/`ignoreboth`/`erasedups` |
| `HISTIGNORE` | 冒号分隔的模式列表，匹配的行不记录（`&` 匹配上一条） |
| `HISTTIMEFORMAT` | 设置后为每条历史加时间戳（如 `'%F %T '`） |

### 历史展开（快捷键与语法）

| 写法 | 含义 |
|---|---|
| `!!` | 上一条命令 |
| `!n` | 第 n 条 |
| `!-n` | 倒数第 n 条 |
| `!string` | 最近一条以 string 开头的 |
| `!?string?` | 最近一条含 string 的 |
| `!$` / `!!:$` | 上一条的**最后一个参数** |
| `!*` | 上一条的所有参数 |
| `!^` | 上一条的第一个参数 |
| `^old^new` | 把上一条中的 old 换成 new 再执行 |
| `!cmd:s/old/new/` | 对指定历史做替换 |
| `Ctrl-R` | 反向增量搜索（`Ctrl-S` 正向） |
| `Alt-.` / `Esc .` | 插入上一条的最后一个参数 |

### 示例

```bash
history | grep ssh               # 搜历史里的 ssh
history 20                       # 最近 20 条
history -d 101                   # 删第 101 条
history -c                       # 清空本次会话历史
history -w                       # 立刻写盘
HISTTIMEFORMAT='%F %T ' history | tail -20
export HISTCONTROL=ignoreboth     # 忽略重复 + 空格开头的
export HISTIGNORE='ls:ll:cd:pwd'  # 这些命令不记
export HISTSIZE=10000 HISTFILESIZE=20000
sudo !!                          # 用 sudo 重跑上一条
ls -la /var/log && cd !$         # cd 到上一条的最后一个参数
!ssh                             # 重跑最近的 ssh 命令
```

### 易踩坑

- **历史在会话退出时才写入 `~/.bash_history`**（除非 `history -a`/`-w`）。多终端并发时后写的会覆盖，导致丢历史。
- **敏感信息（密码、token）会进历史**：命令行里直接写密码是常见事故。用 `HISTCONTROL=ignorespace` 并在命令前加空格，或 `set +o history` 临时关闭。
- `history -c` 只清**内存列表**，不清 `~/.bash_history` 文件；要彻底清需再 `history -w`（覆盖）或手动删文件。
- `HISTSIZE=0` 不记录，但**历史文件里已有的仍在**。
- `!` 在**双引号内会触发展开**（`echo "hi!"` 在交互式 shell 里可能出错），单引号内不展开。
- 脚本（非交互 shell）默认**不记历史**（`HISTFILE` 未设或 `set -o history` 关闭）。
- zsh 的 `history` 行为与 bash 不同（zsh 用 `fc`/`history` 内建，语法有别）。

### macOS 差异

macOS 默认 shell 是 zsh，历史文件是 `~/.zsh_history`，变量是 `HISTSIZE`/`SAVEHIST`，且 zsh 有 `setopt hist_ignore_dups` 等选项。若在 macOS 用 bash，则与 Linux 一致（但 bash 版本是 3.2，很旧）。

---

## 19. alias — 别名

**bash 内建命令**，为命令定义快捷方式。配套 `unalias`。

### 基本语法

```
alias [-p] [name[=value] ...]
unalias [-a] name [name ...]
```

### 选项（全列）

| 选项 | 说明 |
|---|---|
| `-p` | 以 `alias name=value` 形式打印所有别名（等价无参数） |
| （无参数） | 同上，打印所有别名 |
| `name` | 打印该别名的定义 |
| `name=value` | 定义/覆盖别名 |

`unalias`：`-a` 删除所有别名，否则删除指定的。

### 别名展开规则

- 别名**只在交互式 shell 中默认展开**；非交互 shell（脚本）需 `shopt -s expand_aliases`。
- 别名在**解析时**展开，且只在**命令位置**（第一个词）展开。
- 值**以空格结尾**时，下一个词也会尝试别名展开（可做「别名串」）。
- 别名**不能带参数**，需要参数请用 shell 函数。

### 示例

```bash
alias                            # 列出所有别名
alias ll='ls -lah'
alias la='ls -A'
alias ..='cd ..'
alias ...='cd ../..'
alias grep='grep --color=auto'
alias df='df -h'
alias vi='vim'
alias ll='ls -lah'               # 覆盖已有别名
alias git-s='git status -sb'
unalias ll                       # 删除单个
unalias -a                       # 删除全部
\ls                              # 反斜杠绕过别名，用原始 ls
command ls                       # 同理绕过别名
type ll                          # 看 ll 是别名还是别的
```

**持久化**：把别名写进 `~/.bashrc`（交互式）或 `~/.bash_aliases`（Ubuntu 的 `.bashrc` 会 source 它）。

**需要参数的场景改用函数**：

```bash
mkcd() { mkdir -p "$1" && cd "$1"; }
alias 做不到 → 函数可以
```

### 易踩坑

- **别名在脚本里默认不生效**（非交互 shell 不展开别名），需 `shopt -s expand_aliases` 且要在**定义之后**的行才生效。
- 别名定义必须在**使用它的代码之前**（解析时展开）。
- 别名**不能传参数**；`alias grep='grep --color'` 后面跟的参数是追加的，但没法引用「第 N 个参数」。
- 想绕过别名用**反斜杠**（`\ls`）或 `command ls`。
- 别名**不递归**（`alias ls='ls -l'` 不会无限循环）。
- 同名别名会覆盖，且**没有警告**；重复 source `.bashrc` 时旧别名被新值替换（幂等）。
- 交互式与登录 shell 读的文件不同，别名放错地方会「新开的终端没生效」。
- 别名展开发生在**引号之外**，`echo 'll'` 不会展开。

### macOS 差异

macOS 默认 zsh，别名语法相同（`alias` 是内建）。持久化写 `~/.zshrc`。macOS 的 `unalias` 也支持 `-a`。

---

## 20. crontab — 定时任务

管理 cron 定时任务表。`crontab` 命令（cronie）与 `crontab(5)` 文件格式。

### crontab 命令语法

```
crontab [-u user] file
crontab [-u user] [-i] {-e | -l | -r}
```

### crontab 选项（全列，10 项）

| 选项 | 说明 |
|---|---|
| `-u user` | 操作指定用户的 crontab（需权限） |
| `-l` | 显示当前 crontab |
| `-e` | 用 `VISUAL`/`EDITOR` 编辑当前 crontab |
| `-r` | **删除**当前 crontab（**无确认**） |
| `-i` | 配合 `-r`，删除前提示 `y/Y` |
| `-T` | 只测试语法，不安装 |
| `-s` | 编辑/替换前，把当前 SELinux 上下文作为 `MLS_LEVEL` 追加 |
| `-n host` | 集群支持：设置由哪个主机运行任务（需 cron 以 `-c` 启动） |
| `-c` | 集群支持：查询当前由哪个主机运行 |
| `-V` | 版本 |

### crontab 文件格式（crontab(5)）

```
分 时 日 月 周   命令
*  *  *  *  *  command
```

| 字段 | 允许值 |
|---|---|
| 分钟 minute | 0-59 |
| 小时 hour | 0-23 |
| 日 day of month | 1-31 |
| 月 month | 1-12（或名字 jan-dec） |
| 周 day of week | 0-7（0 或 7 = 周日，或名字 sun-sat） |

**匹配规则**：分、时、月三者都匹配，且「日」与「周」**至少一个**匹配时才执行。

**语法元素**：

| 写法 | 含义 |
|---|---|
| `*` | 任意值（first-last） |
| `a-b` | 范围（含两端），如 `8-11` |
| `a,b,c` | 列表 |
| `*/n` | 每 n 个单位，如 `*/15` 每 15 分钟 |
| `a-b/n` | 范围内每 n 个 |
| `@reboot` | 启动时执行一次 |
| `@yearly` / `@annually` | 每年一次（= `0 0 1 1 *`） |
| `@monthly` | 每月一次（= `0 0 1 * *`） |
| `@weekly` | 每周一次（= `0 0 * * 0`） |
| `@daily` / `@midnight` | 每天一次（= `0 0 * * *`） |
| `@hourly` | 每小时（= `0 * * * *`） |

**环境与转义**：

- crontab 里可设环境变量（`PATH=/usr/local/bin:/usr/bin:/bin`、`MAILTO=user@x`、`SHELL=/bin/bash`）。
- 命令里的 `%` 有特殊含义（被当作换行），要字面 `%` 需转义为 `\%`。
- 命令输出会邮件给 `MAILTO` 指定的用户（默认是任务属主）。

### 示例

```bash
crontab -l                                     # 看当前任务
crontab -e                                     # 编辑
crontab -l > backup.txt                        # 备份
crontab backup.txt                             # 恢复/导入
crontab -i -r                                  # 删除（带提示）
crontab -u tom -l                              # 看 tom 的
crontab -T file                                # 只测语法
```

crontab 内容示例：

```
SHELL=/bin/bash
PATH=/usr/local/bin:/usr/bin:/bin
MAILTO=admin@example.com

0 2 * * *      /usr/local/bin/backup.sh >> /var/log/backup.log 2>&1
*/5 * * * *    /usr/bin/curl -s http://localhost/health
0 9 * * 1-5    /usr/local/bin/report.sh
30 3 1 * *     /usr/bin/find /tmp -type f -mtime +7 -delete
@reboot        /usr/local/bin/start-worker.sh
0 */6 * * *    cd /app && ./sync.sh
```

系统级任务放 `/etc/crontab`（多一个「用户名」字段）、`/etc/cron.d/*`、`/etc/cron.{hourly,daily,weekly,monthly}/`。

### 易踩坑

- **cron 的 PATH 极简**（通常只有 `/usr/bin:/bin`），脚本里的命令常「手动能跑、cron 跑不了」。**在 crontab 里显式设 PATH 或用绝对路径**。
- **cron 的环境变量与登录 shell 完全不同**（不读 `.bashrc`/`.profile`），依赖环境变量的脚本要显式设置或用 `bash -lc`。
- **`crontab -r` 直接删除、无确认**，是高频误操作；养成用 `-i` 的习惯。
- **`%` 在 crontab 里是特殊字符**（转成换行），`date +%F` 必须写成 `date +\%F`。
- **命令输出默认邮件**给任务属主，累积会撑爆 `/var/mail`；建议重定向 `>> log 2>&1` 或设 `MAILTO=""`。
- **最小粒度是分钟**，秒级任务需靠 `sleep` 或 systemd timer。
- 任务里**必须用绝对路径**（可执行文件、脚本、输入文件都要）。
- **DST（夏令时）**切换时，「不存在的时间」不会触发，「重复的时间」会触发两次。
- 改完 crontab 后建议 `crontab -l` 确认已生效；某些系统 cron 重载有延迟。
- 用 `run-parts` 的 `/etc/cron.daily` 等目录要求脚本**可执行且文件名不含点**。
- **别把长任务堆在同一时刻**（如全放 `0 0 * * *`），会造成负载尖峰；错开分钟。

### macOS 差异

macOS **仍自带 cron 与 `crontab`**，用法与 Linux 一致（但建议用 `launchd`/`launchctl` 做系统级调度）。macOS 上 cron 的日志在 `/var/log/cron`（需开启）。注意 macOS 的 cron 与 `launchd` 的权限模型不同，Full Disk Access 可能影响任务。

---

## 21. date — 时间

显示或设置系统时间。GNU coreutils 版，手册 12 项。

### 基本语法

```
date [OPTION]... [+FORMAT]
date [-u|--utc|--universal] [MMDDhhmm[[CC]YY][.ss]]
```

### 选项（全列，12 项）

| 选项 | 说明 |
|---|---|
| `-d STRING, --date=STRING` | 显示 STRING 描述的时间（而非当前） |
| `--debug` | 标注解析结果，并对可疑用法告警 |
| `-f FILE, --file=FILE` | 对 FILE 每行做一次 `--date`（`-` 为 stdin） |
| `-I[FMT], --iso-8601[=FMT]` | ISO 8601 格式（`date`/`hours`/`minutes`/`seconds`/`ns`） |
| `--resolution` | 输出时间戳分辨率 |
| `-R, --rfc-email` | RFC 5322 格式（`Mon, 14 Aug 2006 02:34:56 +0000`） |
| `--rfc-3339=FMT` | RFC 3339 格式（`date`/`seconds`/`ns`） |
| `-r FILE, --reference=FILE` | 显示 FILE 的最后修改时间 |
| `-s STRING, --set=STRING` | 设置时间（需 root） |
| `-u, --utc, --universal` | 显示/设置 UTC |
| `--help` | 帮助 |
| `--version` | 版本 |

### 常用格式符

| 格式符 | 含义 |
|---|---|
| `%F` | `%Y-%m-%d`（2026-10-14） |
| `%T` | `%H:%M:%S` |
| `%Y` / `%y` | 4 位 / 2 位年 |
| `%m` / `%d` | 月 / 日（01-12 / 01-31） |
| `%H` / `%M` / `%S` | 时 / 分 / 秒（24 小时制） |
| `%s` | Unix 时间戳（自 1970-01-01 UTC 的秒数） |
| `%j` | 一年中的第几天（001-366） |
| `%A` / `%a` | 星期全名 / 缩写 |
| `%B` / `%b` | 月全名 / 缩写 |
| `%Z` / `%z` | 时区名 / 数字偏移（+0800） |
| `%N` | 纳秒 |
| `%u` / `%w` | 星期几（1-7 / 0-6） |
| `%I` / `%p` | 12 小时制小时 / AM-PM |
| `%%` | 字面 `%` |

### 示例

```bash
date                             # 当前时间
date +%F                         # 2026-10-14
date '+%F %T'                    # 2026-10-14 22:14:34
date +%s                         # Unix 时间戳
date -d @1791987195              # 时间戳转日期
date -d 'next monday'            # 下周一
date -d '2 days ago' +%F         # 两天前
date -d '+1 month' +%F           # 一个月后
date -u                          # UTC
date -Iseconds                   # ISO 8601（含时区）
date -R                          # RFC 5322（邮件头用）
date -r file.txt                 # 文件的 mtime
TZ='America/Los_Angeles' date    # 指定时区
date -d @1791987195 +%F          # 时间戳格式化
date --date='TZ="Asia/Shanghai" 09:00'
sudo date -s '2026-10-14 22:00:00'  # 设时间
```

### 易踩坑

- **`-d` 是 GNU 扩展**，BSD/macOS 的 `date` **没有 `-d`**（macOS 用 `-j -f` 或 `-r`），跨平台脚本要小心。
- **`date -s`（设时间）需要 root**，且会立刻改动系统时钟；生产环境应配 NTP/chrony 而不是手动设。
- **`%` 在 crontab 里要转义**为 `\%`（见 crontab 一节）。
- `date +%s` 是 Unix 时间戳；`date -d @N` 是反向。**别把两者搞混**。
- 时间戳单位：`%s` 是**秒**，Java/JS 常用**毫秒**，转换要乘除 1000。
- `-u` 只影响**显示**（`-s` 时影响设置），不改系统时区。
- 系统时区由 `/etc/localtime`（软链到 `/usr/share/zoneinfo/...`）与 `TZ` 环境变量共同决定；`timedatectl` 是更现代的查看/设置方式。
- **脚本里算时间差**用 `date +%s` 相减，比解析字符串可靠。
- 容器里 `date` 用的是**宿主机时钟**（或 `CLOCK_REALTIME`），但时区可能没配（`/etc/localtime` 缺失 → UTC）。

### macOS 差异

**macOS 的 `date` 是 BSD 版**，差异很大：

| 用途 | Linux | macOS |
|---|---|---|
| 指定时间 | `date -d '2 days ago'` | `date -v-2d` |
| 格式化 | `date +%F` | `date +%F`（一致） |
| 时间戳转日期 | `date -d @123` | `date -r 123` |
| 文件 mtime | `date -r file` | `date -r file`（但语义不同！） |
| 解析字符串 | `date -d '2026-10-14'` | `date -j -f '%F' '2026-10-14'` |

macOS 用 `-v` 做相对调整（`-v+1m`、`-v-1d`），`-j` 表示「不设时间只解析」，`-f` 指定输入格式。**这是 macOS↔Linux 差异最大的命令之一**。

---

## 22. watch — 定时执行

周期性执行命令并全屏显示输出（procps-ng）。手册 18 项。

### 基本语法

```
watch [options] command
```

### 选项（全列，18 项）

| 选项 | 说明 |
|---|---|
| `-b, --beep` | 命令非零退出时响铃 |
| `-c, --color` | 解释 ANSI 颜色/样式 |
| `-C, --no-color` | 不解释 ANSI 颜色/样式 |
| `-d, --differences[=permanent]` | **高亮两次输出的差异**（`permanent` 表示显示自首次以来的所有变化） |
| `-e, --errexit` | 命令出错时冻结更新，按键后退出（退出码为命令的退出码） |
| `-f, --follow` | 不清屏，像 `tail -f` 一样滚动 |
| `-g, --chgexit` | 输出**变化**时退出 |
| `-n SEC, --interval=SEC` | 更新间隔秒（<0.1 或 >2678400 会被夹到边界；支持小数，`.` 或 `,` 都行） |
| `-p, --precise` | 按「上次**开始**」计时（更精确）；默认按「上次**结束**」计时 |
| `-q CYCLES, --equexit=CYCLES` | 输出连续 N 个周期不变时退出 |
| `-r, --no-rerun` | 终端 resize 时不重跑，等下一个周期 |
| `-s DIR, --shotsdir=DIR` | 截图保存目录 |
| `-t, --no-title` | 不显示顶部标题栏 |
| `-w, --no-wrap` | 不折行（长行截断） |
| `-x, --exec` | 用 `exec(3)` 直接执行而非 `sh -c`（更快，但**失去 shell 特性**） |
| `-h, --help` | 帮助 |
| `-v, --version` | 版本 |
| （交互） | 按 `q` 退出；按 `空格` 立即执行一次（不等间隔） |

### 示例

```bash
watch -n 2 df -h                        # 每 2 秒看磁盘
watch -n 1 'ss -tn state established'   # 看连接数
watch -d -n 1 free -h                   # 高亮内存变化
watch -d 'ls -l /var/log'               # 高亮文件变化
watch -n 5 'uptime; free -h; df -h'     # 组合命令（需引号）
watch -g -n 1 'curl -s -o /dev/null -w "%{http_code}" url'  # 状态码变化即退出
watch -q 5 -n 1 'cat /proc/loadavg'     # 5 个周期不变即退出
watch -e -n 1 'test -f /tmp/ready'      # 出错（文件不存在）即冻结退出
watch -n 1 -t 'date'                    # 不显示标题
watch -n 1 -x date                      # 不经 sh（更快）
watch -n1 'grep "^cpu MHz" /proc/cpuinfo | sort -nrk4'
```

### 易踩坑

- **命令必须加引号**（含空格/管道/重定向时），否则 `watch df -h` 会把 `-h` 当成 watch 的选项。
- **`-n` 最小 0.1 秒**，再小会被夹到 0.1；`-n 0` 也变成 0.1。
- **默认按「上次结束」计时**，命令慢时实际间隔 = 命令耗时 + n；要固定节奏用 `-p`。
- `-x`（exec）**不经过 shell**，管道、重定向、通配符都会失效，只有简单命令能用。
- `watch` 会**反复执行命令**，对**有副作用的命令（写文件、发请求）要非常小心**。
- `-d` 在输出快速滚动/变化大时几乎全屏高亮，反而看不清。
- 输出被截断到屏幕大小，**超长输出看不到全部**（用 `-w` 不折行或改终端尺寸）。
- 交互键：`q` 退出，`空格` 立即刷新。

### macOS 差异

macOS 自带 `watch`？**不自带**（macOS 有 `watch` 但那是 `snoop`/`tcpdump` 相关的旧命令，与 Linux 的 procps watch **完全不同**）。Linux 用户到 macOS 需 `brew install watch`。macOS 的替代是 `while true; do clear; cmd; sleep 2; done`。

---

## 23. xargs — 参数传递

把标准输入的数据转成命令行参数传给另一个命令（GNU findutils）。

### 基本语法

```
xargs [options] [command [initial-arguments]]
```

### 选项（全列）

| 选项 | 说明 |
|---|---|
| `-0, --null` | 输入项以 **NUL** 分隔（引号/反斜杠不再特殊） |
| `-a FILE, --arg-file=FILE` | 从 FILE 读输入项（stdin 保持不变） |
| `-d DELIM, --delimiter=DELIM` | 用指定字符分隔（支持 `\n` 等转义） |
| `-E EOF-STR` | 设置文件结束串（遇到则忽略后续输入） |
| `-e[eof-str], --eof[=eof-str]` | `-E` 的同义词（**已从 POSIX 移除，用 `-E`**） |
| `-I REPLACE-STR` | 用输入项替换初始参数里的 REPLACE-STR（隐含 `-x` 与 `-L 1`） |
| `-i[replace-str], --replace[=replace-str]` | `-I` 的同义词（**已从 POSIX 移除**；无参数时等价 `-I{}`） |
| `-L MAX-LINES` | 每次命令最多用 MAX-LINES 个非空输入行（隐含 `-x`） |
| `-l[max-lines], --max-lines[=max-lines]` | `-L` 的同义词（**已从 POSIX 移除**） |
| `-n MAX-ARGS, --max-args=MAX-ARGS` | 每次命令最多 MAX-ARGS 个参数 |
| `-P MAX-PROCS, --max-procs=MAX-PROCS` | 最多同时跑 MAX-PROCS 个进程（`0` = 尽可能多） |
| `-o, --open-tty` | 子进程执行前把 stdin 重开为 `/dev/tty`（跑交互程序用） |
| `-p, --interactive` | 每条命令前询问（隐含 `-t`） |
| `--process-slot-var=NAME` | 给每个子进程设唯一的该环境变量（做简易负载分发） |
| `-r, --no-run-if-empty` | 输入为空时不运行命令（**默认会跑一次**） |
| `-s MAX-CHARS, --max-chars=MAX-CHARS` | 命令行最大字符数 |
| `--show-limits` | 显示命令行长度限制 |
| `-t, --verbose` | 执行前把命令行打印到 stderr |
| `-x, --exit` | 超过 `-s` 大小限制时退出 |
| `--` | 选项结束，后续参数即使以 `-` 开头也当操作数 |
| `--help` / `--version` | 帮助 / 版本 |

**互斥**：`--max-lines(-L/-l)`、`--replace(-I/-i)`、`--max-args(-n)` **互斥**，同时给时**最后一个生效**。

### 退出码

| 码 | 含义 |
|---|---|
| `0` | 成功 |
| `123` | 某次命令以 1-125 退出 |
| `124` | 命令以 255 退出 |
| `125` | 命令被信号杀死 |
| `126` | 命令无法执行 |
| `127` | 命令不存在 |
| `1` | 其他错误（如用法错误） |

### 示例

```bash
find . -name '*.log' -print0 | xargs -0 rm        # 安全删除（-print0/-0）
find . -type f | xargs grep -l TODO               # 批量搜
echo a b c | xargs -n1 echo                       # 每次一个参数
echo a b c | xargs -n2 echo                       # 每次两个
ls *.txt | xargs -I{} mv {} {}.bak                # 占位替换
find . -name '*.jpg' | xargs -P4 -n1 convert {} out/{}  # 4 路并行
cat urls.txt | xargs -n1 -P8 curl -sO
ls | xargs -t -I{} echo "处理 {}"                 # 显示执行的命令
ls | xargs -r echo "有文件才执行"                  # 空输入不执行
printf 'a\0b\0' | xargs -0 -n1 echo               # NUL 分隔
find . -type f -name '*.tmp' | xargs -r rm -v     # 删临时文件
echo "a b" | xargs -d '\n' echo                   # 按换行分隔
```

### 易踩坑

- **文件名含空格/换行/引号时 `xargs` 会拆错**。**必须用 `find -print0 | xargs -0`**（或 `find -exec`）。
- **`xargs` 默认会跑一次命令，即使输入为空**（如 `echo -n | xargs rm` 会执行 `rm`）。加 `-r`（GNU 扩展）避免。
- **`xargs rm` 是 destructive**：一旦拆错参数，可能删掉非预期文件。用 `-t` 先看会执行什么，或用 `-p` 逐条确认。
- **`-I` 隐含 `-L 1`**，即每行一个参数、串行执行，**不能与 `-n`/`-P` 的并行期望混用**（`-I` 与 `-n` 互斥）。
- `-i`/`-l`/`-e` 是**已从 POSIX 移除**的老写法，新脚本用 `-I`/`-L`/`-E`。
- `-P` 并行时**输出顺序不确定**，且多个进程写同一资源需自己加锁。
- `xargs` 的输入**默认按空白（含换行）分隔**，与 shell 类似；要按行用 `-d '\n'`，要按 NUL 用 `-0`。
- 输入项含引号时，默认模式下 `xargs` 会尝试解释引号，可能导致「引号丢失」；`-0` 模式则不解释。
- **`-0` 与 `-d` 会禁用文件结束串**。

### macOS 差异

macOS 自带 `xargs`（BSD 版），**缺少 `-r`、`-d`、`-a`、`--process-slot-var`、`--show-limits`** 等 GNU 扩展。**`-0`/`-n`/`-I`/`-L`/`-P`/`-t`/`-x` 可用**。跨平台脚本注意别用 `-r`（BSD 默认就不会空跑）。

---

## 24. tee — 分流输出

把标准输入**同时**写到标准输出和文件（像水管三通）。手册 6 项。

### 基本语法

```
tee [OPTION]... [FILE]...
```

### 选项（全列，6 项）

| 选项 | 说明 |
|---|---|
| `-a, --append` | **追加**到文件（而非覆盖） |
| `-i, --ignore-interrupts` | 忽略中断信号（`SIGINT`） |
| `-p` | 对管道以更合适的方式运行（诊断写非管道的输出） |
| `--output-error[=MODE]` | 写错误时的行为（见下） |
| `--help` | 帮助 |
| `--version` | 版本 |

`--output-error` 的 MODE：`warn`（写非管道输出出错时告警）、`warn-nopipe`（仅非管道告警）、`exit`（出错即退出）、`exit-nopipe`（非管道出错即退出）；不带 MODE 等价 `warn-nopipe`。

### 示例

```bash
cmd | tee out.log                       # 同时显示并保存
cmd | tee -a out.log                    # 追加
cmd 2>&1 | tee out.log                  # 连 stderr 一起
cmd | tee out1.log out2.log             # 写多个文件
echo x | sudo tee /etc/config           # 以 root 写文件（关键技巧）
echo x | sudo tee -a /etc/config        # 追加
cmd | tee /dev/tty | grep x             # 同时显示并过滤
make 2>&1 | tee build.log | tail -5     # 留全量日志、看尾部
df -h | tee /dev/stderr | wc -l         # 一边显示一边统计
```

### 易踩坑

- **`tee` 默认覆盖文件**，要追加必须 `-a`。
- **`sudo cmd > file` 无效**（重定向由当前 shell 做），要写 root 文件用 **`cmd | sudo tee file`**。这是 sudo 一节的经典坑，tee 是其解药。
- `tee` 把数据**同时**送到 stdout 和文件，若只想要文件不想屏幕输出，用 `> file`（但那样就不能用 sudo 了）。
- `tee` **不缓冲**（或行缓冲），在长管道里可能影响性能；`stdbuf` 可调整。
- `-i` 让 `tee` 在收到 Ctrl-C 时继续把已缓冲数据写完。
- 写多个文件时**任一个失败**可能导致整体失败（配合 `--output-error` 控制）。
- `tee /dev/stderr` 或 `/dev/tty` 可用来「既进管道又显示」。

### macOS 差异

macOS 自带 `tee`（BSD 版），**没有 `-p` 和 `--output-error`**，其余一致。

---

## 25. screen / tmux — 会话保持

终端复用器：SSH 断开后会话继续运行，可重连。`screen` 是老牌，`tmux` 是现代化替代。

### tmux 客户端选项（全列，14 项）

```
tmux [-2CDhlNuVv] [-c shell-command] [-f file] [-L socket-name] [-S socket-path] [-T features] [command [flags]]
```

| 选项 | 说明 |
|---|---|
| `-2` | 假定终端支持 256 色 |
| `-C` | 控制模式启动 |
| `-c shell-command` | 用指定命令代替默认 shell |
| `-D` | 不作为守护进程启动服务器 |
| `-f file` | 指定配置文件（默认 `~/.tmux.conf`） |
| `-h` | 打印用法 |
| `-L socket-name` | 指定 socket 名（默认 `/tmp/tmux-<uid>/default`） |
| `-l` | 以登录 shell 行为（目前无实际作用） |
| `-N` | 即使该命令会启动服务器也不启动 |
| `-S socket-path` | 指定 socket 路径 |
| `-T features` | 设置终端特性 |
| `-u` | 强制 UTF-8 输出（即使终端 locale 不是 UTF-8） |
| `-V` | 显示版本 |
| `-v` | 详细日志（写入 `<socket-path>.log`） |

### tmux 常用子命令（手册共 152 个）

| 命令 | 说明 |
|---|---|
| `new-session`（`new`） | 新建会话（`-s name` 命名，`-d` 分离，`-A` 存在则附加） |
| `attach-session`（`attach`/`a`） | 附加到会话（`-t name`） |
| `detach-client`（`detach`） | 分离当前客户端 |
| `list-sessions`（`ls`） | 列出会话 |
| `kill-session` | 杀掉会话（`-t name`） |
| `kill-server` | 杀掉整个服务器（所有会话） |
| `rename-session` | 重命名会话 |
| `new-window`（`neww`） | 新建窗口（`-n name`） |
| `split-window`（`splitw`） | 分割窗格（`-h` 横分、`-v` 竖分） |
| `select-window` / `select-pane` | 选择窗口/窗格 |
| `list-windows`（`lsw`）/ `list-panes`（`lsp`） | 列出窗口/窗格 |
| `send-keys`（`send`） | 向会话发按键（脚本化） |
| `capture-pane`（`capturep`） | 抓取窗格内容（截图/日志） |
| `resize-pane` / `resize-window` | 调整大小 |
| `swap-pane` / `swap-window` / `move-window` / `join-pane` | 移动/交换 |
| `pipe-pane` | 把窗格输出管道到命令 |
| `source-file`（`source`） | 加载配置文件 |
| `set-option`（`set`）/ `show-options`（`show`） | 设/看选项 |
| `bind-key`（`bind`）/ `unbind-key` / `list-keys`（`lsk`） | 键绑定 |
| `command-prompt` / `display-message`（`display`） | 提示 / 显示消息 |
| `has-session` / `wait-for` / `if-shell` / `run-shell` | 脚本辅助 |

### tmux 常用按键（前缀默认 `Ctrl-b`）

| 按键 | 作用 |
|---|---|
| `C-b d` | 分离（detach） |
| `C-b c` | 新建窗口 |
| `C-b n` / `C-b p` | 下一个 / 上一个窗口 |
| `C-b 0..9` | 切到第 n 个窗口 |
| `C-b ,` | 重命名窗口 |
| `C-b %` | 垂直分割（左右） |
| `C-b "` | 水平分割（上下） |
| `C-b 方向键` | 切换窗格 |
| `C-b z` | 当前窗格最大化/还原 |
| `C-b x` | 关闭当前窗格 |
| `C-b [` | 进入复制模式（可滚动） |
| `C-b ]` | 粘贴 |
| `C-b ?` | 帮助（所有按键） |
| `C-b :` | 命令模式 |
| `C-b t` | 显示时钟 |

### screen 选项（手册共 44 项，节选）

| 选项 | 说明 |
|---|---|
| `-S name` | 指定会话名 |
| `-ls [match]` / `-list` | 列出会话 |
| `-r [pid.tty.host]` | 重连 |
| `-R` | 重连，没有则创建 |
| `-d [pid.tty.host]` | 分离指定会话 |
| `-d -r` | 先分离再重连 |
| `-D -R` | 立刻附加（必要时分离他人） |
| `-x` | 附加到**未分离**的会话（多显示器共享） |
| `-m` | 忽略 `$STY` 强制新建 |
| `-d -m` | 分离模式启动新会话 |
| `-X cmd` | 向运行中的会话发送命令 |
| `-Q` | 查询远程会话的某些状态 |
| `-wipe [match]` | 清理已死的会话 |
| `-c file` | 指定配置文件（默认 `~/.screenrc`） |
| `-t name` | 设置窗口标题 |
| `-p number\|name\|-\|=` | 预选窗口 |
| `-A` | 适配所有窗口尺寸 |
| `-a` | 包含所有能力 |
| `-e xy` | 改转义字符 |
| `-h num` | 历史缓冲行数 |
| `-i` | 让中断键中断当前输出 |
| `-L` | 开启自动输出日志 |
| `-Logfile file` | 日志文件 |
| `-l` / `-ln` | 登录模式 |
| `-O` | 选最佳输出模式 |
| `-P` | 开启认证 |
| `-q` | 抑制错误信息 |
| `-s program` | 用指定 shell |
| `-T term` | 设置终端类型 |
| `-U` | UTF-8 模式 |
| `-v` | 版本 |
| `-w` | 保持显示尺寸不变 |
| `-4` / `-6` | 只用 IPv4 / IPv6 |
| `-f, -fn, -fa` | 流控设置 |

screen 按键（前缀默认 `Ctrl-a`）：`C-a d` 分离、`C-a c` 新建窗口、`C-a n/p` 切换、`C-a "` 窗口列表、`C-a S` 水平分割、`C-a |` 垂直分割、`C-a Tab` 切换区域、`C-a [` 复制模式、`C-a ?` 帮助。

### 示例

```bash
# tmux
tmux new -s work                     # 新建命名会话
tmux new -s work -d                  # 后台新建
tmux ls                              # 列出会话
tmux attach -t work                  # 重连（简写 tmux a -t work）
tmux new -A -s work                  # 存在则附加，否则新建
tmux kill-session -t work            # 杀会话
tmux kill-server                     # 杀所有
tmux split-window -h                 # 左右分屏
tmux send-keys -t work 'ls' Enter    # 向会话发命令
tmux capture-pane -p -t work         # 抓取窗格内容
tmux source-file ~/.tmux.conf        # 重载配置
tmux set -g mouse on                 # 开鼠标支持

# screen
screen -S work                       # 新建命名会话
screen -ls                           # 列出
screen -r work                       # 重连
screen -d -r work                    # 先分离再重连
screen -x work                       # 共享附加（不分离对方）
screen -S work -X quit               # 结束指定会话
screen -dmS work bash                # 后台起一个跑 bash 的会话
screen -L -Logfile out.log -S s      # 开启输出日志
```

**典型工作流**（SSH 断开也不中断）：

```bash
ssh host
tmux new -s deploy
# 在里面跑长时间任务
# C-b d 分离，或直接关掉 SSH
# 之后重新 ssh，tmux a -t deploy 继续
```

### 易踩坑

- **忘记 `-t` 指定会话**时，`tmux attach` 只对唯一会话有效；多个会话会报错要你选。
- **`tmux kill-server` 会杀掉所有会话**，是 destructive 级操作，误用会丢掉所有后台任务。
- **tmux 的 socket 名（`-L`）决定会话隔离**：不同 `-L` 的会话互不可见。
- screen 的默认前缀是 `Ctrl-a`，tmux 是 `Ctrl-b`，**互相切换时最容易按错**；`Ctrl-a a` 可发送字面 `Ctrl-a`。
- **配置文件不同**：tmux 是 `~/.tmux.conf`，screen 是 `~/.screenrc`。
- **tmux 里滚动要看历史需先 `C-b [`**（复制模式），鼠标支持需显式 `set -g mouse on`。
- **分离（detach）不等于退出**：`C-b d` 让会话继续跑；直接 `exit` 或 `C-b x` 才是结束。
- **嵌套 tmux/screen 时前缀键会冲突**，需连按两次或改前缀。
- screen 的 `-x` 是「共享附加」，多人可同时看同一会话，适合结对排障。
- 会话在**服务器重启后丢失**（除非配了 systemd 自启 + 持久化方案如 `tmux-resurrect`）。

### macOS 差异

macOS **不自带 screen/tmux**，需 `brew install tmux screen`。tmux 用法与 Linux 一致。macOS 的 Terminal.app/iTerm2 支持原生标签与分屏，但仍推荐 tmux（尤其要 SSH 保活时）。iTerm2 的 tmux 集成模式（`tmux -CC`）在 macOS 上体验很好。

---

## 附录 A：包管理器对照表

| 操作 | Debian/Ubuntu | RHEL/CentOS/Fedora | Arch | macOS |
|---|---|---|---|---|
| 刷新索引 | `apt update` | `dnf makecache` | `pacman -Sy` | `brew update` |
| 安装 | `apt install` | `dnf install` | `pacman -S` | `brew install` |
| 卸载 | `apt remove` | `dnf remove` | `pacman -R` | `brew uninstall` |
| 彻底卸载 | `apt purge` | `dnf remove` | `pacman -Rns` | `brew uninstall --zap` |
| 升级全部 | `apt upgrade` / `full-upgrade` | `dnf upgrade` | `pacman -Syu` | `brew upgrade` |
| 搜索 | `apt search` | `dnf search` | `pacman -Ss` | `brew search` |
| 包信息 | `apt show` | `dnf info` | `pacman -Si` | `brew info` |
| 列已装 | `apt list --installed` / `dpkg -l` | `dnf list installed` / `rpm -qa` | `pacman -Q` | `brew list` |
| 文件属于哪个包 | `dpkg -S` / `apt-file search` | `rpm -qf` / `dnf provides` | `pacman -Qo` | `brew which-formula` |
| 包装了哪些文件 | `dpkg -L` | `rpm -ql` | `pacman -Ql` | `brew list <pkg>` |
| 清理缓存 | `apt clean` / `autoclean` | `dnf clean all` | `pacman -Sc` | `brew cleanup` |
| 自动清依赖 | `apt autoremove` | `dnf autoremove` | `pacman -Qdtq \| pacman -Rns -` | `brew autoremove` |
| 本地包安装 | `dpkg -i x.deb` | `rpm -ivh x.rpm` | `pacman -U x.pkg.tar.zst` | `brew install ./x.rb` |

## 附录 B：压缩格式对照表

| 格式 | 压缩 | 解压 | 特点 |
|---|---|---|---|
| gzip `.gz` | `gzip` | `gunzip` / `zcat` | 快、通用、单文件 |
| bzip2 `.bz2` | `bzip2` | `bunzip2` / `bzcat` | 比 gzip 小、比 xz 快 |
| xz `.xz` | `xz` | `unxz` / `xz -d` | 压缩率最高、慢、耗内存 |
| zstd `.zst` | `zstd` | `unzstd` | 快且压缩率好（现代首选） |
| compress `.Z` | `compress` | `uncompress` | 古老、已淘汰 |
| zip `.zip` | `zip` | `unzip` | 多文件+目录、跨平台、随机访问 |
| 7z `.7z` | `7z a` | `7z x` | 压缩率高、支持 AES 加密 |
| tar 打包 | `tar cf` | `tar xf` | 只打包不压缩（多文件容器） |

**tar 组合速记**：`-z` gzip、`-j` bzip2、`-J` xz、`--zstd` zstd、`-a` 按扩展名自动。压缩比大致：`xz > zstd(-19) > bzip2 > gzip`；速度：`gzip ≈ zstd > bzip2 > xz`。

## 附录 C：危险操作清单

| 命令 | 危险点 | 安全做法 |
|---|---|---|
| `apt purge` / `dnf remove` / `rpm -e` / `dpkg -P` | 卸载并删配置 | 确认包名；先 `-s`/`--test` 演练 |
| `apt full-upgrade` / `dnf upgrade` | 可能移除包 | 先 `apt-get -s dist-upgrade` |
| `apt autoremove` / `dnf autoremove` | 删掉「不再被依赖」的包 | `apt-mark manual <pkg>` 保命 |
| `apt --force-yes` / `--allow-remove-essential` | 跳过所有确认 | 绝不用于生产 |
| `dnf --nogpgcheck` / `rpm --nodeps` | 跳过校验/依赖 | 只在隔离环境 |
| `rpm -e --nodeps` / `dpkg -i` 强装 | 破坏依赖 | 用 dnf/apt 而非底层工具装 |
| `chmod -R 777 /` | 毁掉系统权限 | 永远别对 `/` 递归；`--preserve-root` |
| `chown -R user:group /` | 毁掉属主 | 先 `-n` 确认目标；避免 `/` |
| `rm -rf` 类（配合 `xargs`） | `xargs rm` 拆错参数 | `find -print0 \| xargs -0`；先 `-t` 看 |
| `tar -x` 覆盖 | 覆盖同名文件；tar 炸弹 | 先 `tar tf` 看清单；警惕 `../` 路径 |
| `tar -P -x` | 保留绝对路径可覆盖系统文件 | 不用 `-P` |
| `crontab -r` | **无确认**删除所有定时任务 | 用 `crontab -i -r`；先 `crontab -l > bak` |
| `date -s` | 立刻改系统时钟 | 用 NTP/chrony；需 root |
| `useradd -o` / `-u` 冲突 | UID 重复致文件归属混乱 | 先确认 UID 未被占用 |
| `usermod -G`（漏 `-a`） | 覆盖用户所有附加组 | 始终 `-aG` |
| `passwd -d` | 账号变无密码 | 不要用；用 `passwd -l` 锁 |
| `zip -P` / `unzip -P` | 密码明文暴露在 `ps`/history | 用 7z/age/gpg |
| `unzip -o` | 静默覆盖 | 用 `-n` 或先 `-l` |
| `tee`（无 `-a`） | 覆盖文件 | 需要保留用 `-a` |
| `tmux kill-server` | 杀掉所有会话 | 用 `kill-session -t name` |
| `sudo` + 编辑器/`sh -c` | 提权后破坏力放大 | 用 `sudoedit`；限定命令 |
| `> file`（重定向） | 立即截断文件 | 先备份；用 `>>` 追加 |

## 附录 D：macOS ↔ Linux 差异速查

| 用途 | Linux | macOS |
|---|---|---|
| 包管理 | `apt` / `dnf` / `pacman` | `brew`（Homebrew） |
| 建用户 | `useradd` + `passwd` | `sysadminctl -addUser` / `dscl` |
| 删用户 | `userdel -r` | `sysadminctl -deleteUser` / `dscl . -delete` |
| 加组 | `usermod -aG` | `dseditgroup -o edit -a user -t user group` |
| tar | GNU tar | **bsdtar**（缺 `--transform` 等） |
| date 相对时间 | `date -d '2 days ago'` | `date -v-2d` |
| date 时间戳 | `date -d @N` | `date -r N` |
| date 解析字符串 | `date -d '2026-10-14'` | `date -j -f '%F' '2026-10-14'` |
| watch | `watch -n 2 cmd` | **无**（需 `brew install watch`） |
| 查看环境 | `env` / `printenv` | 同（但 GUI 用 `launchctl getenv`） |
| 设 GUI 环境变量 | `export`（配 `.profile`） | `launchctl setenv` |
| 定时任务 | `crontab` / systemd timer | `crontab` / **`launchd`**（推荐） |
| 服务管理 | `systemctl` | `launchctl` / `brew services` |
| 会话保持 | `tmux` / `screen` | `tmux`（需 brew 装） |
| 默认 shell | bash | **zsh** |
| 默认 shell 配置 | `~/.bashrc` | `~/.zshrc` |
| 历史文件 | `~/.bash_history` | `~/.zsh_history` |
| echo 转义 | 默认**不**解释 | zsh 默认**解释** |
| zip 编码选项 | `unzip -O gbk` | 通常**不支持** `-O` |
| xargs 空输入 | 默认会跑（用 `-r` 避免） | 默认**不会**跑 |
