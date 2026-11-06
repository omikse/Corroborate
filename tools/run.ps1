# Builds the current code and starts it from app\, the tool's own folder (its settings, results
# and recordings live there). Run through run.cmd at the repository root.
#
#   run.cmd                 build, update app\, start the workbench
#   run.cmd -NoStart        build and update app\ only
#   run.cmd <path or URL>   ... and open that source (arguments go to the workbench)
#
# cargo only ever writes target\, so building never waits for a running app. A program in app\
# that is still running cannot be overwritten, but it can be renamed: it is moved aside as
# <name>.exe.old-<time> and the new one put in its place. The open window keeps running the old
# build; the next start is the new one. Old copies are deleted once nothing runs them.

param(
    [switch]$NoStart,
    [Parameter(ValueFromRemainingArguments = $true)] [string[]]$AppArgs
)
# Errors from cargo and git are judged by their exit codes; stderr is only progress.
$ErrorActionPreference = 'Continue'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
$app = Join-Path $root 'app'

# The build shown in the workbench's top bar: the commit, + when there are uncommitted changes.
$hash = (git rev-parse --short HEAD)
$dirty = (git status --porcelain --untracked-files=no)
$env:CORROBORATE_BUILD = if ($dirty) { "$hash+" } else { "$hash" }

Write-Host "Building $env:CORROBORATE_BUILD (release)..."
cargo build --release
if ($LASTEXITCODE -ne 0) {
    Write-Host 'Build failed: app\ is unchanged.'
    exit 1
}

New-Item -ItemType Directory -Force $app | Out-Null
Get-ChildItem $app -Filter '*.old-*' | ForEach-Object {
    try { Remove-Item $_.FullName -Force -ErrorAction Stop } catch { }
}

$programs = 'corroborate', 'corroborate-cli', 'planes-view', 'enf-plugin', 'meta-plugin', 'adsb-check'
foreach ($p in $programs) {
    $src = Join-Path $root "target\release\$p.exe"
    $dst = Join-Path $app "$p.exe"
    if (-not (Test-Path $src)) { continue }
    if (Test-Path $dst) {
        if ((Get-FileHash $src).Hash -eq (Get-FileHash $dst).Hash) { continue }
        try {
            Remove-Item $dst -Force -ErrorAction Stop
        } catch {
            $aside = "$p.exe.old-" + (Get-Date -Format 'yyyyMMdd-HHmmss')
            Rename-Item $dst $aside
            Write-Host "$p is running: the old one is kept as $aside until it closes."
        }
    }
    Copy-Item $src $dst
    Write-Host "updated $p"
}

if (-not $NoStart) {
    $exe = Join-Path $app 'corroborate.exe'
    if ($AppArgs) {
        Start-Process $exe -WorkingDirectory $app -ArgumentList $AppArgs
    } else {
        Start-Process $exe -WorkingDirectory $app
    }
}
