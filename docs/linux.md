# Rustler on Linux (x86_64, no GPU)

Validated 2026-11-15 on Ubuntu 26.04 (WSL2), x86_64, 7.8 GB RAM, glibc 2.43, with `RUSTLER_HOME=~/.rustler-linux-test`
and `--gpu none` (CPU engine, libx264). Everything below was run for real; the numbers are measured, not estimated.
Cloud agents (for example Muse) use this file to install and run Rustler on a Linux box.

## 1. Prerequisites

| Need | Why | Notes |
|---|---|---|
| Linux x86_64, glibc >= 2.39 for a prebuilt binary built on a recent distro | the `rustler` binary is dynamically linked to libc/libm only | build on the target if the box is older (Ubuntu 22.04 has 2.35) |
| `curl`, `git`, `tar`, `xz` | rustup, clone, archive extraction of the ffmpeg static build | `unzip` is not needed (zips are read in-process) |
| a C compiler and linker (`gcc` or `clang`) | `cargo build` (the `aws-lc-sys` TLS dependency compiles C and assembly) | not needed if you use a prebuilt `rustler` binary; see 2b if you have no root |
| Python | not needed; `rustler setup` installs its own Python 3.11 through uv | |
| libGL / libglib | not needed (the engine installs only `opencv-python-headless` on Linux, see Known limits) | |
| NVIDIA driver | optional; with none, setup picks the CPU engine and renders with libx264 | |
| ~3 GB free disk | `RUSTLER_HOME` is about 2.0 GB with the CPU engine (about 4.0 GB with the CUDA engine) | |

Network is needed for `rustler setup` (GitHub, PyPI, Hugging Face, ffmpeg and yt-dlp releases).

## 2. Get the binary

### 2a. Build (normal case, a compiler is installed)

```sh
curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal      # user-level, no sudo
. ~/.cargo/env
git clone <rustler repo> ~/rustler && cd ~/rustler
CARGO_BUILD_JOBS=2 cargo build --release -p rustler-cli             # ~/rustler/target/release/rustler
```

Debian/Ubuntu: `sudo apt-get install -y build-essential` provides the compiler. Edition 2024 needs a recent
stable Rust (validated with 1.100). Put the binary on PATH: `ln -s $PWD/target/release/rustler ~/.local/bin/rustler`.

### 2b. Build without root and without gcc (validated)

The validation box had no `gcc`, no `pkg-config`, no libc headers and passwordless sudo was unavailable
(`sudo -n true` fails). A user-level `zig cc` is a drop-in C compiler and linker and works for the whole workspace:

```sh
mkdir -p ~/zig ~/zigbin
curl -sSfL https://ziglang.org/download/0.15.2/zig-x86_64-linux-0.15.2.tar.xz | tar -xJ -C ~/zig --strip-components=1
# zig wants "x86_64-linux-gnu", cargo passes "x86_64-unknown-linux-gnu" and -lgcc_s; translate both
cat > ~/zigbin/cc <<'EOF'
#!/bin/bash
args=()
for a in "$@"; do
  case "$a" in
    --target=x86_64-unknown-linux-gnu) a="--target=x86_64-linux-gnu";;
    -lgcc_s) a="-lunwind";;
  esac
  args+=("$a")
done
exec "$HOME/zig/zig" cc "${args[@]}"
EOF
sed 's/zig" cc/zig" c++/' ~/zigbin/cc > ~/zigbin/c++ && chmod +x ~/zigbin/cc ~/zigbin/c++
export PATH=$HOME/zigbin:$HOME/.cargo/bin:$PATH CC=$HOME/zigbin/cc CXX=$HOME/zigbin/c++
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=$HOME/zigbin/cc
CARGO_BUILD_JOBS=2 cargo build --release -p rustler-cli
```

`ar`, `ranlib` and `perl` came from the base image (binutils). To target an older glibc, append a version to the zig
target (`--target=x86_64-linux-gnu.2.31`); this was not exercised here.

## 3. Install the tools and check

```sh
export RUSTLER_HOME=$HOME/.rustler        # default; any directory works
rustler setup                             # LITE: ffmpeg+ffprobe (static, libass), yt-dlp, uv, Python 3.11 base engine, whisper base, fonts, built-in styles
rustler setup --with ocr,speakers         # add heavy analyzers (audio-events, cuda, sfx-pack too); --profile full = everything
rustler setup --profile full --gpu none   # the previous full install; --gpu none never adds the ~2.2 GB of CUDA wheels
rustler doctor                            # JSON; `"ready": true` is the pass condition
```

`setup` is idempotent and prints JSON progress events on stderr and one result object on stdout. It needs no root.
With no NVIDIA driver it installs the CPU engine automatically (`--gpu auto` is the default); `doctor` then shows
`nvidia_gpu: null`, `libx264: ok` and the NVENC/QSV/AMF/VideoToolbox probes as failed, which is expected. Renders
use libx264 whenever no hardware encoder passes its probe, or when you pass `--prefer-gpu false`.

What `setup` puts in `RUSTLER_HOME`: `bin/{ffmpeg,ffprobe,yt-dlp,uv}` (mode 755), `engine/` (+ `.venv`), `python/`,
`models/` (whisper small, YuNet, YAMNet), `fonts/` (Anton, Poppins, Noto Sans Devanagari), `assets/` (7 built-in styles,
4 fonts, 26 CC0 SFX).

## 4. Core flow (all of it ran on Linux)

```sh
rustler project create demo --dir ./demo
rustler --project ./demo source import "/path/to/video.mp4"          # unicode paths are fine
rustler --project ./demo analyze quick src_X
rustler --project ./demo highlights candidates src_X --count 5 --min-len 8 --max-len 30
rustler --project ./demo see tiles src_X --around "phrase"
rustler --project ./demo layout detect-webcam src_X                  # streamer sources
rustler --project ./demo clip create --name hook-1 --source-id src_X --start-unit 3 --end-unit 10 --expect-text "..."
rustler --project ./demo overlay add-text clp_X "Hook" --start 0 --end 3
rustler --project ./demo asset list --kind sfx                       # then: audio add-sfx clp_X asset:<id> --at 1.0
rustler --project ./demo render run clp_X --prefer-gpu false         # libx264
rustler --project ./demo qc run clp_X
rustler --project ./demo export clip clp_X --name hook-1
rustler --project ./demo mcp serve                                   # stdio MCP (one tool per registry command)
rustler --project ./demo gui --no-open --port 47900                  # prints http://127.0.0.1:47900/?t=<token>; no token = HTTP 401
```

## 5. Measured timings (CPU only, `--gpu none`)

Machine: WSL2 VM with 24 logical CPUs exposed, 7.8 GB RAM (a smaller cloud box will be slower, mostly in the whisper
and face-track steps). Cargo used `CARGO_BUILD_JOBS=2`.

| Step | Time |
|---|---|
| `cargo build --release -p rustler-cli`, cold (incl. aws-lc) | 4 min 22 s |
| rebuild after an engine-file change | 1 min 31 s |
| `rustler setup --gpu none`, fresh home (about 2.0 GB on disk) | 3 min 01 s |
| `rustler setup` with the CUDA engine (auto, GPU visible) | 6 min 54 s |
| `source import` of a 55 s file (probe, proxy, audio) | 2.9 s |
| `analyze quick` on a 55 s talk (whisper small int8, scenes, audio) | 26.6 s |
| `analyze quick` on a 40 s stream clip | 18.6 s |
| `analyze ocr` on the 40 s clip | 11.4 s |
| `analyze faces` / `layout detect-webcam` / `analyze events` | 0.9 s / 0.6 s / 0.4 s |
| `render run` 29.4 s clip, libx264, captions + overlay + SFX, two-pass loudnorm | 7.2 s |
| `render run` 12 s clip, split-vertical + overlay + captions | 3.4 s |
| `qc run` (12 checks + contact sheet, includes face tracking) | 8 to 9 s |
| `cargo test --workspace` (debug, jobs=2, includes compile) | 7 min 20 s, 477 passed, 0 failed, 5 ignored |
| engine pytest (`uv run --project engine pytest -q`) | 114 passed, 5 skipped (14.7 s warm; 5 min on first run) |

## 6. Known limits and notes

- **Only x86_64 Linux was tested.** aarch64 downloads are not validated here.
- **Binary glibc floor**: the binary built here needs GLIBC 2.39. Build on the target (or with the zig recipe and an
  older glibc target) for older distros.
- **OpenCV**: `rapidocr` depends on the GUI `opencv-python` wheel, which overwrote the headless `cv2` with a Qt build that
  links libGL.so.1, libglib and libgthread at import and so fails on minimal server images. `engine/pyproject.toml` now
  overrides it away on Linux only (`[tool.uv] override-dependencies`). An engine installed before this fix has no usable
  `cv2` after an in-place re-sync: delete `$RUSTLER_HOME/engine` and run `rustler setup` again.
- **No GPU**: setup installs the CPU engine, whisper runs int8 on CPU, renders use libx264. Expect whisper `small` at
  roughly real time or faster on a many-core box; `medium` is much slower (`rustler setup --whisper-models medium`).
- **WSL**: the NVIDIA driver is passed through (`/usr/lib/wsl/lib/nvidia-smi`), so `setup` picks the CUDA engine and
  `render run` picks `h264_nvenc` unless you pass `--gpu none` and `--prefer-gpu false`. A true GPU-less box could not
  be reproduced inside WSL; the libx264 path was exercised with `--prefer-gpu false`.
- **yt-dlp** (`yt-dlp_linux` standalone binary) works: `source info` on a YouTube URL ran in 3 s. YouTube downloads may
  need cookies or a JS runtime depending on YouTube's current checks; that is independent of the OS.
- **Memory**: keep `CARGO_BUILD_JOBS=2` on boxes with 8 GB or less. Linking the release binary is the peak.
- Tests that need ffmpeg/yt-dlp/uv read `RUSTLER_HOME`; export it before `cargo test` so they run rather than skip.
- WSL specifics met during validation: a stale WSL VM can return `E_UNEXPECTED`; `wsl --shutdown` fixes it. `git clone`
  of a worktree directory over `/mnt/c` fails (its `.git` file holds a Windows path); clone the main repository and
  check out the branch.
