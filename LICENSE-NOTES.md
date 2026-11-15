# License notes

- **Rustler itself**: MIT (`license = "MIT"` in the workspace `Cargo.toml`). No separate LICENSE file text has been
  added to the repository yet; the SPDX identifier above is the declaration.
- **Third-party code and fonts**: see `THIRD_PARTY_NOTICES.md`.
- **Tools downloaded by `rustler setup`** keep their own licenses and are not part of these release archives:
  FFmpeg (the BtbN static GPL build on Windows/Linux, with libass), yt-dlp (Unlicense), uv (MIT/Apache-2.0),
  faster-whisper and the other Python packages (see `engine/pyproject.toml`), fonts (SIL OFL 1.1).
- **Bundled media**: none. The asset library is filled at setup time with CC0 Kenney sound effects and OFL fonts.
  Music is never installed by default; the optional `rustler setup run --music-pack` fetches 11 FreePD tracks
  (CC0 1.0 Universal, per the statement on freepd.com; every manifest entry records license, source URL and sha256).
