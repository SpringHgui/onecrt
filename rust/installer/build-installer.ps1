# 构建 OneCRT Windows 安装包（Inno Setup）
#
#   powershell -ExecutionPolicy Bypass -File rust/installer/build-installer.ps1
#   ... -SkipBuild      # 复用已构建的 target\release\onecrt.exe
#   ... -Iscc <path>    # 指定 ISCC.exe
#
# 产物：rust\target\installer\onecrt-<版本>-windows-x64-setup.exe

param(
    [switch]$SkipBuild,
    [string]$Iscc = ""
)

$ErrorActionPreference = 'Stop'

$root = Split-Path -Parent $PSScriptRoot
$cargo = Join-Path $root 'Cargo.toml'

if (-not $SkipBuild) {
    Push-Location $root
    try {
        cargo build --release -p onecrt
        if ($LASTEXITCODE -ne 0) { throw "cargo build 失败，退出码 $LASTEXITCODE" }
    }
    finally {
        Pop-Location
    }
}

$match = (Select-String -Path $cargo -Pattern '^version = "([^"]+)"' | Select-Object -First 1)
if (-not $match) { throw '读不到 rust/Cargo.toml 的 workspace 版本号' }
$version = $match.Matches[0].Groups[1].Value

$stage = Join-Path $root 'target\installer\stage'
if (Test-Path $stage) { Remove-Item $stage -Recurse -Force }
New-Item -ItemType Directory -Force $stage | Out-Null
Copy-Item (Join-Path $root 'target\release\onecrt.exe') $stage

# Rust 的 MSVC 目标默认动态链接 VC 运行库，带上它的 DLL 免得目标机没装 VC++ 运行库
$crt = Join-Path $env:SystemRoot 'System32\vcruntime140.dll'
if (Test-Path $crt) {
    Copy-Item $crt $stage
}
else {
    Write-Warning '没找到 vcruntime140.dll，安装包将依赖目标机自带的 VC++ 运行库'
}

if (-not $Iscc) {
    $candidates = @(
        "$env:ProgramFiles\Inno Setup 7\ISCC.exe",
        "${env:ProgramFiles(x86)}\Inno Setup 7\ISCC.exe",
        "$env:ProgramFiles\Inno Setup 6\ISCC.exe",
        "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe"
    )
    $Iscc = $candidates | Where-Object { Test-Path $_ } | Select-Object -First 1
}
if (-not $Iscc) { throw '找不到 ISCC.exe，请先安装 Inno Setup 6 或 7' }

$out = Join-Path $root 'target\installer'
& $Iscc "/DMyAppVersion=$version" "/DStageDir=$stage" "/O$out" (Join-Path $PSScriptRoot 'onecrt.iss')
if ($LASTEXITCODE -ne 0) { throw "ISCC 编译失败，退出码 $LASTEXITCODE" }

Get-ChildItem $out -Filter '*.exe' |
    Sort-Object LastWriteTime |
    Select-Object Name, @{ n = 'SizeMB'; e = { [math]::Round($_.Length / 1MB, 1) } }, FullName |
    Format-List
