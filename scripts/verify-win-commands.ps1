# verify-win-commands.ps1 -- Windows 内置工具集全命令验证（常用功能 + 报告）。
#
# 设计（与其他 verify-* 脚本互补，三者各管一层）：
# - verify-dll-layout.ps1 ：DLL lib/ 布局的运行期加载验证（单命令深挖）；
# - verify-installer-layout.ps1 ：安装包内容映射断言（不执行安装）；
# - 本脚本：**每个命令**真实执行一次常用功能验证，双模式覆盖——
#     1) shortname 模式：stage\tools\<cmd>.exe 硬链接（安装后用户的真实通路，
#        argv[0] 分发；OWN_ARGV 家族 rg/fd/awk/bat/onefetch/parallel/vim 只能走这条）；
#     2) subcommand 模式：stage\coreutils.exe <cmd> ...（另一条分发路径）。
#   探针不止 --version：文本流（grep/sed/tr/cut/wc/sort/uniq/tac/nl/od/tee）走
#   stdin→stdout 断言，编码/哈希断言已知测试向量，文件族（mkdir/cp/rm/...）在
#   每行独立的种子工作目录里真实操作，子进程族（timeout/nice/nohup/stdbuf）真实
#   派生，DLL 家族（jq/yq/qalc/nmap/...）各配最小功能用例。
#
# 命令清单不硬编码：运行期执行受测 coreutils.exe（无参数）解析其 stderr 的
# `commands: ...` 行——与 tools-catalog 单一事实来源天然同步（清单即二进制自述）。
#
# 产物：控制台明细 + markdown 报告（-ReportPath，默认 stage 下）+ 追加
# GITHUB_STEP_SUMMARY。任一命令 FAIL / TIMEOUT 即退出 1（车道门禁）。
#
# 用法（在含 src-tauri/binaries 全量产物的仓库根，仅限 Windows CI）：
#   pwsh -NoProfile -File scripts/verify-win-commands.ps1 `
#     -BinariesDir src-tauri/binaries
# 注意：不要给参数传以 `-` 开头的值（pwsh -File 的二次解析坑，见
# verify-dll-layout.ps1 头注释）；本脚本参数均为普通路径/数字。
#
# 字符串字面量全 ASCII（PS 5.1 对无 BOM UTF-8 中文有编码坑），中文只在注释里。

[CmdletBinding()]
param(
    # build-tools.mjs 全量构建产物目录（coreutils-*.exe + 六个 DLL + 数据树）
    [string]$BinariesDir = 'src-tauri/binaries',
    # 模拟安装布局的装配目录（每次重建，先清空）
    [string]$Stage = '',
    # 每命令超时秒数（防 TUI/交互命令卡死车道的兜底）
    [int]$TimeoutSec = 60,
    # markdown 报告输出路径（默认 <Stage>\verify-commands-report.md）
    [string]$ReportPath = ''
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
try { [Console]::OutputEncoding = [Text.Encoding]::UTF8 } catch { }

if (-not $Stage) {
    $base = if ($env:RUNNER_TEMP) { $env:RUNNER_TEMP } else { $env:TEMP }
    $Stage = Join-Path $base 'veloterm-verify-commands'
}
if (-not $ReportPath) { $ReportPath = Join-Path $Stage 'verify-commands-report.md' }

$log = New-Object System.Collections.Generic.List[string]
function Say([string]$s) { Write-Host $s; $script:log.Add($s) }

# ---------------------------------------------------------------- 布局装配 --

if (-not (Test-Path $BinariesDir)) {
    Write-Host "binaries dir not found: $BinariesDir"
    exit 1
}
$BinariesDir = (Resolve-Path $BinariesDir).Path

# 全量构建必须齐备的产物（缺一即失败——本车道就是来兜「打包缺件」的）
$coreutils = Get-ChildItem -Path $BinariesDir -Filter 'coreutils-*.exe' |
    Select-Object -First 1
$requiredDlls = @(
    'veloterm_goffi.dll', 'qalcffi.dll', 'nmapffi.dll',
    'ncatffi.dll', 'npingffi.dll', 'vim64.dll')
$requiredDirs = @('nmap-data', 'vim-runtime')
$requiredFiles = @('ca-bundle.crt')

if (-not $coreutils) { Write-Host 'coreutils-*.exe not found in binaries dir'; exit 1 }
foreach ($dll in $requiredDlls) {
    if (-not (Test-Path (Join-Path $BinariesDir $dll))) {
        Write-Host "missing build product: $dll (full build via build-tools.mjs required)"
        exit 1
    }
}
foreach ($d in $requiredDirs) {
    if (-not (Test-Path (Join-Path $BinariesDir $d))) {
        Write-Host "missing build product dir: $d"
        exit 1
    }
}
foreach ($f in $requiredFiles) {
    if (-not (Test-Path (Join-Path $BinariesDir $f))) {
        Write-Host "missing build product: $f"
        exit 1
    }
}

Say '=== verify-win-commands ==='
Say ("binaries  : {0}" -f $BinariesDir)
Say ("coreutils : {0} ({1:N1} MB)" -f $coreutils.Name, ($coreutils.Length / 1MB))

# 装配与安装包一致的目录形状：
#   stage\coreutils.exe
#   stage\tools\<cmd>.exe    （硬链接短名，全部命令）
#   stage\lib\*.dll          （六个 FFI DLL）
#   stage\nmap-data\ ...     （nmap 数据树，平铺安装根）
#   stage\vim-runtime\ ...   （vim 运行时，平铺安装根）
#   stage\ca-bundle.crt      （ncat 证书束，平铺安装根）
if (Test-Path $Stage) { Remove-Item -Recurse -Force $Stage }
New-Item -ItemType Directory -Path $Stage | Out-Null
$stageExe = Join-Path $Stage 'coreutils.exe'
$toolsDir = Join-Path $Stage 'tools'
$libDir = Join-Path $Stage 'lib'
$workRoot = Join-Path $Stage 'work'
New-Item -ItemType Directory -Path $toolsDir, $libDir, $workRoot | Out-Null

Copy-Item $coreutils.FullName $stageExe
foreach ($dll in $requiredDlls) {
    Copy-Item (Join-Path $BinariesDir $dll) $libDir
}
Copy-Item (Join-Path $BinariesDir 'nmap-data') $Stage -Recurse
Copy-Item (Join-Path $BinariesDir 'vim-runtime') $Stage -Recurse
Copy-Item (Join-Path $BinariesDir 'ca-bundle.crt') $Stage

# ---------------------------------------------------------------- 执行 harness --

# 单次进程探针：独立进程执行（进程内 exit 的 sd/wget --version 在子进程里无害），
# 重定向三流 + 超时杀树。退出码在 Dispose 之前取（Dispose 后访问会抛
# ObjectDisposedException）。
function Invoke-Probe {
    param(
        [Parameter(Mandatory)][string]$FileName,
        [string[]]$Argv = @(),
        [string]$StdinText = '',
        [string]$WorkDir,
        [int]$TimeoutSeconds
    )
    $psi = [System.Diagnostics.ProcessStartInfo]::new()
    $psi.FileName = $FileName
    foreach ($a in $Argv) { [void]$psi.ArgumentList.Add($a) }
    $psi.WorkingDirectory = $WorkDir
    $psi.UseShellExecute = $false
    $psi.RedirectStandardInput = $true
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $psi.StandardOutputEncoding = [Text.Encoding]::UTF8
    $psi.StandardErrorEncoding = [Text.Encoding]::UTF8

    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    $p = [System.Diagnostics.Process]::Start($psi)
    $timedOut = $false
    $code = -1
    $stdout = ''
    $stderr = ''
    try {
        if ($StdinText) { $p.StandardInput.Write($StdinText) }
        $p.StandardInput.Close()
        $outTask = $p.StandardOutput.ReadToEndAsync()
        $errTask = $p.StandardError.ReadToEndAsync()
        if (-not $p.WaitForExit($TimeoutSeconds * 1000)) {
            $timedOut = $true
            try { $p.Kill($true) } catch { }
            $p.WaitForExit()
        }
        if ($p.HasExited) { $code = $p.ExitCode }
        try { $stdout = $outTask.Result } catch { }
        try { $stderr = $errTask.Result } catch { }
    } finally {
        $sw.Stop()
        if (-not $p.HasExited) {
            try { $p.Kill($true) } catch { }
        }
        $p.Dispose()
    }
    [pscustomobject]@{
        Exit     = $code
        Stdout   = [string]$stdout
        Stderr   = [string]$stderr
        Ms       = $sw.ElapsedMilliseconds
        TimedOut = $timedOut
    }
}

# 每行探针独立的种子工作目录（文件族状态互不串扰；短名/子命令两轮也互不影响）：
#   a.txt = "x\ny\n"   b.txt = "x\nz\n"   c.txt = "c\n"   probe_rmdir\（空目录）
function New-SeedDir([string]$Name) {
    $d = Join-Path $workRoot $Name
    New-Item -ItemType Directory -Path $d -Force | Out-Null
    Set-Content -Path (Join-Path $d 'a.txt') -Value "x`ny`n" -NoNewline
    Set-Content -Path (Join-Path $d 'b.txt') -Value "x`z`n" -NoNewline
    Set-Content -Path (Join-Path $d 'c.txt') -Value "c`n" -NoNewline
    New-Item -ItemType Directory -Path (Join-Path $d 'probe_rmdir') -Force | Out-Null
    $d
}

# --------------------------------------------------------------- 探针覆写表 --
# 字段：argv（参数数组）/ stdin（标准输入文本）/ exit（期望退出码：int 或
# 'nonnull'）/ out（stdout 必含子串，字符串或数组=每项都须命中；内嵌 "`n"
# 可做顺序敏感断言）/ nonempty（stdout 非空即可）/ note（报告备注）。
# 未列出的命令默认探针 = --version 且退出 0（版本通路本身也是分发冒烟）。
#
# 断言常量依据：md5/sha256 为 "hello\n" 的公开测试向量；base32/base64 族
# 子串取编码输出的稳定前缀（不依赖 padding 风格）。
$LF = "`n"
$overrides = @{
    # ---- 文本流族（stdin -> stdout）----
    'cat'       = @{ argv = @(); stdin = 'hello'; out = 'hello' }
    'head'      = @{ argv = @('-n', '1'); stdin = "aa${LF}bb${LF}"; out = 'aa' }
    'tail'      = @{ argv = @('-n', '1'); stdin = "aa${LF}bb${LF}"; out = 'bb' }
    'grep'      = @{ argv = @('zz'); stdin = "zz-line${LF}other${LF}"; out = 'zz-line' }
    'sed'       = @{ argv = @('s/bbb/ccc/'); stdin = "aaa bbb${LF}"; out = 'aaa ccc' }
    'tr'        = @{ argv = @('a-z', 'A-Z'); stdin = 'hello'; out = 'HELLO' }
    'cut'       = @{ argv = @('-d', ',', '-f', '2'); stdin = 'x,y'; out = 'y' }
    'wc'        = @{ argv = @('-l'); stdin = "a${LF}b${LF}"; out = '2' }
    'sort'      = @{ argv = @('-n'); stdin = "3${LF}1${LF}2${LF}"; out = "1${LF}2${LF}3" }
    'uniq'      = @{ argv = @(); stdin = "a${LF}a${LF}b${LF}"; out = "a${LF}b" }
    'tac'       = @{ argv = @(); stdin = "aa${LF}bb${LF}"; out = "bb${LF}aa" }
    'nl'        = @{ argv = @(); stdin = "x${LF}y${LF}"; out = "2`ty"; note = 'line numbering' }
    'od'        = @{ argv = @('-An', '-tx1'); stdin = 'A'; out = '41' }
    'tee'       = @{ argv = @('probe_tee.txt'); stdin = 'tee_out'; out = 'tee_out' }
    'paste'     = @{ argv = @('-d', ',', 'a.txt', 'b.txt'); out = 'x,x' }
    'sd'        = @{ argv = @('z', 'w'); stdin = 'zz'; out = 'ww' }
    'fold'      = @{ argv = @('-w', '2'); stdin = 'abcd'; out = "ab${LF}cd" }
    # ---- echo/expr/seq/factor/date ----
    'echo'      = @{ argv = @('hello'); out = 'hello' }
    'printf'    = @{ argv = @('%s-%s', 'a', 'b'); out = 'a-b' }
    'expr'      = @{ argv = @('2', '+', '3'); out = '5' }
    'seq'       = @{ argv = @('2', '4'); out = @('2', '3', '4') }
    'factor'    = @{ argv = @('12'); out = @('2', '3') }
    'date'      = @{ argv = @('+%Y'); out = '20'; note = 'format pipeline (year prefix)' }
    # ---- 编码/哈希（已知测试向量）----
    'base32'    = @{ argv = @(); stdin = 'hello'; out = 'NBSWY3DP' }
    'base64'    = @{ argv = @(); stdin = 'hello'; out = 'aGVsbG8' }
    'basenc'    = @{ argv = @('--base64'); stdin = 'hello'; out = 'aGVsbG8' }
    'base32hex' = @{ argv = @(); stdin = 'hello'; out = 'D1IMOR3F' }
    'base64url' = @{ argv = @(); stdin = 'hello'; out = 'aGVsbG8' }
    'hex'       = @{ argv = @(); stdin = 'hello'; out = '68656c6c6f' }
    'hexupper'  = @{ argv = @(); stdin = 'hello'; out = '68656C6C6F' }
    'md5sum'    = @{ argv = @(); stdin = "hello${LF}"; out = 'b1946ac92492d2347c6235b4d2611184' }
    'sha256sum' = @{ argv = @(); stdin = "hello${LF}"; out = '5891b5b522d5df086d0ff0b110fbd9d21bb4fc7163af34d08286a2e846f6be03' }
    'urlencode' = @{ argv = @('a b'); out = 'a%20b' }
    'urldecode' = @{ argv = @('a%20b'); out = 'a b' }
    # ---- 文件族（每行独立种子目录，真实操作文件系统）----
    'mkdir'     = @{ argv = @('probe_mkdir'); note = 'dir created' }
    'rmdir'     = @{ argv = @('probe_rmdir'); note = 'seeded empty dir removed' }
    'rm'        = @{ argv = @('c.txt'); note = 'seeded file removed' }
    'cp'        = @{ argv = @('a.txt', 'a_copy.txt') }
    'mv'        = @{ argv = @('c.txt', 'c_moved.txt') }
    'ln'        = @{ argv = @('a.txt', 'a_link.txt'); note = 'hard link' }
    'link'      = @{ argv = @('a.txt', 'a_link2.txt'); note = 'hard link' }
    'unlink'    = @{ argv = @('c.txt') }
    'touch'     = @{ argv = @('probe_touch.txt') }
    'truncate'  = @{ argv = @('-s', '0', 'a.txt') }
    'shred'     = @{ argv = @('c.txt') }
    'split'     = @{ argv = @('-l', '1', 'a.txt', 'probe_split_') }
    'mktemp'    = @{ argv = @('probe_XXXXXX'); out = 'probe_' }
    'basename'  = @{ argv = @('/x/y/z.txt'); out = 'z.txt' }
    'dirname'   = @{ argv = @('/x/y/z.txt'); out = '/x' }
    'realpath'  = @{ argv = @('a.txt'); out = 'a.txt' }
    'find'      = @{ argv = @('-name', 'a.txt'); out = 'a.txt' }
    'xargs'     = @{ argv = @('echo'); stdin = 'zz'; out = 'zz' }
    'tar'       = @{ argv = @('-cf', 'probe.tar', 'a.txt'); note = 'tarball created' }
    'du'        = @{ argv = @('.') }
    'df'        = @{ argv = @('.') }
    'diff'      = @{ argv = @('a.txt', 'b.txt'); exit = 1; note = 'GNU: 1 = files differ' }
    'cmp'       = @{ argv = @('a.txt', 'a.txt'); note = 'identical -> 0' }
    'sdiff'     = @{ argv = @('a.txt', 'b.txt'); exit = 1; note = 'GNU: 1 = differ' }
    # ---- 派生子进程族 ----
    'timeout'   = @{ argv = @('2', 'cmd', '/c', 'ver'); note = 'child spawn' }
    'nice'      = @{ argv = @('cmd', '/c', 'ver'); note = 'child spawn' }
    'nohup'     = @{ argv = @('cmd', '/c', 'ver'); note = 'child spawn, stdout->nohup.out' }
    'stdbuf'    = @{ argv = @('-o0', 'cmd', '/c', 'ver'); note = 'child spawn; GNU form -o0 attached' }
    'sleep'     = @{ argv = @('0') }
    'ps'        = @{ argv = @(); nonempty = $true; note = 'process listing' }
    # ---- 视图/搜索工具 ----
    'eza'       = @{ argv = @('--oneline', 'a.txt'); out = 'a.txt' }
    'tree'      = @{ argv = @('a.txt'); out = 'a.txt' }
    'bat'       = @{ argv = @('--paging=never', 'a.txt'); out = 'y' }
    'rg'        = @{ argv = @('zz'); stdin = 'zz-line'; out = 'zz-line' }
    'fd'        = @{ argv = @('a.txt'); out = 'a.txt' }
    'fzf'       = @{ argv = @('-f', 'zz'); stdin = "zz-line${LF}no${LF}"; out = 'zz-line'; note = 'filter mode (non-interactive)' }
    'parallel'  = @{ argv = @('echo'); stdin = 'hello'; out = 'hello' }
    # more 在非 tty stdin 下会挂死（CI 实测 60s 超时）——uutils more 支持
    # --version（版本扫描单测在案），换版本探针
    'more'      = @{ argv = @('--version'); note = 'stdin pager hangs w/o tty (CI-observed); version probe instead' }
    # ---- CLI 语义例外（coreutils.rs 版本扫描单测的已知例外）----
    'false'     = @{ exit = 1; note = 'GNU semantics: always 1, even with --version' }
    'test'      = @{ argv = @('a', '=', 'a'); note = 'no --version support' }
    'which'     = @{ argv = @('cmd'); note = 'no --version support; PATH lookup' }
    # ---- DLL 家族功能用例 ----
    'jq'        = @{ argv = @('-n', '1+1'); out = '2' }
    'yq'        = @{ argv = @('-n', '.a = "b"'); out = 'a: b' }
    'bc'        = @{ argv = @(); stdin = "1+1${LF}"; out = '2' }
    'qalc'      = @{ argv = @('-t', '5+2'); out = '7' }
    'openssl'   = @{ argv = @('version'); out = 'OpenSSL' }
    'awk'       = @{ argv = @('BEGIN { print 40+2 }'); out = '42' }
    'vim'       = @{ out = 'VIM' }
    'nmap'      = @{ out = 'Nmap' }
    'ncat'      = @{ outAny = 'Ncat'; note = 'version banner may go to stderr' }
    'nping'     = @{ nonempty = $true }
    'telnet'    = @{ argv = @(); exit = 'nonnull'; note = 'no args -> usage, nonzero' }
}

# OWN_ARGV 家族：解析器直读自身进程 argv / Win32 命令行，仅短名模式可用
$ownArgv = @('awk', 'bat', 'fd', 'onefetch', 'parallel', 'rg', 'vim')

function Get-ProbeSpec([string]$cmd) {
    $spec = @{ argv = @('--version'); exit = 0 }
    if ($overrides.ContainsKey($cmd)) {
        foreach ($k in $overrides[$cmd].Keys) { $spec[$k] = $overrides[$cmd][$k] }
    }
    $spec
}

# 单命令单模式执行 + 判定，返回报告行。
function Invoke-One {
    param([string]$Cmd, [string]$Mode)
    $spec = Get-ProbeSpec $Cmd
    $exe = if ($Mode -eq 'shortname') {
        Join-Path $toolsDir "$Cmd.exe"
    } else {
        $stageExe
    }
    $argv = if ($Mode -eq 'shortname') { [string[]]$spec['argv'] }
    else { , @($Cmd) + @($spec['argv']) }
    $stdin = [string]$spec['stdin']
    if ($null -eq $stdin) { $stdin = '' }
    $wd = New-SeedDir ("{0}-{1}" -f $Mode, $Cmd)

    $r = Invoke-Probe -FileName $exe -Argv $argv -StdinText $stdin `
        -WorkDir $wd -TimeoutSeconds $TimeoutSec

    $verdict = 'PASS'
    $notes = @()
    if ($spec['note']) { $notes += [string]$spec['note'] }
    if ($r.TimedOut) {
        $verdict = 'FAIL'
        $notes += ("TIMEOUT after {0}s (killed)" -f $TimeoutSec)
    } else {
        $wantExit = $spec['exit']
        $exitOk = if ('nonnull' -eq "$wantExit") { $r.Exit -ne 0 }
        else { $r.Exit -eq [int]$wantExit }
        if (-not $exitOk) {
            $verdict = 'FAIL'
            $notes += ("exit {0}, want {1}" -f $r.Exit, $wantExit)
        }
        # 断言前把 CRLF 归一成 LF：顺序敏感断言（sort/uniq/tac/fold）在工具
        # 输出平台换行时不能误判（GNU 语义是 \n，但按 \r\n 防御性兼容）
        $outNorm = ([string]$r.Stdout) -replace "`r`n", "`n"
        if ($spec['out']) {
            foreach ($want in @($spec['out'])) {
                if (-not $outNorm.Contains([string]$want)) {
                    $verdict = 'FAIL'
                    $notes += ("stdout missing '{0}'" -f ($want -replace "`r", '\r' -replace "`n", '\n'))
                }
            }
        }
        if ($spec['outAny']) {
            # outAny：stdout+stderr 合并后判断子串（有些工具的版本横幅走 stderr）
            $anyNorm = ($outNorm + "`n" + (([string]$r.Stderr) -replace "`r`n", "`n"))
            foreach ($want in @($spec['outAny'])) {
                if (-not $anyNorm.Contains([string]$want)) {
                    $verdict = 'FAIL'
                    $notes += ("output missing '{0}'" -f $want)
                }
            }
        }
        if ($spec['nonempty'] -and -not $outNorm.Trim()) {
            $verdict = 'FAIL'
            $notes += 'stdout empty'
        }
        if ($r.Exit -lt 0) {
            $notes += ("CRASH 0x{0:X8}" -f $r.Exit)
        }
    }
    $firstOut = (([string]$r.Stdout -split "`r?`n") | Where-Object { $_ } |
        Select-Object -First 1)
    if (-not $firstOut) {
        $firstOut = (([string]$r.Stderr -split "`r?`n") | Where-Object { $_ } |
            Select-Object -First 1)
    }
    if ($firstOut) { $firstOut = $firstOut.Trim() }
    if ($firstOut.Length -gt 80) { $firstOut = $firstOut.Substring(0, 80) + '...' }

    [pscustomobject]@{
        Cmd = $Cmd; Mode = $Mode
        Argv = ($argv -join ' ')
        Exit = $r.Exit; Want = "$($spec['exit'])"
        Ms = $r.Ms; Verdict = $verdict
        Note = ($notes -join '; ')
        Out1 = [string]$firstOut
        Stdout = [string]$r.Stdout; Stderr = [string]$r.Stderr
    }
}

# ------------------------------------------------------------- 命令清单发现 --

# 受测二进制自述：`coreutils.exe`（无参）在 stderr 打印
#   coreutils (uutils) + veloterm extras: N commands
#   usage: coreutils <command> [args...]
#   commands: <空格分隔全表>
# （用 harness 而非 `2>&1`：$ErrorActionPreference='Stop' 下原生命令 stderr
#  重定向可能被当作终止错误）
$disc = Invoke-Probe -FileName $stageExe -Argv @() -StdinText '' `
    -WorkDir $workRoot -TimeoutSeconds $TimeoutSec
$commandsLine = (([string]$disc.Stderr) -split "`r?`n") |
    Where-Object { $_ -like 'commands: *' } | Select-Object -First 1
if (-not $commandsLine) {
    Say 'FATAL: coreutils.exe printed no "commands:" line (dispatch entry broken?)'
    Say ([string]$disc.Stderr)
    exit 1
}
$commands = @($commandsLine.Substring('commands: '.Length) -split '\s+' |
    Where-Object { $_ })
if ($commands.Count -lt 100) {
    Say ("FATAL: command list suspiciously short: {0}" -f $commands.Count)
    exit 1
}
Say ("commands  : {0} (from coreutils.exe self-report)" -f $commands.Count)

# 硬链接短名：全部命令（安装后 src/tools/hardlinks.rs 做的同一件事）
foreach ($cmd in $commands) {
    New-Item -ItemType HardLink `
        -Path (Join-Path $toolsDir "$cmd.exe") `
        -Target $stageExe | Out-Null
}

# -------------------------------------------------------------------- 扫描 --

$rows = New-Object System.Collections.Generic.List[object]

# 第 0 行：无参自述（usage 列表通路；GNU 惯例退出非零）
$selfOk = (-not $disc.TimedOut) -and ($disc.Exit -ne 0) -and
    (([string]$disc.Stderr).Contains('commands:'))
$rows.Add([pscustomobject]@{
    Cmd = '(coreutils no-args)'; Mode = 'self-report'
    Argv = ''; Exit = $disc.Exit; Want = 'nonnull'
    Ms = $disc.Ms; Verdict = $(if ($selfOk) { 'PASS' } else { 'FAIL' })
    Note = $(if (-not $selfOk) { 'usage/self-report line missing' })
    Out1 = 'commands: ' + $commands.Count; Stdout = ''; Stderr = [string]$disc.Stderr
})

$totalSw = [System.Diagnostics.Stopwatch]::StartNew()
Say ("`n-- pass 1: shortname mode ({0} commands) --" -f $commands.Count)
foreach ($cmd in $commands) {
    try { $row = Invoke-One -Cmd $cmd -Mode 'shortname' }
    catch {
        $row = [pscustomobject]@{ Cmd = $cmd; Mode = 'shortname'; Argv = ''; Exit = -9999; Want = '0';
            Ms = 0; Verdict = 'FAIL'; Note = "HARNESS ERROR: $($_.Exception.Message)";
            Out1 = ''; Stdout = ''; Stderr = [string]$_.Exception.Message }
    }
    $rows.Add($row)
    Say ("  [{0}] {1,-12} exit={2,-4} {3,5} ms  {4}" -f `
        $row.Verdict, $cmd, $row.Exit, $row.Ms, $row.Note)
}

$subCmds = @($commands | Where-Object { $ownArgv -notcontains $_ })
Say ("`n-- pass 2: subcommand mode ({0} commands, OWN_ARGV excluded) --" -f `
    $subCmds.Count)
foreach ($cmd in $subCmds) {
    try { $row = Invoke-One -Cmd $cmd -Mode 'subcommand' }
    catch {
        $row = [pscustomobject]@{ Cmd = $cmd; Mode = 'subcommand'; Argv = ''; Exit = -9999; Want = '0';
            Ms = 0; Verdict = 'FAIL'; Note = "HARNESS ERROR: $($_.Exception.Message)";
            Out1 = ''; Stdout = ''; Stderr = [string]$_.Exception.Message }
    }
    $rows.Add($row)
    Say ("  [{0}] {1,-12} exit={2,-4} {3,5} ms  {4}" -f `
        $row.Verdict, $cmd, $row.Exit, $row.Ms, $row.Note)
}
$totalSw.Stop()

# -------------------------------------------------------------------- 报告 --

$failRows = @($rows | Where-Object { $_.Verdict -ne 'PASS' })
$passCount = $rows.Count - $failRows.Count

function Trunc([string]$s, [int]$n) {
    $t = $s
    if ($t.Length -gt $n) { $t = $t.Substring(0, $n) + '...' }
    $t
}

$sha = $env:GITHUB_SHA
if ($sha) { $sha = $sha.Substring(0, [Math]::Min(10, $sha.Length)) }
$md = New-Object System.Collections.Generic.List[string]
$md.Add('# verify-win-commands report')
$md.Add('')
$md.Add('- date      : {0} (UTC)' -f ([DateTime]::UtcNow.ToString('yyyy-MM-dd HH:mm:ss')))
$md.Add('- commit    : {0}' -f $(if ($sha) { $sha } else { '(local)' }))
$md.Add(('- runner    : {0} {1}' -f $env:RUNNER_OS, $env:ImageOS))
$md.Add('- binaries  : {0}' -f $BinariesDir)
$md.Add('- commands  : {0} (self-reported by coreutils.exe)' -f $commands.Count)
$md.Add(('- probes    : {0} rows = 1 self-report + {1} shortname + {2} subcommand' -f `
    $rows.Count, $commands.Count, $subCmds.Count))
$md.Add(('- wall time : {0:N1} s (timeout {1}s per command)' -f `
    $totalSw.Elapsed.TotalSeconds, $TimeoutSec))
$md.Add('')
$md.Add(('**RESULT: {0}** -- {1}/{2} passed' -f `
    $(if ($failRows.Count) { 'FAIL' } else { 'PASS' }), $passCount, $rows.Count))
$md.Add('')
$md.Add('| # | command | mode | probe argv | exit | want | ms | verdict | note |')
$md.Add('| --: | -- | -- | -- | --: | --: | --: | -- | -- |')
$i = 0
foreach ($row in $rows) {
    $i++
    $argvCell = ([string]$row.Argv -replace '\|', '\|')
    $noteCell = ([string]$row.Note -replace '\|', '\|')
    $md.Add(('| {0} | `{1}` | {2} | `{3}` | {4} | {5} | {6} | **{7}** | {8} |' -f `
        $i, $row.Cmd, $row.Mode, $argvCell, $row.Exit, $row.Want, $row.Ms,
        $row.Verdict, $noteCell))
}

if ($failRows.Count) {
    $md.Add('')
    $md.Add('## failure details')
    foreach ($row in $failRows) {
        $md.Add('')
        $md.Add(('### {0} ({1}) -- {2}' -f $row.Cmd, $row.Mode, $row.Note))
        $md.Add('```')
        $md.Add(('argv   : {0} {1}' -f $row.Cmd, $row.Argv))
        $md.Add(('exit   : {0} (want {1})' -f $row.Exit, $row.Want))
        $md.Add(('stdout : {0}' -f (Trunc $row.Stdout 1500)))
        $md.Add(('stderr : {0}' -f (Trunc $row.Stderr 1500)))
        $md.Add('```')
    }
}

$mdText = $md -join "`r`n"
Set-Content -Path $ReportPath -Value $mdText -Encoding utf8

Say ''
Say ("-- report written: {0}" -f $ReportPath)
Say ("RESULT: {0} -- {1}/{2} probes passed ({3:N1}s)" -f `
    $(if ($failRows.Count) { 'FAIL' } else { 'PASS' }), $passCount, $rows.Count,
    $totalSw.Elapsed.TotalSeconds)
if ($failRows.Count) {
    Say 'failed probes:'
    foreach ($row in $failRows) {
        Say ("  {0,-22} {1,-12} {2}" -f $row.Cmd, $row.Mode, $row.Note)
    }
}

if ($env:GITHUB_STEP_SUMMARY) {
    ("`r`n" + $mdText + "`r`n") |
        Out-File -FilePath $env:GITHUB_STEP_SUMMARY -Append -Encoding utf8
}
if ($failRows.Count) { exit 1 }
exit 0
