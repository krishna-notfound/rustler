# Rustler installer for Windows x64 (no admin needed). Works in Windows PowerShell 5.1 and PowerShell 7.
#
#   install.ps1 [--version vX.Y.Z] [--no-setup] [--dir <path>] [--repo owner/name]
#   (-Version / -NoSetup / -Dir / -Repo spellings are accepted too)
#
# Downloads the latest GitHub release asset rustler-<tag>-windows-x64.zip, verifies it against SHA256SUMS.txt,
# installs it into ~\.rustler\app (override: $env:RUSTLER_INSTALL_DIR or --dir), adds that folder to the USER
# PATH, copies the Claude Code skill to ~\.claude\skills\rustler when ~\.claude exists, then runs
# `rustler setup run` (skip with --no-setup). Idempotent: running it again upgrades in place.
#
# The repository is private. Authentication, first match wins:
#   1. $env:RUSTLER_TOKEN or $env:GITHUB_TOKEN (token with read access to the repo's releases) -> GitHub API
#   2. an authenticated `gh` CLI (gh auth status)
#
# Environment: RUSTLER_INSTALL_DIR, RUSTLER_CLAUDE_DIR (default ~\.claude), RUSTLER_REPO
#   (default krishna-notfound/rustler), RUSTLER_PATH_SCOPE (User | Process | None, default User),
#   RUSTLER_HOME (data dir used by `rustler setup`).

$ErrorActionPreference = 'Stop'
[Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
$ProgressPreference = 'SilentlyContinue'

$Repo = if ($env:RUSTLER_REPO) { $env:RUSTLER_REPO } else { 'krishna-notfound/rustler' }
$InstallDir = if ($env:RUSTLER_INSTALL_DIR) { $env:RUSTLER_INSTALL_DIR } else { Join-Path $HOME '.rustler\app' }
$ClaudeDir = if ($env:RUSTLER_CLAUDE_DIR) { $env:RUSTLER_CLAUDE_DIR } else { Join-Path $HOME '.claude' }
$PathScope = if ($env:RUSTLER_PATH_SCOPE) { $env:RUSTLER_PATH_SCOPE } else { 'User' }
$Token = if ($env:RUSTLER_TOKEN) { $env:RUSTLER_TOKEN } else { $env:GITHUB_TOKEN }
$Version = ''
$RunSetup = $true
$Api = 'https://api.github.com'

function Say([string]$step, [string]$status, [string]$detail = '') {
    $d = ($detail -replace '\\', '\\' -replace '"', '\"')
    Write-Host ('{{"step":"{0}","status":"{1}","detail":"{2}"}}' -f $step, $status, $d)
}
function Die([string]$step, [string]$detail) { Say $step 'error' $detail; exit 1 }

# arguments (both --kebab and -Pascal spellings)
$i = 0
while ($i -lt $args.Count) {
    switch -Regex ($args[$i]) {
        '^(--no-setup|-NoSetup)$' { $RunSetup = $false }
        '^(--version|-Version)$' { $i++; $Version = $args[$i] }
        '^(--dir|-Dir)$' { $i++; $InstallDir = $args[$i] }
        '^(--repo|-Repo)$' { $i++; $Repo = $args[$i] }
        default { Die 'args' ("unknown argument: " + $args[$i]) }
    }
    $i++
}

if ($env:OS -ne 'Windows_NT') { Die 'platform' 'this installer is for Windows; on Linux use scripts/install.sh' }
if (-not [Environment]::Is64BitOperatingSystem) { Die 'platform' 'only x64 builds are published' }

$Tmp = Join-Path ([IO.Path]::GetTempPath()) ('rustler-install-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $Tmp | Out-Null
try {
    # ---- how to talk to GitHub -----------------------------------------------------------------
    $Mode = ''
    $gh = Get-Command gh -ErrorAction SilentlyContinue
    if ($Token) { $Mode = 'token' }
    elseif ($gh) {
        & gh auth status *> $null
        if ($LASTEXITCODE -eq 0) { $Mode = 'gh' }
    }
    if (-not $Mode) { Die 'auth' "the repo is private: set RUSTLER_TOKEN (or GITHUB_TOKEN) to a token with read access to $Repo, or run 'gh auth login'" }
    Say 'auth' 'ok' $Mode

    $headers = @{ Authorization = "Bearer $Token"; Accept = 'application/vnd.github+json'; 'X-GitHub-Api-Version' = '2022-11-28'; 'User-Agent' = 'rustler-installer' }

    # ---- resolve the release ---------------------------------------------------------------------
    $assetId = @{}
    if ($Mode -eq 'token') {
        $url = if ($Version) { "$Api/repos/$Repo/releases/tags/$Version" } else { "$Api/repos/$Repo/releases/latest" }
        try { $rel = Invoke-RestMethod -Uri $url -Headers $headers } catch { Die 'release' "release not found (or the token cannot read $Repo): $($_.Exception.Message)" }
        $Tag = $rel.tag_name
        foreach ($a in $rel.assets) { $assetId[$a.name] = $a.id }
    } else {
        if ($Version) { $Tag = $Version } else {
            $Tag = (& gh release view -R $Repo --json tagName -q .tagName)
            if ($LASTEXITCODE -ne 0 -or -not $Tag) { Die 'release' "no release found in $Repo" }
        }
    }
    Say 'release' 'ok' $Tag

    $Zip = "rustler-$Tag-windows-x64.zip"
    function Fetch([string]$name, [string]$dest) {
        if ($Mode -eq 'token') {
            if (-not $assetId.ContainsKey($name)) { Die 'download' "asset $name is not in release $Tag" }
            $h = @{ Authorization = "Bearer $Token"; Accept = 'application/octet-stream'; 'User-Agent' = 'rustler-installer' }
            try { Invoke-WebRequest -Uri "$Api/repos/$Repo/releases/assets/$($assetId[$name])" -Headers $h -OutFile $dest -UseBasicParsing }
            catch { Die 'download' "download of $name failed: $($_.Exception.Message)" }
        } else {
            $dl = Join-Path $Tmp 'dl'
            New-Item -ItemType Directory -Force -Path $dl | Out-Null
            & gh release download $Tag -R $Repo -p $name -D $dl --clobber *> $null
            if ($LASTEXITCODE -ne 0) { Die 'download' "download of $name failed" }
            Move-Item -Force (Join-Path $dl $name) $dest
        }
    }
    $zipPath = Join-Path $Tmp $Zip
    $sumsPath = Join-Path $Tmp 'SHA256SUMS.txt'
    Fetch $Zip $zipPath
    Fetch 'SHA256SUMS.txt' $sumsPath
    Say 'download' 'ok' ("$Zip " + (Get-Item $zipPath).Length + ' bytes')

    # ---- verify ----------------------------------------------------------------------------------
    $want = $null
    foreach ($line in Get-Content $sumsPath) {
        $p = $line.Trim() -split '\s+', 2
        if ($p.Count -eq 2 -and $p[1].TrimStart('*') -eq $Zip) { $want = $p[0].ToLower() }
    }
    if (-not $want) { Die 'verify' "$Zip is not listed in SHA256SUMS.txt" }
    $have = (Get-FileHash -Algorithm SHA256 $zipPath).Hash.ToLower()
    if ($want -ne $have) { Die 'verify' "sha256 mismatch for ${Zip}: expected $want, got $have" }
    Say 'verify' 'ok' "sha256 $have"

    # ---- install ---------------------------------------------------------------------------------
    $x = Join-Path $Tmp 'x'
    Expand-Archive -Path $zipPath -DestinationPath $x -Force
    $exe = Get-ChildItem -Path $x -Recurse -Filter 'rustler.exe' | Select-Object -First 1
    if (-not $exe) { Die 'install' 'the archive has no rustler.exe' }
    $src = $exe.Directory.FullName
    New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
    foreach ($item in 'rustler.exe', 'rustler-desktop.exe', 'skills', 'docs', 'README.md', 'THIRD_PARTY_NOTICES.md', 'LICENSE-NOTES.md', 'VERSION') {
        $from = Join-Path $src $item
        if (-not (Test-Path $from)) { continue }
        $to = Join-Path $InstallDir $item
        if (Test-Path $to) {
            # a running exe cannot be overwritten but can be renamed away
            try { Remove-Item -Recurse -Force $to }
            catch { Move-Item -Force $to ($to + '.old-' + [Guid]::NewGuid().ToString('N').Substring(0, 6)) }
        }
        Copy-Item -Recurse -Force $from $to
    }
    Get-ChildItem -Path $InstallDir -Filter '*.old-*' -ErrorAction SilentlyContinue | ForEach-Object { try { Remove-Item -Recurse -Force $_.FullName } catch {} }
    Say 'install' 'ok' $InstallDir

    # ---- PATH (user level, no admin) -------------------------------------------------------------
    if ($PathScope -ne 'None') {
        $cur = [Environment]::GetEnvironmentVariable('Path', $PathScope)
        if (-not $cur) { $cur = '' }
        $parts = $cur -split ';' | Where-Object { $_ -ne '' }
        if ($parts | Where-Object { $_.TrimEnd('\') -ieq $InstallDir.TrimEnd('\') }) { Say 'path' 'ok' "already on the $PathScope PATH" }
        else {
            [Environment]::SetEnvironmentVariable('Path', (($parts + $InstallDir) -join ';'), $PathScope)
            Say 'path' 'ok' "added $InstallDir to the $PathScope PATH (open a new terminal)"
        }
        if (-not (($env:Path -split ';') | Where-Object { $_.TrimEnd('\') -ieq $InstallDir.TrimEnd('\') })) { $env:Path = "$env:Path;$InstallDir" }
    } else { Say 'path' 'skipped' 'RUSTLER_PATH_SCOPE=None' }

    # ---- Claude Code skill -----------------------------------------------------------------------
    $skillSrc = Join-Path $InstallDir 'skills\rustler'
    if ((Test-Path $ClaudeDir) -and (Test-Path $skillSrc)) {
        $skillDst = Join-Path $ClaudeDir 'skills\rustler'
        New-Item -ItemType Directory -Force -Path (Join-Path $ClaudeDir 'skills') | Out-Null
        if (Test-Path $skillDst) { Remove-Item -Recurse -Force $skillDst }
        Copy-Item -Recurse -Force $skillSrc $skillDst
        Say 'skill' 'ok' $skillDst
    } else {
        Say 'skill' 'skipped' "no $ClaudeDir; copy $skillSrc to ~\.claude\skills\rustler to give Claude Code the skill"
    }

    # ---- smoke test + setup ----------------------------------------------------------------------
    $bin = Join-Path $InstallDir 'rustler.exe'
    $ver = (& $bin --version 2>&1 | Select-Object -First 1)
    if ($LASTEXITCODE -ne 0) { Die 'smoke' 'rustler.exe --version failed' }
    Say 'version' 'ok' "$ver"
    if ($RunSetup) {
        Say 'setup' 'start' 'rustler setup run (downloads ffmpeg, yt-dlp, uv, the Python engine and models; several minutes)'
        & $bin setup run | Out-Null
        if ($LASTEXITCODE -ne 0) { Die 'setup' 'rustler setup run failed; re-run it manually' }
        Say 'setup' 'ok' "run 'rustler doctor run' to check"
    } else { Say 'setup' 'skipped' "--no-setup; run 'rustler setup run' later" }
    Say 'done' 'ok' $InstallDir
}
finally {
    Remove-Item -Recurse -Force $Tmp -ErrorAction SilentlyContinue
}
