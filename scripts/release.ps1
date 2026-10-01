<#
.SYNOPSIS
Builds the release exe and publishes it as the GitHub release for an existing tag.

.PARAMETER Tag
The pushed tag, `v` then the version in Cargo.toml, as v0.1.0.

.PARAMETER Token
A GitHub token that can write releases: the workflow's GITHUB_TOKEN, or `gh auth token` locally.

.EXAMPLE
.\scripts\release.ps1 -Tag v0.1.0 -Token (gh auth token)
#>
param(
    [Parameter(Mandatory)][string]$Tag,
    [Parameter(Mandatory)][string]$Token
)
$ErrorActionPreference = 'Stop'
$root = Resolve-Path (Join-Path $PSScriptRoot '..')
$manifest = Join-Path $root 'Cargo.toml'

$version = (cargo metadata --manifest-path $manifest --no-deps --format-version 1 | ConvertFrom-Json).packages |
    Where-Object name -EQ 'carronade' |
    Select-Object -ExpandProperty version
if ($LASTEXITCODE -ne 0) {
    throw "cargo metadata failed with exit code $LASTEXITCODE"
}
if ($Tag -ne "v$version") {
    throw "tag $Tag does not match version $version in $manifest"
}

& cargo build --manifest-path $manifest --release
if ($LASTEXITCODE -ne 0) {
    throw "cargo build --release failed with exit code $LASTEXITCODE"
}

$staging = Join-Path $root "target\release-package\carronade-$Tag"
$zip = Join-Path $root "target\carronade-$Tag-x86_64-pc-windows-msvc.zip"
if (Test-Path $staging) {
    Remove-Item -Recurse $staging
}
New-Item -ItemType Directory $staging | Out-Null
$files = 'target\release\carronade.exe', 'config.toml', 'LICENSE', 'README.md'
Copy-Item -Path ($files | ForEach-Object { Join-Path $root $_ }) -Destination $staging
Compress-Archive -Path (Join-Path $staging '*') -DestinationPath $zip -Force

$env:GH_TOKEN = $Token
& gh release create $Tag $zip --repo CLBRITTON2/carronade --title $Tag --generate-notes --verify-tag
if ($LASTEXITCODE -ne 0) {
    throw "gh release create $Tag failed with exit code $LASTEXITCODE"
}
