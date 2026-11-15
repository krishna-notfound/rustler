#!/usr/bin/env bash
# Rustler installer for Linux x86_64 (no root needed).
#
#   install.sh [--version vX.Y.Z] [--no-setup] [--dir <path>] [--repo owner/name]
#
# Downloads the latest GitHub release asset rustler-<tag>-linux-x64.tar.gz, verifies it against SHA256SUMS.txt,
# installs it into ~/.rustler/app (override: RUSTLER_INSTALL_DIR or --dir), links ~/.local/bin/rustler,
# copies the Claude Code skill to ~/.claude/skills/rustler when ~/.claude exists, then runs `rustler setup run`
# (skip with --no-setup). Idempotent: running it again upgrades in place.
#
# The repository is private. Authentication, first match wins:
#   1. RUSTLER_TOKEN or GITHUB_TOKEN (a token that can read the repo's releases) -> GitHub API + curl
#   2. an authenticated `gh` CLI (gh auth status)
#
# Environment: RUSTLER_INSTALL_DIR, RUSTLER_BIN_DIR (default ~/.local/bin), RUSTLER_CLAUDE_DIR (default ~/.claude),
#              RUSTLER_REPO (default krishna-notfound/rustler), RUSTLER_HOME (data dir used by `rustler setup`).
set -euo pipefail

REPO="${RUSTLER_REPO:-krishna-notfound/rustler}"
INSTALL_DIR="${RUSTLER_INSTALL_DIR:-$HOME/.rustler/app}"
BIN_DIR="${RUSTLER_BIN_DIR:-$HOME/.local/bin}"
CLAUDE_DIR="${RUSTLER_CLAUDE_DIR:-$HOME/.claude}"
TOKEN="${RUSTLER_TOKEN:-${GITHUB_TOKEN:-}}"
VERSION=""
RUN_SETUP=1
API="https://api.github.com"

say() { # say <step> <status> [detail]
  printf '{"step":"%s","status":"%s","detail":"%s"}\n' "$1" "$2" "${3:-}"
}
die() { say "$1" "error" "$2"; exit 1; }

while [ $# -gt 0 ]; do
  case "$1" in
    --no-setup) RUN_SETUP=0 ;;
    --version) VERSION="${2:?--version needs a value}"; shift ;;
    --dir) INSTALL_DIR="${2:?--dir needs a value}"; shift ;;
    --repo) REPO="${2:?--repo needs a value}"; shift ;;
    -h|--help) sed -n '2,19p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) die args "unknown argument: $1" ;;
  esac
  shift
done

[ "$(uname -s)" = "Linux" ] || die platform "this installer is for Linux; on Windows use scripts/install.ps1"
case "$(uname -m)" in x86_64|amd64) ;; *) die platform "only x86_64 builds are published (this machine: $(uname -m))" ;; esac
command -v tar >/dev/null || die deps "tar is required"
command -v sha256sum >/dev/null || die deps "sha256sum is required"

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# ---- how to talk to GitHub -----------------------------------------------------------------------
MODE=""
if [ -n "$TOKEN" ]; then
  command -v curl >/dev/null || die deps "curl is required when using a token"
  MODE=token
elif command -v gh >/dev/null && gh auth status >/dev/null 2>&1; then
  MODE=gh
else
  die auth "the repo is private: set RUSTLER_TOKEN (or GITHUB_TOKEN) to a token with read access to $REPO, or run 'gh auth login'"
fi
say auth ok "$MODE"

api_get() { # api_get <path> -> stdout
  curl -fsSL -H "Authorization: Bearer $TOKEN" -H "Accept: application/vnd.github+json" \
       -H "X-GitHub-Api-Version: 2022-11-28" "$API$1"
}

# ---- resolve the release ------------------------------------------------------------------------
declare -A ASSET_ID
if [ "$MODE" = token ]; then
  if [ -n "$VERSION" ]; then REL="$(api_get "/repos/$REPO/releases/tags/$VERSION")" || die release "release $VERSION not found (or the token cannot read $REPO)"
  else REL="$(api_get "/repos/$REPO/releases/latest")" || die release "no release found (or the token cannot read $REPO)"; fi
  TAG="$(printf '%s\n' "$REL" | sed -n 's/^  "tag_name": "\(.*\)",$/\1/p' | head -1)"
  [ -n "$TAG" ] || die release "could not read tag_name from the release"
  while read -r name id; do ASSET_ID["$name"]="$id"; done < <(printf '%s\n' "$REL" | awk '
    /"url": "https:\/\/api.github.com\/repos\/[^"]*\/releases\/assets\/[0-9]+"/ { match($0, /assets\/[0-9]+/); id = substr($0, RSTART + 7, RLENGTH - 7) }
    /^      "name": "/ { if (id != "") { n = $0; sub(/^      "name": "/, "", n); sub(/",?$/, "", n); print n, id; id = "" } }')
else
  if [ -n "$VERSION" ]; then TAG="$VERSION"; else TAG="$(gh release view -R "$REPO" --json tagName -q .tagName)" || die release "no release found in $REPO"; fi
fi
say release ok "$TAG"

TAR="rustler-$TAG-linux-x64.tar.gz"
fetch() { # fetch <asset name> <dest>
  if [ "$MODE" = token ]; then
    id="${ASSET_ID[$1]:-}"
    [ -n "$id" ] || die download "asset $1 is not in release $TAG"
    curl -fsSL -H "Authorization: Bearer $TOKEN" -H "Accept: application/octet-stream" "$API/repos/$REPO/releases/assets/$id" -o "$2" \
      || die download "download of $1 failed"
  else
    gh release download "$TAG" -R "$REPO" -p "$1" -D "$TMP/dl" --clobber >/dev/null 2>&1 || die download "download of $1 failed"
    mv "$TMP/dl/$1" "$2"
  fi
}
mkdir -p "$TMP/dl"
fetch "$TAR" "$TMP/$TAR"
fetch SHA256SUMS.txt "$TMP/SHA256SUMS.txt"
say download ok "$TAR $(stat -c %s "$TMP/$TAR") bytes"

# ---- verify -------------------------------------------------------------------------------------
WANT="$(awk -v f="$TAR" '{n=$2; sub(/^\*/,"",n); if (n==f) print $1}' "$TMP/SHA256SUMS.txt" | head -1)"
[ -n "$WANT" ] || die verify "$TAR is not listed in SHA256SUMS.txt"
HAVE="$(sha256sum "$TMP/$TAR" | cut -d' ' -f1)"
[ "$WANT" = "$HAVE" ] || die verify "sha256 mismatch for $TAR: expected $WANT, got $HAVE"
say verify ok "sha256 $HAVE"

# ---- install ------------------------------------------------------------------------------------
mkdir -p "$TMP/x"
tar -xzf "$TMP/$TAR" -C "$TMP/x"
SRC="$(find "$TMP/x" -maxdepth 2 -name rustler -type f | head -1)"
[ -n "$SRC" ] || die install "the archive has no rustler binary"
SRC="$(dirname "$SRC")"
mkdir -p "$INSTALL_DIR"
# replace the managed files; never touch anything else that lives in INSTALL_DIR
for item in rustler skills docs README.md THIRD_PARTY_NOTICES.md LICENSE-NOTES.md VERSION; do
  [ -e "$SRC/$item" ] || continue
  rm -rf "${INSTALL_DIR:?}/$item"
  cp -a "$SRC/$item" "$INSTALL_DIR/$item"
done
chmod 755 "$INSTALL_DIR/rustler"
say install ok "$INSTALL_DIR"

# ---- PATH (user level) --------------------------------------------------------------------------
mkdir -p "$BIN_DIR"
ln -sf "$INSTALL_DIR/rustler" "$BIN_DIR/rustler"
case ":$PATH:" in
  *":$BIN_DIR:"*) say path ok "$BIN_DIR is on PATH" ;;
  *) say path hint "add $BIN_DIR to PATH: echo 'export PATH=\"$BIN_DIR:\$PATH\"' >> ~/.bashrc && . ~/.bashrc" ;;
esac

# ---- Claude Code skill --------------------------------------------------------------------------
if [ -d "$CLAUDE_DIR" ] && [ -d "$INSTALL_DIR/skills/rustler" ]; then
  mkdir -p "$CLAUDE_DIR/skills"
  rm -rf "$CLAUDE_DIR/skills/rustler"
  cp -a "$INSTALL_DIR/skills/rustler" "$CLAUDE_DIR/skills/rustler"
  say skill ok "$CLAUDE_DIR/skills/rustler"
else
  say skill skipped "no $CLAUDE_DIR; copy $INSTALL_DIR/skills/rustler to ~/.claude/skills/rustler to give Claude Code the skill"
fi

# ---- smoke test + setup -------------------------------------------------------------------------
VER="$("$INSTALL_DIR/rustler" --version 2>&1 | head -1)" || die smoke "rustler --version failed (glibc too old? build from source: docs/linux.md)"
say version ok "$VER"
if [ "$RUN_SETUP" = 1 ]; then
  say setup start "rustler setup run (downloads ffmpeg, yt-dlp, uv, the Python engine and models; several minutes)"
  "$INSTALL_DIR/rustler" setup run >"$TMP/setup.json" || die setup "rustler setup run failed; re-run it manually"
  say setup ok "run 'rustler doctor run' to check"
else
  say setup skipped "--no-setup; run 'rustler setup run' later"
fi
say done ok "$INSTALL_DIR"
