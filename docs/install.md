# Installing Rustler (agents: read this to install it for the user)

Prebuilt binaries are published as GitHub release assets of the **private** repo
`krishna-notfound/rustler`. Two one-file installers (`scripts/install.ps1`, `scripts/install.sh`) download the
latest release, verify its SHA256, install it without admin/root, add it to the user PATH, give Claude Code the
skill and run `rustler setup run`.

## Release assets (v0.1.0)

| Asset | Contents |
|---|---|
| `rustler-v0.1.0-windows-x64.zip` | `rustler.exe` (CLI, MCP server, web GUI), `rustler-desktop.exe` (native window), `skills/`, `docs/`, `README.md`, `THIRD_PARTY_NOTICES.md`, `LICENSE-NOTES.md`, `VERSION` |
| `rustler-v0.1.0-linux-x64.tar.gz` | `rustler` (CLI, MCP server, web GUI; no desktop window), `skills/`, `docs/`, `README.md`, `THIRD_PARTY_NOTICES.md`, `LICENSE-NOTES.md`, `VERSION` |
| `SHA256SUMS.txt` | sha256 of the two archives (`sha256sum -c` format) |

The Linux binary is built against glibc 2.31 (zig cc), so it runs on Ubuntu 20.04+ / Debian 11+ and newer.
Only x86_64 Linux and Windows x64 are published. macOS and aarch64 are not built.

## Authentication (the repo is private)

The installer needs one of these (first match wins):

1. `RUSTLER_TOKEN` or `GITHUB_TOKEN` in the environment: a GitHub token with read access to the repo's
   contents/releases (fine-grained: *Contents: read*; classic: `repo`). Used with the GitHub API asset
   endpoint and `Accept: application/octet-stream`.
2. An authenticated GitHub CLI: `gh auth login` (then `gh auth status` succeeds).

Without either, the installer stops with `{"step":"auth","status":"error",...}` and tells you what to set.

## Windows (PowerShell 5.1 or 7, no admin)

```powershell
# with gh logged in: fetch the installer from the private repo and run it
gh api repos/krishna-notfound/rustler/contents/scripts/install.ps1 -H "Accept: application/vnd.github.raw" > $env:TEMP\rustler-install.ps1
powershell -ExecutionPolicy Bypass -File $env:TEMP\rustler-install.ps1
# or, from a checkout of the repo:  powershell -ExecutionPolicy Bypass -File scripts\install.ps1
```

Flags: `--version v0.1.0` (default: latest), `--no-setup` (skip `rustler setup run`), `--dir <path>`,
`--repo owner/name` (`-NoSetup`, `-Version`, `-Dir`, `-Repo` also work).
Installs into `%USERPROFILE%\.rustler\app` (override `RUSTLER_INSTALL_DIR`), adds that folder to the **user** PATH
(`[Environment]::SetEnvironmentVariable(..., 'User')`; open a new terminal), copies the skill to
`%USERPROFILE%\.claude\skills\rustler` if `%USERPROFILE%\.claude` exists.

## Linux x86_64 (no root)

```sh
# with gh logged in
gh api repos/krishna-notfound/rustler/contents/scripts/install.sh -H "Accept: application/vnd.github.raw" | bash
# with a token instead of gh
curl -fsSL -H "Authorization: Bearer $RUSTLER_TOKEN" -H "Accept: application/vnd.github.raw" \
  https://api.github.com/repos/krishna-notfound/rustler/contents/scripts/install.sh | bash
# pass flags:  ... | bash -s -- --no-setup --version v0.1.0
```

Installs into `~/.rustler/app` (override `RUSTLER_INSTALL_DIR` / `--dir`), symlinks `~/.local/bin/rustler`
(override `RUSTLER_BIN_DIR`; prints a PATH hint when `~/.local/bin` is not on PATH), copies the skill to
`~/.claude/skills/rustler` if `~/.claude` exists (override `RUSTLER_CLAUDE_DIR`). Needs `curl` (or `gh`), `tar`,
`sha256sum`; no compiler.

## What the installers print

One JSON-ish line per step on stdout, for agents to parse:
`{"step":"auth|release|download|verify|install|path|skill|version|setup|done","status":"ok|skipped|hint|start|error","detail":"..."}`.
Exit code is non-zero after an `error` line. Re-running is safe (idempotent): files managed by the installer are
replaced, nothing else in the install folder is touched, PATH is not duplicated.

## After install

```sh
rustler --version
rustler setup run              # unless the installer already ran it: FFmpeg, yt-dlp, uv, Python engine, models, fonts, assets
rustler doctor run             # "ready": true means you can render
rustler commands list          # 118 commands
```

`rustler setup run --music-pack` additionally downloads the optional CC0 music starter pack (11 FreePD tracks,
about 50 MB). Music is never installed by default: bring any track with `rustler asset add` or
`rustler audio add-music <clip> --path <file>`.
MCP for Claude Code: `claude mcp add rustler -- rustler mcp serve`. Windows desktop window: `rustler-desktop.exe`
(see `docs/desktop.md`). Linux build/run details: `docs/linux.md`.

## Manual download (no installer)

```sh
gh release download v0.1.0 -R krishna-notfound/rustler -p "rustler-v0.1.0-linux-x64.tar.gz" -p SHA256SUMS.txt
sha256sum -c --ignore-missing SHA256SUMS.txt
tar -xzf rustler-v0.1.0-linux-x64.tar.gz     # run ./rustler-v0.1.0-linux-x64/rustler ; copy skills/rustler to ~/.claude/skills/
```

Windows: `gh release download v0.1.0 -R krishna-notfound/rustler -p "*windows-x64.zip" -p SHA256SUMS.txt`,
check with `(Get-FileHash <zip>).Hash` against `SHA256SUMS.txt`, then `Expand-Archive`.
Or open the release page in the browser (signed in to GitHub) and download the assets by hand.

## Maintainers: cutting a release

1. Windows: PowerShell, `$env:CARGO_BUILD_JOBS=2; cargo build --release -p rustler-cli -p rustler-desktop`.
2. Linux (WSL, no root, glibc 2.31 floor): the zig cc recipe of `docs/linux.md` section 2b with the target
   rewritten to `--target=x86_64-linux-gnu.2.31`, then `cargo build --release -p rustler-cli`.
3. Package: a folder `rustler-<tag>-<os>-x64/` holding the binaries, `skills/`, `docs/`, `README.md`,
   `THIRD_PARTY_NOTICES.md`, `LICENSE-NOTES.md` and a `VERSION` file; zip (Windows) / tar.gz (Linux); `SHA256SUMS.txt`
   lists both.
4. `gh release create <tag> <assets> --title "Rustler <tag>" --notes-file <notes> --target <branch>`. The repo
   stays private.
