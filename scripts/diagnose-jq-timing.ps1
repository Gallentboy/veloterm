# diagnose-jq-timing.ps1 -- split jq's startup cost into four segments on Windows.
#
# 背景（用户机 D:\software\veloterm\tools 实测）：
#   * in-main 探针（VELOTERM_JQ_TIMING）：`jq -n 1+1` 内部仅 1.9~3.1ms；
#   * 但进程寿命（内核 StartTime->ExitTime）稳定 454ms，`--version` 仅 27~45ms；
#     三种计时口径（Measure-Command / Start-Process+Stopwatch / cmd 中转）一致，
#     Defender 实时保护关闭、无 MOTW 标记仍复现。
#   => 缺口在 umain 之外：pre_main（进程创建→入口）或 post_probe（退出路径+
#      内核回收），且只在「编译过 builtin.jq」的路径上出现。
#
# 本脚本按「同一次运行」把总寿命切成四段（需配套探针构建，见 vendor/jq/src/main.c）：
#   pre_main   进程创建 -> umain 入口            （探针：GetProcessTimes 创建时刻）
#   in_main    入口 -> 探针点                    （探针：QPC 分段）
#   post_probe 探针点 -> 进程消亡                （脚本：life - uptime）
#   life       进程创建 -> 进程消亡              （脚本：内核时间戳）
# 外加内存/CPU 快照（ws/peak_ws/private/cpu/handles）与退出路径二分：
#   VELOTERM_JQ_EXIT_MODE=quick|terminate   _exit（跳过 atexit/stdio）/
#                                           TerminateProcess（再跳过 CRT 与加载器收尾）
#   VELOTERM_JQ_EXIT_AFTER=compile          编译后即退（隔离 exec 段）
#
# 用法：
#   pwsh -NoProfile -File scripts/diagnose-jq-timing.ps1 `
#       -Coreutils src-tauri/binaries/coreutils-x86_64-pc-windows-msvc.exe
#   powershell -ExecutionPolicy Bypass -File .\diagnose-jq-timing.ps1 -Coreutils .\coreutils.exe
# 可选：-Runs <n>（默认 5）/ -Jq17 <path>（官方对照，默认自动找或下载）/ -SkipDownload
#
# 说明：字符串字面量全 ASCII（PS 5.1 对无 BOM UTF-8 中文有编码坑），中文只在注释里。

[CmdletBinding()]
param(
    [string]$Coreutils,
    [string]$Jq17,
    [int]$Runs = 5,
    [switch]$SkipDownload
)

$ErrorActionPreference = 'Continue'
$ProgressPreference = 'SilentlyContinue'
try { [Console]::OutputEncoding = [Text.Encoding]::UTF8 } catch { }

$log = New-Object System.Collections.Generic.List[string]
function Say([string]$s) { Write-Host $s; $script:log.Add([string]$s) }
function Head([string]$s) { Say ''; Say ('==== ' + $s + ' ====') }

# ---------------------------------------------------------------- locate exe
if (-not $Coreutils) {
    $cands = @()
    $cands += (Join-Path (Get-Location).ProviderPath 'coreutils.exe')
    if ($PSScriptRoot) {
        $cands += (Join-Path $PSScriptRoot 'coreutils.exe')
        $cands += (Join-Path (Split-Path $PSScriptRoot -Parent) 'src-tauri/binaries/coreutils-x86_64-pc-windows-msvc.exe')
    }
    foreach ($c in $cands) { if (Test-Path $c) { $Coreutils = $c; break } }
}
if (-not $Coreutils -or -not (Test-Path $Coreutils)) {
    Write-Host 'coreutils.exe not found: pass -Coreutils <path>'
    exit 1
}
$Coreutils = (Resolve-Path $Coreutils).Path

# Scratch dir with the shipped layout: coreutils.exe + jq.exe hard link (tools\jq.exe).
$work = Join-Path $env:TEMP 'veloterm-jq-diag'
if (Test-Path $work) { Remove-Item $work -Recurse -Force -ErrorAction SilentlyContinue }
New-Item -ItemType Directory -Path $work -Force | Out-Null
$cu = Join-Path $work 'coreutils.exe'
Copy-Item $Coreutils $cu -Force
$jq = Join-Path $work 'jq.exe'
try { New-Item -ItemType HardLink -Path $jq -Target $cu -Force | Out-Null }
catch { Copy-Item $cu $jq -Force }
$echoExe = Join-Path $work 'echo.exe'
try { New-Item -ItemType HardLink -Path $echoExe -Target $cu -Force | Out-Null } catch { }

Say '=== diagnose-jq-timing (four-segment breakdown) ==='
Say ("coreutils : {0} ({1:N1} MB)" -f $Coreutils, ((Get-Item $Coreutils).Length / 1MB))
Say ("workdir   : {0}" -f $work)
Say ("runs      : {0}" -f $Runs)

# ---------------------------------------------------------------- environment
Head 'environment'
try {
    $st = Get-MpComputerStatus -ErrorAction Stop
    $mp = Get-MpPreference -ErrorAction Stop
    $excl = if ($mp.ExclusionPath) { $mp.ExclusionPath -join '; ' } else { '(none)' }
    Say ("Defender realtime={0} antispyware={1} exclusions={2}" -f `
            $st.RealTimeProtectionEnabled, $st.AntispywareEnabled, $excl)
} catch { Say 'Defender query unavailable (non-Defender AV or no permission)' }
Say ("MOTW coreutils.exe: {0}" -f [bool](Get-Item $cu -Stream Zone.Identifier -ErrorAction SilentlyContinue))
try {
    $cpu = Get-CimInstance Win32_Processor | Select-Object -First 1
    Say ("CPU: {0}  {1}C/{2}T" -f $cpu.Name, $cpu.NumberOfCores, $cpu.NumberOfLogicalProcessors)
} catch { }
try { Say ("OS : {0}" -f (Get-CimInstance Win32_OperatingSystem).Caption) } catch { }
# 第三方安全软件（Defender 关闭时仍可能有国产 AV 在拦进程创建/退出）
try {
    $avs = @(Get-CimInstance -Namespace root/SecurityCenter2 -ClassName AntiVirusProduct -ErrorAction Stop)
    if ($avs.Count -gt 0) {
        foreach ($a in $avs) { Say ("AV : {0}  state=0x{1:X}" -f $a.displayName, $a.productState) }
    } else { Say 'AV : (none registered in SecurityCenter2)' }
} catch { Say 'AV : SecurityCenter2 query unavailable' }

# ---------------------------------------------------------------- helpers
function Invoke-Probe([string[]]$jqArgs, [hashtable]$extraEnv) {
    # 抓 stderr 的探针行：Start-Process + 文件重定向（PS 5.1 直连 stderr 会包
    # NativeCommandError；内嵌引号经 cmd 中转会被 \" 转义）。
    $errFile = Join-Path $work 'probe-err.txt'
    $outFile = Join-Path $work 'probe-out.txt'
    Remove-Item $errFile, $outFile -ErrorAction SilentlyContinue
    $saved = @{}
    $env:VELOTERM_JQ_TIMING = '1'
    if ($extraEnv) {
        foreach ($k in $extraEnv.Keys) {
            $saved[$k] = [Environment]::GetEnvironmentVariable($k)
            Set-Item -Path ("Env:" + $k) -Value $extraEnv[$k]
        }
    }
    Start-Process -FilePath $jq -ArgumentList $jqArgs -NoNewWindow -Wait `
        -RedirectStandardOutput $outFile -RedirectStandardError $errFile
    Remove-Item Env:VELOTERM_JQ_TIMING -ErrorAction SilentlyContinue
    if ($extraEnv) {
        foreach ($k in $extraEnv.Keys) {
            if ($null -ne $saved[$k]) { Set-Item -Path ("Env:" + $k) -Value $saved[$k] }
            else { Remove-Item -Path ("Env:" + $k) -ErrorAction SilentlyContinue }
        }
    }
    if (Test-Path $errFile) { return ([string](Get-Content $errFile -Raw -Encoding UTF8)).Trim() }
    return ''
}

# 一次运行同时拿到：探针（pre_main/in_main/内存）+ 内核寿命 -> 四段分解
$segRows = New-Object System.Collections.Generic.List[object]
function Measure-Segments([string]$name, [string[]]$jqArgs, [hashtable]$extraEnv) {
    $preMain = @(); $inMain = @(); $uptimes = @(); $lives = @()
    $peakWs = @(); $priv = @(); $cpuMs = @(); $codes = @()
    $lastProbe = ''
    for ($i = 0; $i -lt $Runs; $i++) {
        $errFile = Join-Path $work 'seg-err.txt'
        $outFile = Join-Path $work 'seg-out.txt'
        Remove-Item $errFile, $outFile -ErrorAction SilentlyContinue
        $saved = @{}
        $env:VELOTERM_JQ_TIMING = '1'
        if ($extraEnv) {
            foreach ($k in $extraEnv.Keys) {
                $saved[$k] = [Environment]::GetEnvironmentVariable($k)
                Set-Item -Path ("Env:" + $k) -Value $extraEnv[$k]
            }
        }
        $p = Start-Process -FilePath $jq -ArgumentList $jqArgs -NoNewWindow -PassThru `
            -RedirectStandardOutput $outFile -RedirectStandardError $errFile
        $life = $null
        try { $p.WaitForExit(); $life = ($p.ExitTime - $p.StartTime).TotalMilliseconds } catch { }
        $code = $null; try { $code = [int]$p.ExitCode } catch { }
        Remove-Item Env:VELOTERM_JQ_TIMING -ErrorAction SilentlyContinue
        if ($extraEnv) {
            foreach ($k in $extraEnv.Keys) {
                if ($null -ne $saved[$k]) { Set-Item -Path ("Env:" + $k) -Value $saved[$k] }
                else { Remove-Item -Path ("Env:" + $k) -ErrorAction SilentlyContinue }
            }
        }
        $raw = ''
        if (Test-Path $errFile) { $raw = [string](Get-Content $errFile -Raw -Encoding UTF8) }
        $lastProbe = $raw.Trim()
        $pm = 0.0; $um = 0.0; $inm = 0.0; $pw = 0.0; $pv = 0.0; $cp = 0.0
        if ($raw -match 'pre_main=(\d+)ms uptime=(\d+)ms') {
            $pm = [double]$Matches[1]; $um = [double]$Matches[2]
        }
        if ($raw -match 'total=([\d.\-]+)ms') { $inm = [double]$Matches[1] }
        if ($raw -match 'peak_ws=([\d.]+)MB') { $pw = [double]$Matches[1] }
        if ($raw -match 'private=([\d.]+)MB') { $pv = [double]$Matches[1] }
        if ($raw -match 'cpu=([\d.]+)ms') { $cp = [double]$Matches[1] }
        if ($null -eq $life) { $life = 0 }
        $preMain += $pm; $inMain += $inm; $uptimes += $um; $lives += [math]::Round($life, 1)
        $peakWs += $pw; $priv += $pv; $cpuMs += $cp
        $codes += $(if ($null -ne $code) { ('0x{0:X8}' -f [uint32]([int64]$code -band [int64]0xFFFFFFFF)) } else { 'n/a' })
    }
    $avg = { param($a) if ($a.Count -eq 0) { 0 } else { [math]::Round(($a | Measure-Object -Average).Average, 1) } }
    $post = @(); for ($i = 0; $i -lt $Runs; $i++) { $post += [math]::Round($lives[$i] - $uptimes[$i], 1) }
    $row = [pscustomobject]@{
        Case       = $name
        pre_main   = (& $avg $preMain)
        in_main    = (& $avg $inMain)
        uptime     = (& $avg $uptimes)
        post_probe = (& $avg $post)
        life       = (& $avg $lives)
        peak_ws_MB = (& $avg $peakWs)
        private_MB = (& $avg $priv)
        cpu_ms     = (& $avg $cpuMs)
        exitcodes  = (($codes | Select-Object -Unique) -join '/')
    }
    $script:segRows.Add($row)
    Say ("  {0,-26} pre={1,7} in={2,6} post={3,7} life={4,7}  peak_ws={5,7}MB private={6,7}MB cpu={7,6}ms exit={8}" -f `
            $row.Case, $row.pre_main, $row.in_main, $row.post_probe, $row.life,
            $row.peak_ws_MB, $row.private_MB, $row.cpu_ms, $row.exitcodes)
    if ($lastProbe) { Say ('      probe: ' + ($lastProbe -replace "`r?`n", ' | ')) }
}

function Measure-Wall([string]$name, [scriptblock]$body) {
    $ms = @()
    for ($i = 0; $i -lt $Runs; $i++) { $ms += [math]::Round((Measure-Command $body).TotalMilliseconds, 1) }
    Say ("  {0,-26} min={1,8} avg={2,8} max={3,8}  [{4}]" -f $name, `
            [math]::Round(($ms | Measure-Object -Minimum).Minimum, 1), `
            [math]::Round(($ms | Measure-Object -Average).Average, 1), `
            [math]::Round(($ms | Measure-Object -Maximum).Maximum, 1), ($ms -join ','))
}

# ---------------------------------------------------------------- cold start
Head 'cold start (before warmup)'
Say ("  cold jq -n 1+1 = {0} ms" -f [int](Measure-Command { & $jq -n 1+1 | Out-Null }).TotalMilliseconds)

# ---------------------------------------------------------------- probe lines
Head 'probe lines (single runs, VELOTERM_JQ_TIMING=1)'
foreach ($c in @(
        @{ n = 'jq -n 1+1';                    a = @('-n', '1+1') },
        @{ n = 'jq -n .';                      a = @('-n', '.') },
        @{ n = 'jq --version';                 a = @('--version') },
        @{ n = 'jq -n 1+ (syntax error)';      a = @('-n', '1+') },
        @{ n = 'jq -n range(100000)';          a = @('-n', 'range(100000)') },
        @{ n = 'jq -n 1+1 exit-after-compile'; a = @('-n', '1+1'); e = @{ VELOTERM_JQ_EXIT_AFTER = 'compile' } },
        @{ n = 'jq -n 1+1 exit-mode=terminate'; a = @('-n', '1+1'); e = @{ VELOTERM_JQ_EXIT_MODE = 'terminate' } })) {
    $raw = Invoke-Probe $c.a $c.e
    Say ("  [{0}]" -f $c.n)
    if (-not $raw) { Say '    (no probe output)' }
    else { foreach ($ln in ($raw -split "`r?`n")) { if ($ln.Trim()) { Say ('    ' + $ln.Trim()) } } }
}

# ---------------------------------------------------------------- four segments
Head 'four-segment breakdown (same run: probe + kernel lifetime)'
Say '  pre_main = process-create -> entry | in_main = entry -> probe point | post_probe = probe point -> death | life = total'
Measure-Segments 'jq -n 1+1'                      @('-n', '1+1')
Measure-Segments 'jq -n .'                        @('-n', '.')
Measure-Segments 'jq --version'                   @('--version')
Measure-Segments 'jq -n 1+ (syntax error)'        @('-n', '1+')
Measure-Segments 'jq -n 1+1 exit-after-compile'   @('-n', '1+1') @{ VELOTERM_JQ_EXIT_AFTER = 'compile' }
Measure-Segments 'jq -n 1+1 exit-mode=quick'      @('-n', '1+1') @{ VELOTERM_JQ_EXIT_MODE = 'quick' }
Measure-Segments 'jq -n 1+1 exit-mode=terminate'  @('-n', '1+1') @{ VELOTERM_JQ_EXIT_MODE = 'terminate' }
Measure-Segments 'jq --version exit-mode=terminate' @('--version') @{ VELOTERM_JQ_EXIT_MODE = 'terminate' }

# ---------------------------------------------------------------- controls
Head 'control: same binary, other commands (coreutils.exe subcommand mode)'
foreach ($c in @(
        @{ n = 'coreutils echo hi';   a = @('echo', 'hi') },
        @{ n = 'coreutils ls';        a = @('ls') },
        @{ n = 'coreutils true';      a = @('true') },
        @{ n = 'coreutils jq -n 1+1'; a = @('jq', '-n', '1+1') })) {
    Measure-Wall $c.n { & $cu @($c.a) | Out-Null }
}

Head 'control: hard-link short name echo.exe (if created)'
if (Test-Path $echoExe) { Measure-Wall 'echo.exe hi' { & $echoExe hi | Out-Null } }

# ---------------------------------------------------------------- official jq
Head 'official jq 1.7.1 reference (jq17)'
if (-not $Jq17) {
    foreach ($cand in @('jq17.exe', 'jq17', 'jq-official.exe')) {
        foreach ($dir in @((Split-Path $Coreutils -Parent), $work, (Get-Location).ProviderPath)) {
            $p = Join-Path $dir $cand
            if (Test-Path $p) { $Jq17 = $p; break }
        }
        if ($Jq17) { break }
    }
}
if (-not $Jq17 -and -not $SkipDownload) {
    $dl = Join-Path $work 'jq17.exe'
    $url = 'https://github.com/jqlang/jq/releases/download/jq-1.7.1/jq-windows-amd64.exe'
    Say ('  downloading ' + $url)
    try { curl.exe -sSL --retry 2 -o $dl $url } catch { }
    if ((Test-Path $dl) -and (Get-Item $dl).Length -gt 1MB) { $Jq17 = $dl } else { Say '  download failed' }
}
if ($Jq17 -and (Test-Path $Jq17)) {
    Unblock-File $Jq17 -ErrorAction SilentlyContinue
    Say ("  reference: {0} ({1:N1} MB)" -f $Jq17, ((Get-Item $Jq17).Length / 1MB))
    Measure-Wall 'jq17 -n 1+1'    { & $Jq17 -n 1+1 | Out-Null }
    Measure-Wall 'jq17 -n .'      { & $Jq17 -n . | Out-Null }
    Measure-Wall 'jq17 --version' { & $Jq17 --version | Out-Null }
    # jq17 无探针：只测内核寿命（与 life 列同口径）
    foreach ($c in @(@{ n = 'jq17 -n 1+1'; a = @('-n', '1+1') }, @{ n = 'jq17 --version'; a = @('--version') })) {
        $lives = @()
        for ($i = 0; $i -lt $Runs; $i++) {
            $p = Start-Process -FilePath $Jq17 -ArgumentList $c.a -NoNewWindow -PassThru
            $life = $null
            try { $p.WaitForExit(); $life = ($p.ExitTime - $p.StartTime).TotalMilliseconds } catch { }
            if ($null -ne $life) { $lives += [math]::Round($life, 1) }
        }
        Say ("  {0,-26} life avg={1,8} ms  [{2}]" -f ('life:' + $c.n), `
                [math]::Round(($lives | Measure-Object -Average).Average, 1), ($lives -join ','))
    }
} else {
    Say '  official jq not available, section skipped'
}

# ---------------------------------------------------------------- crash evidence
Head 'crash evidence (Application-log events mentioning coreutils|jq)'
try {
    $since = (Get-Date).AddMinutes(-30)
    $evts = @(Get-WinEvent -FilterHashtable @{ LogName = 'Application'; StartTime = $since } -ErrorAction Stop |
        Where-Object { $_.Message -match 'coreutils|jq' })
    if ($evts.Count -gt 0) {
        foreach ($e in ($evts | Select-Object -First 6)) {
            Say ("  [{0}] {1}: {2}" -f $e.TimeCreated, $e.ProviderName, ($e.Message -split "`r?`n")[0])
        }
    } else { Say '  no Application-log events mentioning coreutils/jq in the last 30 minutes' }
} catch { Say ('  event log query unavailable: ' + $_.Exception.Message) }

# ---------------------------------------------------------------- summary
Head 'summary: four segments'
$segRows | Format-Table -AutoSize | Out-String | ForEach-Object { Say $_ }

Say @'

How to read (columns pre_main / in_main / post_probe / life):
* pre_main large and equal for `-n 1+1` and `--version` -> loader/process creation cost.
* post_probe large only for jq runs that compiled builtin.jq, and
    - exit-mode=terminate makes it fast  -> user-mode teardown (atexit / CRT / heap /
      loader DllMain). Fix path: skip teardown for a one-shot CLI.
    - exit-mode=quick also fast          -> atexit handlers or stdio flush.
    - terminate still slow               -> kernel address-space teardown (memory size).
      Cross-check peak_ws / private: large values support this.
* exit-after-compile fast but full run slow -> cost tied to exec/output stage.
* non-zero exit codes / WerFault events   -> the delay is a crash + WER handling,
      i.e. heap corruption during teardown (would need a fix, not a micro-opt).
* control commands (echo/ls/true) also slow -> not jq-specific.
* jq17 fast on the same machine while ours is slow -> our build (static CRT / MSVC
      heap usage) is the differentiator, not jq logic.
'@

$outFile = Join-Path $work 'diagnose-jq-timing.result.txt'
($log -join "`r`n") | Out-File -FilePath $outFile -Encoding utf8
Say ("saved: {0}" -f $outFile)
if ($env:GITHUB_STEP_SUMMARY) {
    ("`r`n## diagnose-jq-timing`r`n`r`n" + ($log -join "`r`n")) |
        Out-File -FilePath $env:GITHUB_STEP_SUMMARY -Append -Encoding utf8
}
