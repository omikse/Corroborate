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
$modules = Join-Path $app 'modules'
New-Item -ItemType Directory -Force $modules | Out-Null
Get-ChildItem $app -Recurse -Filter '*.old-*' | ForEach-Object {
    try { Remove-Item $_.FullName -Force -ErrorAction Stop } catch { }
}

# Put a built program at $dst; one that is running is moved aside, not overwritten.
function Deploy-Program([string]$exe, [string]$dst) {
    $src = Join-Path $root "target\release\$exe"
    if (-not (Test-Path $src)) { return }
    if (Test-Path $dst) {
        if ((Get-FileHash $src).Hash -eq (Get-FileHash $dst).Hash) { return }
        try {
            Remove-Item $dst -Force -ErrorAction Stop
        } catch {
            $aside = "$exe.old-" + (Get-Date -Format 'yyyyMMdd-HHmmss')
            Rename-Item $dst $aside
            Write-Host "$exe is running: the old one is kept as $aside until it closes."
        }
    }
    Copy-Item $src $dst
    Write-Host "updated $dst"
}

# The tool's own programs sit in app\.
foreach ($p in 'corroborate.exe', 'corroborate-cli.exe', 'planes-view.exe') {
    Deploy-Program $p (Join-Path $app $p)
}

# Each module folder (src\modules\<label>\, with a module.toml) goes to app\modules\<label>\:
# its module.toml, look scripts, READMEs and data, and the programs its module.toml names. A file
# changed in app\ after the last copy is kept (and listed), so tweaks made there are not lost.
$moved = @()
foreach ($dir in Get-ChildItem (Join-Path $root 'src\modules') -Directory) {
    $manifest = Join-Path $dir.FullName 'module.toml'
    if (-not (Test-Path $manifest)) { continue }
    $dst = Join-Path $modules $dir.Name
    New-Item -ItemType Directory -Force $dst | Out-Null
    $own = @(Get-ChildItem $dir.FullName -File | Where-Object { $_.Extension -in '.toml', '.rhai', '.md', '.json' })
    # The types the module brings: types\<id>.toml.
    $types = Join-Path $dir.FullName 'types'
    if (Test-Path $types) {
        New-Item -ItemType Directory -Force (Join-Path $dst 'types') | Out-Null
        $own += @(Get-ChildItem $types -File -Filter '*.toml')
    }
    foreach ($f in $own) {
        $to = if ($f.DirectoryName -eq $types) { Join-Path (Join-Path $dst 'types') $f.Name } else { Join-Path $dst $f.Name }
        if (-not (Test-Path $to) -or (Get-Item $to).LastWriteTime -lt $f.LastWriteTime) {
            Copy-Item $f.FullName $to
        } elseif ((Get-FileHash $to).Hash -ne (Get-FileHash $f.FullName).Hash) {
            Write-Host "kept your change: $to (differs from $($f.FullName))"
        }
    }
    $text = Get-Content -Raw $manifest
    $names = [regex]::Matches($text, '(?m)^\s*program\s*=\s*"([^"]+)"') | ForEach-Object { $_.Groups[1].Value }
    $list = [regex]::Match($text, '(?m)^\s*programs\s*=\s*\[([^\]]*)\]')
    if ($list.Success) { $names += [regex]::Matches($list.Groups[1].Value, '"([^"]+)"') | ForEach-Object { $_.Groups[1].Value } }
    foreach ($exe in $names) {
        Deploy-Program $exe (Join-Path $dst $exe)
        $moved += $exe
    }
}

# Programs that now live in their module folder (and meta-plugin, renamed metadata) leave app\.
foreach ($exe in $moved + 'meta-plugin.exe') {
    $old = Join-Path $app $exe
    if (Test-Path $old) {
        try { Remove-Item $old -Force -ErrorAction Stop; Write-Host "removed $old (moved to its module folder)" }
        catch { Write-Host "$old is running: remove it after it closes." }
    }
}

# Each type folder (src\types\<id>\, with a type.toml) goes to app\types\<id>\, kept like module
# files: a type.toml changed in app\ after the last copy is kept (and listed).
foreach ($dir in Get-ChildItem (Join-Path $root 'src\types') -Directory) {
    $from = Join-Path $dir.FullName 'type.toml'
    if (-not (Test-Path $from)) { continue }
    $dst = Join-Path (Join-Path $app 'types') $dir.Name
    New-Item -ItemType Directory -Force $dst | Out-Null
    $to = Join-Path $dst 'type.toml'
    if (-not (Test-Path $to) -or (Get-Item $to).LastWriteTime -lt (Get-Item $from).LastWriteTime) {
        Copy-Item $from $to
    } elseif ((Get-FileHash $to).Hash -ne (Get-FileHash $from).Hash) {
        Write-Host "kept your change: $to (differs from $from)"
    }
}

if (-not $NoStart) {
    $exe = Join-Path $app 'corroborate.exe'
    if ($AppArgs) {
        Start-Process $exe -WorkingDirectory $app -ArgumentList $AppArgs
    } else {
        Start-Process $exe -WorkingDirectory $app
    }
}
