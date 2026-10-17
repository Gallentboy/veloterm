# verify-dll-layout.ps1 -- 验证 FFI DLL 的 `lib/` 布局在运行期真的能被找到。
#
# 背景：三个 DLL 例外形态（veloterm_goffi.dll / qalcffi.dll / nmapffi.dll）随安装包
# 分发在安装根 `lib/` 子目录（映射见 tauri.windows.conf.json 的 bundle.resources），
# 运行期由 coreutils 的 *_impl.rs 经共享模块 crate::dll_paths 解析候选路径后
# LoadLibrary。候选顺序：本级 lib/ → 上级 lib/ → 同目录平铺兜底（前者服务
# coreutils.exe 直跑，中者服务 tools\ 短名硬链接启动，末者是 dev/手工布局兜底；
# lib/ 一律排在平铺之前，防止旧版残留的平铺 DLL 遮蔽新装文件）。
#
# 本脚本在 CI 上把「安装后的目录形状」真实搭出来并执行命令——这是唯一能覆盖
# DLL 加载路径的验证方式（cargo test 只覆盖候选路径的纯函数逻辑，Windows 车道
# 也不跑 cargo test；打包映射本身只有 publish 能验）。
#
# 用法：
#   pwsh -NoProfile -File scripts/verify-dll-layout.ps1 `
#     -Coreutils src-tauri/binaries/coreutils-x86_64-pc-windows-msvc.exe `
#     -Dll src-tauri/binaries/veloterm_goffi.dll -Cmd yq -VersionArg --version
#
# 断言（任一失败即以非零码退出，供 CI 车道硬门禁）：
#   1. lib/ 布局（tools\ 短名启动）→ 命令可执行、退出码 0
#   2. lib/ 布局（coreutils.exe 直跑）→ 同上
#   3. 平铺兜底布局（DLL 与 exe 同目录）→ 同上
#   4. DLL 完全缺失 → 退出码 2 且 stderr 给出「DLL 不可用」诊断
#
# 字符串字面量全 ASCII（PS 5.1 对无 BOM UTF-8 中文有编码坑），中文只在注释里。

[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Coreutils,
    [Parameter(Mandatory = $true)][string]$Dll,
    [string]$Cmd = 'yq',
    [string]$VersionArg = '--version'
)

$ErrorActionPreference = 'Continue'
$ProgressPreference = 'SilentlyContinue'
try { [Console]::OutputEncoding = [Text.Encoding]::UTF8 } catch { }

$log = New-Object System.Collections.Generic.List[string]
function Say([string]$s) { Write-Host $s; $script:log.Add([string]$s) }
function Fail([string]$s) { Say ("  FAIL: " + $s); $script:failed = $true }

if (-not (Test-Path $Coreutils)) { Write-Host "coreutils not found: $Coreutils"; exit 1 }
if (-not (Test-Path $Dll)) { Write-Host "DLL not found: $Dll"; exit 1 }
$Coreutils = (Resolve-Path $Coreutils).Path
$Dll = (Resolve-Path $Dll).Path
$dllName = Split-Path $Dll -Leaf

Say '=== verify-dll-layout ==='
Say ("coreutils : {0} ({1:N1} MB)" -f $Coreutils, ((Get-Item $Coreutils).Length / 1MB))
Say ("dll       : {0} ({1:N1} MB)" -f $dllName, ((Get-Item $Dll).Length / 1MB))
Say ("probe cmd : {0} {1}" -f $Cmd, $VersionArg)

$script:failed = $false
$root = Join-Path $env:TEMP 'veloterm-dll-layout'
if (Test-Path $root) { Remove-Item $root -Recurse -Force -ErrorAction SilentlyContinue }

# 执行一次并返回 @{ Code; Out; Err }（用文件重定向抓流，避开 PS 5.1 的
# NativeCommandError 包装与内嵌引号转义问题）
function Invoke-Probe([string]$exe, [string[]]$cmdArgs) {
    $outFile = Join-Path $root 'out.txt'
    $errFile = Join-Path $root 'err.txt'
    Remove-Item $outFile, $errFile -ErrorAction SilentlyContinue
    $p = Start-Process -FilePath $exe -ArgumentList $cmdArgs -NoNewWindow -PassThru -Wait `
        -RedirectStandardOutput $outFile -RedirectStandardError $errFile
    $out = if (Test-Path $outFile) { ([string](Get-Content $outFile -Raw -Encoding UTF8)).Trim() } else { '' }
    $err = if (Test-Path $errFile) { ([string](Get-Content $errFile -Raw -Encoding UTF8)).Trim() } else { '' }
    return @{ Code = [int]$p.ExitCode; Out = $out; Err = $err }
}

# 断言：命令可执行且退出码 0、stdout 非空
function Assert-Runs([string]$label, [string]$exe, [string[]]$cmdArgs) {
    $r = Invoke-Probe $exe $cmdArgs
    $ok = ($r.Code -eq 0) -and ($r.Out.Length -gt 0)
    Say ("  [{0}] exit={1} stdout={2}" -f $label, $r.Code, $(if ($r.Out) { $r.Out -replace "`r?`n", ' | ' } else { '(empty)' }))
    if ($r.Err) { Say ("      stderr: " + ($r.Err -replace "`r?`n", ' | ')) }
    if (-not $ok) { Fail "$label: expected exit 0 with output" }
}

# 断言：DLL 缺失时按「命令不可用」惯例退出 2 并给出诊断
# （诊断串含 DLL 名，故按名匹配——避免在 PS 5.1 脚本里写中文正则）
function Assert-Unavailable([string]$label, [string]$exe, [string[]]$cmdArgs) {
    $r = Invoke-Probe $exe $cmdArgs
    $ok = ($r.Code -eq 2) -and ($r.Err -match [regex]::Escape($dllName))
    Say ("  [{0}] exit={1} stderr={2}" -f $label, $r.Code, $(if ($r.Err) { $r.Err -replace "`r?`n", ' | ' } else { '(empty)' }))
    if (-not $ok) { Fail "$label: expected exit 2 with '$dllName' named on stderr" }
}

# ── 布局 1：lib/ 子目录 + tools\ 硬链接短名（安装后的标准形状）──────────
Say ''
Say '-- layout 1: lib/ + tools\ short-name hard link (installed shape) --'
$app = Join-Path $root 'app1'
New-Item -ItemType Directory -Path (Join-Path $app 'lib') -Force | Out-Null
New-Item -ItemType Directory -Path (Join-Path $app 'tools') -Force | Out-Null
Copy-Item $Coreutils (Join-Path $app 'coreutils.exe') -Force
Copy-Item $Dll (Join-Path $app ('lib\' + $dllName)) -Force
$shortName = Join-Path $app ('tools\' + $Cmd + '.exe')
try {
    New-Item -ItemType HardLink -Path $shortName -Target (Join-Path $app 'coreutils.exe') -Force | Out-Null
} catch {
    Copy-Item (Join-Path $app 'coreutils.exe') $shortName -Force   # 极端环境不支持硬链接时退化
}
Assert-Runs 'tools\<cmd>.exe (lib/ layout)' $shortName @($VersionArg)

# ── 布局 2：lib/ 子目录 + coreutils.exe 直跑（子命令模式）─────────────
Say ''
Say '-- layout 2: lib/ + coreutils.exe subcommand mode --'
Assert-Runs 'coreutils.exe <cmd> (lib/ layout)' (Join-Path $app 'coreutils.exe') @($Cmd, $VersionArg)

# ── 布局 3：平铺兜底（DLL 与 exe 同目录，dev/手工布局）────────────────
Say ''
Say '-- layout 3: flat fallback (DLL next to exe) --'
$app3 = Join-Path $root 'app3'
New-Item -ItemType Directory -Path $app3 -Force | Out-Null
Copy-Item $Coreutils (Join-Path $app3 'coreutils.exe') -Force
Copy-Item $Dll (Join-Path $app3 $dllName) -Force
Assert-Runs 'coreutils.exe <cmd> (flat layout)' (Join-Path $app3 'coreutils.exe') @($Cmd, $VersionArg)

# ── 布局 4：DLL 完全缺失（诊断路径）─────────────────────────────────
Say ''
Say '-- layout 4: DLL missing (diagnostic path) --'
$app4 = Join-Path $root 'app4'
New-Item -ItemType Directory -Path $app4 -Force | Out-Null
Copy-Item $Coreutils (Join-Path $app4 'coreutils.exe') -Force
Assert-Unavailable 'coreutils.exe <cmd> (no DLL)' (Join-Path $app4 'coreutils.exe') @($Cmd, $VersionArg)

# ── 布局 5：升级残留防护（平铺的陈旧 DLL 不得遮蔽 lib/ 里的新 DLL）─────
# 从旧版本升级的机器上，安装根可能残留上一版平铺的 DLL（NSIS 卸载清单已不含
# 它，不会自动清理）。这里在平铺位置放一个**能加载但没有目标导出符号**的真实
# DLL（系统 version.dll），在 lib/ 放真 DLL：
#   - lib/ 优先 → 加载真 DLL，yq 正常输出（PASS）
#   - 平铺优先 → 加载 version.dll 成功但取不到导出符号，命令以 2 失败（FAIL）
# 用「能加载的错 DLL」而非垃圾文件是关键：垃圾文件会让 LoadLibrary 失败并
# 落到下一个候选，即使顺序写反也照样通过，测不出遮蔽问题。
Say ''
Say '-- layout 5: stale flat DLL must not shadow lib/ (upgrade residue) --'
$decoy = Join-Path $env:SystemRoot 'System32\version.dll'
if (-not (Test-Path $decoy)) {
    Say ("  SKIP: decoy DLL not available at {0}" -f $decoy)
} else {
    $app5 = Join-Path $root 'app5'
    New-Item -ItemType Directory -Path (Join-Path $app5 'lib') -Force | Out-Null
    Copy-Item $Coreutils (Join-Path $app5 'coreutils.exe') -Force
    Copy-Item $Dll (Join-Path $app5 ('lib\' + $dllName)) -Force
    Copy-Item $decoy (Join-Path $app5 $dllName) -Force   # 平铺位置的「陈旧 DLL」
    Assert-Runs 'coreutils.exe <cmd> (lib/ beats stale flat)' (Join-Path $app5 'coreutils.exe') @($Cmd, $VersionArg)
}

# ── 汇总 ────────────────────────────────────────────────────────────
Say ''
if ($script:failed) {
    Say 'RESULT: FAIL'
} else {
    Say 'RESULT: PASS (lib/ layout resolves, flat fallback works, missing-DLL path is diagnostic)'
}
$outFile = Join-Path $root 'verify-dll-layout.result.txt'
($log -join "`r`n") | Out-File -FilePath $outFile -Encoding utf8
Say ("saved: {0}" -f $outFile)
if ($env:GITHUB_STEP_SUMMARY) {
    ("`r`n## verify-dll-layout`r`n`r`n" + ($log -join "`r`n")) |
        Out-File -FilePath $env:GITHUB_STEP_SUMMARY -Append -Encoding utf8
}
if ($script:failed) { exit 1 }
