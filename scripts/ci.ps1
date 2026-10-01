<#
.SYNOPSIS
Checks formatting, lints and runs every test, failing on the first step that fails.

.DESCRIPTION
The integration tests open real picker windows, so this needs an interactive desktop: a GitHub Windows runner has
one. Run it locally with the desktop left alone.
#>
$ErrorActionPreference = 'Stop'
$manifest = Join-Path $PSScriptRoot '..\Cargo.toml'

function Invoke-Cargo([string[]]$arguments) {
    & cargo @arguments
    if ($LASTEXITCODE -ne 0) {
        throw "cargo $($arguments -join ' ') failed with exit code $LASTEXITCODE"
    }
}

Invoke-Cargo @('fmt', '--manifest-path', $manifest, '--check')
Invoke-Cargo @('clippy', '--manifest-path', $manifest, '--all-targets', '--', '-D', 'warnings')
Invoke-Cargo @('test', '--manifest-path', $manifest)
