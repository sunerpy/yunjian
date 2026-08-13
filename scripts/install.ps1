# 云笺 yunjian 安装脚本（Windows / PowerShell）。
#
# 一条命令装好 CLI，然后把用户交给 `yunjian corpus fetch`。
#
#   irm https://raw.githubusercontent.com/sunerpy/yunjian/main/scripts/install.ps1 | iex
#
# 可用环境变量（与 scripts/install.sh 逐字同名，两边不许分叉）：
#
#   YUNJIAN_VERSION       要装的版本，形如 `v0.1.0` 或 `0.1.0`。缺省取最新正式发布。
#   YUNJIAN_INSTALL_DIR   安装目录。缺省 `$HOME\.local\bin`。
#   YUNJIAN_BASE_URL      发布资产的下载前缀。缺省 GitHub Releases。
#   YUNJIAN_API_URL       解析最新版本用的 API 前缀。缺省 GitHub API。
#
# 退出码与 `yunjian` 自身的约定一致（见 docs/CLI.zh.md）：
#
#   0  装好了
#   2  用法错误：架构不受支持、版本号写错
#   3  取不到东西：下载失败、资产不存在、**校验和不匹配**
#
# 校验和不匹配走 3 而不是 2 是刻意的：调用方改命令没用，要改的是「拿到的那份文件」。
# 且任何一次校验失败都**不落盘**——先在临时目录里校验，通过了才装进目标目录。

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$Repo = 'sunerpy/yunjian'
$BinaryName = 'yunjian'

function Get-EnvOrDefault {
    param(
        [Parameter(Mandatory = $true)][string] $Name,
        [Parameter(Mandatory = $true)][AllowEmptyString()][string] $Default
    )
    $value = [Environment]::GetEnvironmentVariable($Name)
    if ([string]::IsNullOrWhiteSpace($value)) { return $Default }
    return $value
}

# 日志一律走 stderr，stdout 留给可能的管道消费方。与 CLI 的两条流约定同源。
function Write-Info {
    param([Parameter(Mandatory = $true)][AllowEmptyString()][string] $Message)
    [Console]::Error.WriteLine($Message)
}

function Exit-WithUsageError {
    param([Parameter(Mandatory = $true)][string] $Message)
    [Console]::Error.WriteLine("error: $Message")
    exit 2
}

function Exit-WithUnavailableError {
    param([Parameter(Mandatory = $true)][string] $Message)
    [Console]::Error.WriteLine("error: $Message")
    exit 3
}

function Get-TargetTriple {
    $arch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture
    switch ($arch) {
        'X64' { return 'x86_64-pc-windows-msvc' }
        'Arm64' { return 'aarch64-pc-windows-msvc' }
        default {
            Exit-WithUsageError "不支持的 CPU 架构 $arch；发布产物只覆盖 x86_64 与 aarch64"
        }
    }
}

function Resolve-ReleaseTag {
    param([Parameter(Mandatory = $true)][string] $ApiUrl)

    $requested = [Environment]::GetEnvironmentVariable('YUNJIAN_VERSION')
    if (-not [string]::IsNullOrWhiteSpace($requested)) {
        if ($requested.StartsWith('v')) { return $requested }
        return "v$requested"
    }

    try {
        $latest = Invoke-RestMethod -Uri "$ApiUrl/releases/latest" -UseBasicParsing
    } catch {
        Exit-WithUnavailableError '取不到最新版本；用 YUNJIAN_VERSION 显式指定，例如 $env:YUNJIAN_VERSION = "v0.1.0"'
    }
    if ([string]::IsNullOrWhiteSpace($latest.tag_name)) {
        Exit-WithUnavailableError '最新发布里读不到 tag_name；用 YUNJIAN_VERSION 显式指定版本'
    }
    return $latest.tag_name
}

function Save-RemoteFile {
    param(
        [Parameter(Mandatory = $true)][string] $Uri,
        [Parameter(Mandatory = $true)][string] $Path
    )
    try {
        Invoke-WebRequest -Uri $Uri -OutFile $Path -UseBasicParsing | Out-Null
        return $true
    } catch {
        return $false
    }
}

# ---------------------------------------------------------------- 主流程

$baseUrl = Get-EnvOrDefault -Name 'YUNJIAN_BASE_URL' -Default "https://github.com/$Repo/releases/download"
$apiUrl = Get-EnvOrDefault -Name 'YUNJIAN_API_URL' -Default "https://api.github.com/repos/$Repo"
$installDir = Get-EnvOrDefault -Name 'YUNJIAN_INSTALL_DIR' -Default (Join-Path $HOME '.local\bin')

$tag = Resolve-ReleaseTag -ApiUrl $apiUrl
$version = $tag.TrimStart('v')
$target = Get-TargetTriple
$archive = "$BinaryName-$version-$target.zip"

$workDir = Join-Path ([System.IO.Path]::GetTempPath()) ("yunjian-install-" + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $workDir -Force | Out-Null

try {
    Write-Info "云笺 yunjian $tag"
    Write-Info "尝试 $archive"

    $archivePath = Join-Path $workDir $archive
    if (-not (Save-RemoteFile -Uri "$baseUrl/$tag/$archive" -Path $archivePath)) {
        Exit-WithUnavailableError "在 $tag 下找不到适配本机的产物（$archive）"
    }

    # 摘要文件是**必需**的，取不到就中止。没有摘要的安装等于没有校验，
    # 而「悄悄跳过校验」比「装不上」危险得多。
    $sumPath = "$archivePath.sha256"
    if (-not (Save-RemoteFile -Uri "$baseUrl/$tag/$archive.sha256" -Path $sumPath)) {
        Exit-WithUnavailableError "取不到 $archive.sha256；缺少摘要时拒绝安装"
    }

    # 摘要文件是 `sha256sum` 格式：`<hex>  <filename>`，只取第一段。
    $expected = ((Get-Content -Path $sumPath -Raw) -split '\s+')[0]
    if ([string]::IsNullOrWhiteSpace($expected)) {
        Exit-WithUnavailableError "$archive.sha256 里读不出摘要"
    }
    $actual = (Get-FileHash -Path $archivePath -Algorithm SHA256).Hash

    if ($expected -ne $actual) {
        [Console]::Error.WriteLine("error: $archive 校验和不匹配，未安装任何文件")
        [Console]::Error.WriteLine("  期望 $expected")
        [Console]::Error.WriteLine("  实际 $actual")
        exit 3
    }
    Write-Info "校验和通过（sha256 $actual）"

    Expand-Archive -Path $archivePath -DestinationPath $workDir -Force
    $extracted = Join-Path $workDir "$BinaryName.exe"
    if (-not (Test-Path -Path $extracted)) {
        Exit-WithUnavailableError "$archive 里没有 $BinaryName.exe 可执行文件"
    }

    New-Item -ItemType Directory -Path $installDir -Force | Out-Null
    Copy-Item -Path $extracted -Destination (Join-Path $installDir "$BinaryName.exe") -Force

    Write-Info "已安装 $(Join-Path $installDir "$BinaryName.exe")（$target）"

    # ------------------------------------------------------------ 下一步

    $pathEntries = ($env:PATH -split ';')
    if ($pathEntries -notcontains $installDir) {
        Write-Info ''
        Write-Info "注意：$installDir 不在 PATH 上。当前会话可以这样加上："
        Write-Info "  `$env:PATH = `"$installDir;`$env:PATH`""
    }

    Write-Info ''
    Write-Info '下一步：'
    Write-Info '  yunjian corpus fetch      # 下载并校验语料库（约 211 MiB）'
    Write-Info '  yunjian search 明月       # 查一句试试'
} finally {
    if (Test-Path -Path $workDir) {
        Remove-Item -Path $workDir -Recurse -Force -ErrorAction SilentlyContinue
    }
}
