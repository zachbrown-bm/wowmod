#Requires -Version 5.1
<#
.SYNOPSIS
    Builds wowmod in release mode and installs it onto your PATH.

.DESCRIPTION
    Runs `cargo install --path <repo> --locked`, which puts wowmod.exe in
    Cargo's bin directory (%USERPROFILE%\.cargo\bin by default, or $CARGO_HOME\bin).
    The Rust installer adds that directory to PATH, so after this finishes you
    can run `wowmod` from any terminal.

    Re-run the script after pulling changes to upgrade. Remove with:
        cargo uninstall wowmod

.EXAMPLE
    .\install.ps1
#>
[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'

if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    Write-Error "cargo was not found. Install Rust from https://rustup.rs and open a new terminal, then re-run this script."
}

$repo = $PSScriptRoot
Write-Host "Installing wowmod from $repo ..."
cargo install --path $repo --locked
if ($LASTEXITCODE -ne 0) {
    Write-Error "cargo install failed with exit code $LASTEXITCODE."
}

if ($env:CARGO_HOME) { $binDir = Join-Path $env:CARGO_HOME 'bin' }
else                 { $binDir = Join-Path $HOME '.cargo\bin' }

$onPath = ($env:PATH -split ';') | Where-Object { $_ -and ($_.TrimEnd('\') -ieq $binDir.TrimEnd('\')) }
if ($onPath) {
    Write-Host ""
    Write-Host "Installed: $(Join-Path $binDir 'wowmod.exe')"
    Write-Host "Run 'wowmod' from any terminal. Re-run this script to upgrade, 'cargo uninstall wowmod' to remove."
} else {
    Write-Warning "$binDir is not on your PATH. Add it (System Properties > Environment Variables, or:)"
    Write-Host "    [Environment]::SetEnvironmentVariable('Path', `$env:Path + ';$binDir', 'User')"
    Write-Host "then open a new terminal."
}
