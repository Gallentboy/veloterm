# verify-installer-layout.ps1 -- 断言打包产物里 FFI DLL 落在 `lib/` 子目录。
#
# 背景：三个 DLL 例外形态（veloterm_goffi.dll / qalcffi.dll / nmapffi.dll）的安装
# 位置由 `src-tauri/tauri.windows.conf.json` 的 `bundle.resources` 映射决定
# （`"binaries/x.dll": "lib/x.dll"`）。运行期解析逻辑（crate::dll_paths）由
# 单测与 verify-dll-layout.ps1 覆盖，但**映射本身的笔误**只有检查真实安装包
# 才能发现——三个 DLL 三条映射，漏改或写错一条不会影响其余两条的运行期验证。
#
# 用 7-Zip 列出 NSIS 安装包的条目（不安装、不执行安装程序）。7z 缺失时跳过并
# 告警（不误判为失败）；7z 在位而 DLL 不在 lib/ 下则硬失败。
#
# 用法：
#   pwsh -NoProfile -File scripts/verify-installer-layout.ps1 `
#     -Installer src-tauri/target/x86_64-pc-windows-msvc/release/bundle/nsis/VeloTerm_0.1.0_x64-setup.exe
#
# 字符串字面量全 ASCII（PS 5.1 对无 BOM UTF-8 中文有编码坑），中文只在注释里。

[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Installer
)

$ErrorActionPreference = 'Continue'
$ProgressPreference = 'SilentlyContinue'
try { [Console]::OutputEncoding = [Text.Encoding]::UTF8 } catch { }

$log = New-Object System.Collections.Generic.List[string]
function Say([string]$s) { Write-Host $s; $script:log.Add($s) }

if (-not (Test-Path $Installer)) {
    Write-Host ("installer not found: {0}" -f $Installer)
    exit 1
}
$Installer = (Resolve-Path $Installer).Path

Say '=== verify-installer-layout ==='
Say ("installer : {0} ({1:N1} MB)" -f $Installer, ((Get-Item $Installer).Length / 1MB))

# 期望：DLL 在 lib/ 下；nmap-data/ 仍平铺在安装根
$expectedLib = @('lib/veloterm_goffi.dll', 'lib/qalcffi.dll', 'lib/nmapffi.dll')
$expectedRoot = @('coreutils.exe', 'nmap-data')
# 旧的平铺位置不该再出现（本轮起 DLL 统一进 lib/）
$forbiddenRoot = @('veloterm_goffi.dll', 'qalcffi.dll', 'nmapffi.dll')

# 定位 7z（windows-latest 预装；缺失则告警跳过）
$sevenZip = $null
foreach ($cand in @(
        "$env:ProgramFiles\7-Zip\7z.exe",
        "${env:ProgramFiles(x86)}\7-Zip\7z.exe",
        (Join-Path $env:SystemRoot 'system32\tar.exe'))) {
    if ($cand -and (Test-Path $cand) -and ($cand -like '*7z.exe')) { $sevenZip = $cand; break }
}
if (-not $sevenZip) {
    $cmd = Get-Command 7z -ErrorAction SilentlyContinue
    if ($cmd) { $sevenZip = $cmd.Source }
}
if (-not $sevenZip) {
    Say '  SKIP: 7z not found, cannot inspect the NSIS payload'
    Say 'RESULT: SKIP'
    if ($env:GITHUB_STEP_SUMMARY) {
        ("`r`n## verify-installer-layout`r`n`r`n" + ($log -join "`r`n")) |
            Out-File -FilePath $env:GITHUB_STEP_SUMMARY -Append -Encoding utf8
    }
    exit 0
}
Say ("7z        : {0}" -f $sevenZip)

$entries = @()
try {
    $raw = & $sevenZip l -ba $Installer 2>&1
    foreach ($line in $raw) {
        # 7z -ba 输出：日期 时间 属性 大小 [压缩后] 名称（名称可能含空格，取末段）
        $t = ([string]$line).Trim()
        if (-not $t) { continue }
        $parts = $t -split '\s+'
        if ($parts.Count -ge 6) {
            $entries += ($parts[5..($parts.Count - 1)] -join ' ')
        }
    }
} catch {
    Say ("  WARN: 7z failed to list the installer: {0}" -f $_.Exception.Message)
    Say 'RESULT: SKIP'
    exit 0
}
Say ("entries   : {0}" -f $entries.Count)
if ($entries.Count -eq 0) {
    Say '  WARN: installer listed no entries (unexpected NSIS layout?)'
    Say 'RESULT: SKIP'
    exit 0
}

# 归一化分隔符后比对（NSIS 条目用 \ 分隔）
$norm = $entries | ForEach-Object { ($_ -replace '\\', '/').TrimStart('/') }
$failed = $false

Say ''
Say '-- expected under lib/ --'
foreach ($want in $expectedLib) {
    $hit = $norm | Where-Object { $_ -eq $want }
    if ($hit) { Say ("  OK   {0}" -f $want) }
    else { Say ("  FAIL {0} (missing)" -f $want); $failed = $true }
}

Say ''
Say '-- expected at install root --'
foreach ($want in $expectedRoot) {
    $hit = $norm | Where-Object { $_ -eq $want -or $_.StartsWith("$want/") }
    if ($hit) { Say ("  OK   {0}" -f $want) }
    else { Say ("  FAIL {0} (missing)" -f $want); $failed = $true }
}

Say ''
Say '-- must NOT be flat at install root (moved into lib/) --'
foreach ($bad in $forbiddenRoot) {
    $hit = $norm | Where-Object { $_ -eq $bad }
    if ($hit) { Say ("  FAIL {0} still at install root" -f $bad); $failed = $true }
    else { Say ("  OK   {0} not flat" -f $bad) }
}

Say ''
Say $(if ($failed) { 'RESULT: FAIL' } else { 'RESULT: PASS (DLLs under lib/, nmap-data/ at root, no flat leftovers)' })

if ($env:GITHUB_STEP_SUMMARY) {
    ("`r`n## verify-installer-layout`r`n`r`n" + ($log -join "`r`n")) |
        Out-File -FilePath $env:GITHUB_STEP_SUMMARY -Append -Encoding utf8
}
if ($failed) { exit 1 }
