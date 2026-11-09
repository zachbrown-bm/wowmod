#Requires -Version 5.1
<#
.SYNOPSIS
    Installs the wowmod command-line tool so it can be run from any terminal.

.DESCRIPTION
    Copies wowmod.exe to %LOCALAPPDATA%\Programs\wowmod and adds that folder to
    your user PATH. No Rust toolchain is needed for the default mode.

    Default:  downloads the latest GitHub release (or -Version vX.Y.Z).
    -Local:   installs the exe you already built with `cargo build --release`
              (target\release\wowmod.exe next to this script), overwriting any
              release build that was installed earlier.

.EXAMPLE
    # Users: one line, no clone needed
    irm https://raw.githubusercontent.com/zachbrown-bm/wowmod/main/install.ps1 | iex

.EXAMPLE
    # Users: from a clone, or pin a version
    .\install.ps1
    .\install.ps1 -Version v0.1.0

.EXAMPLE
    # Developers: install what you just built
    cargo build --release
    .\install.ps1 -Local
#>
[CmdletBinding()]
param(
    [switch]$Local,
    [string]$Version = 'latest',
    [string]$Repo = 'zachbrown-bm/wowmod'
)

$ErrorActionPreference = 'Stop'

$installDir = Join-Path $env:LOCALAPPDATA 'Programs\wowmod'
$target = Join-Path $installDir 'wowmod.exe'
New-Item -ItemType Directory -Force -Path $installDir | Out-Null

if ($Local) {
    if (-not $PSScriptRoot) {
        Write-Error "-Local only works when the script is run from a clone of the repo (not piped to iex)."
    }
    $built = Join-Path $PSScriptRoot 'target\release\wowmod.exe'
    if (-not (Test-Path $built)) {
        Write-Error "No release build at $built. Run 'cargo build --release' first."
    }
    Write-Host "Installing local build $built"
    Copy-Item $built $target -Force
} else {
    [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
    $api = if ($Version -eq 'latest') { "https://api.github.com/repos/$Repo/releases/latest" }
           else { "https://api.github.com/repos/$Repo/releases/tags/$Version" }
    Write-Host "Looking up $Version release of $Repo ..."
    try {
        $release = Invoke-RestMethod -Uri $api -Headers @{ 'User-Agent' = 'wowmod-install' }
    } catch {
        if ($_.Exception.Response -and [int]$_.Exception.Response.StatusCode -eq 404) {
            Write-Error "No $Version release found at https://github.com/$Repo/releases. Developers: 'cargo build --release' then '.\install.ps1 -Local'."
        }
        throw
    }
    $asset = $release.assets | Where-Object { $_.name -eq 'wowmod-windows-x86_64.zip' } | Select-Object -First 1
    if (-not $asset) {
        Write-Error "Release $($release.tag_name) has no wowmod-windows-x86_64.zip asset."
    }

    $tmp = Join-Path ([IO.Path]::GetTempPath()) ("wowmod-install-" + [IO.Path]::GetRandomFileName())
    New-Item -ItemType Directory -Path $tmp | Out-Null
    try {
        $zip = Join-Path $tmp $asset.name
        Write-Host "Downloading $($asset.browser_download_url)"
        Invoke-WebRequest -Uri $asset.browser_download_url -OutFile $zip -UseBasicParsing
        Expand-Archive -Path $zip -DestinationPath $tmp -Force
        $exe = Get-ChildItem -Path $tmp -Recurse -Filter 'wowmod.exe' | Select-Object -First 1
        if (-not $exe) { Write-Error "wowmod.exe was not found inside $($asset.name)." }
        Write-Host "Installing $($release.tag_name)"
        Copy-Item $exe.FullName $target -Force
    } finally {
        Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
    }
}

# Make sure the install folder is on the user's PATH (persisted + this session).
function Test-OnPath([string]$PathList) {
    [bool](($PathList -split ';') | Where-Object { $_ -and ($_.TrimEnd('\') -ieq $installDir) })
}
$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
if (-not (Test-OnPath $userPath)) {
    $newPath = if ($userPath) { "$userPath;$installDir" } else { $installDir }
    [Environment]::SetEnvironmentVariable('Path', $newPath, 'User')
    Write-Host "Added $installDir to your user PATH. Open a new terminal to pick it up."
}
if (-not (Test-OnPath $env:Path)) {
    $env:Path = "$env:Path;$installDir"
}

Write-Host ""
Write-Host "Installed: $target"
& $target --version

# Warn if a different wowmod earlier on PATH would shadow this one.
$others = @(Get-Command wowmod -All -ErrorAction SilentlyContinue |
    Where-Object { $_.Source -and ($_.Source -ine $target) })
if ($others) {
    Write-Warning "Another wowmod is also on your PATH and may run instead of this one:"
    $others | ForEach-Object { Write-Host "    $($_.Source)" }
    Write-Host "Remove it (for a cargo install: 'cargo uninstall wowmod') or move $installDir earlier in PATH."
}
