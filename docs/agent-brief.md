# Brief for implementation agents (read first)

- Plan: `docs/superpowers/plans/2026-11-15-slice1-first-real-clip.md` — read header, Global Constraints,
  Review Focus, File Structure, the "5-HOUR SCOPE" section at the end, and **your task block** + the
  Interfaces of tasks you consume. Spec: `docs/superpowers/specs/2026-11-15-rustler-design.md`.
- Frozen contracts: `crates/rustler-core` models/ids/errors/events and `crates/rustler-registry`. Do not change
  public signatures in files you don't own. If you truly need a contract change, stop and report it.
- Edit only the files your task owns. Fill `todo!()` bodies; keep signatures.
- **Windows toolchain:** run `cargo` through the **PowerShell** tool (in Git Bash `link.exe` resolves to GNU
  coreutils `link` and linking fails). If a `.cargo/config.toml` linker workaround exists, it is already set.
- Tools for tests: ffmpeg/ffprobe/yt-dlp/uv live in `~/.rustler/bin` once `rustler setup` (Task 4) has run.
  Until then, tool-dependent tests must **skip with a printed reason**, never fail. Test helper env:
  `RUSTLER_HOME` (default `~/.rustler`).
- TDD: write failing tests, see them fail, implement, see them pass. Report real command output.
- Commit in your worktree/branch with message per plan, ending with a blank line +
  `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Do not push; the orchestrator merges and pushes.
- Final report: files changed, tests run + real results, deviations from the plan and why, open issues.
- **Already installed on this machine (2026-11-15):** `~/.rustler/bin/{ffmpeg,ffprobe,yt-dlp,uv}.exe`
  (FFmpeg BtbN GPL build with libass; `h264_nvenc` probe OK on the RTX 4060). Test speech sample:
  `~/.rustler/test-media/jfk.wav`. So tool-dependent tests CAN and SHOULD run here.
- **No AI API integrations** anywhere (user decision): no LLM/vision/STT/TTS HTTP calls. The driving agent
  does all thinking; Rustler is local tools only.
