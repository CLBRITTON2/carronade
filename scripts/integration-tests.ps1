<#
.SYNOPSIS
Runs the tests in tests, which open real picker windows and read the real Start menu.

.DESCRIPTION
They need an interactive desktop: a GitHub Windows runner has one. Run them locally with the desktop left alone,
since anything that takes focus cancels the open picker.
#>
. (Join-Path $PSScriptRoot 'cargo.ps1')

Invoke-Cargo @('test', '--manifest-path', $manifest, '--test', '*')
