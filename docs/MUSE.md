# Giving Rustler to Muse (or any cloud agent)

The user is travelling; Muse runs Rustler on its own Linux machine and makes the clips.

## 1. Access (private repo)
Muse needs a GitHub token with read access to `krishna-notfound/rustler`
(fine-grained token → Repository access: rustler → Contents: Read). Give it to Muse as `RUSTLER_TOKEN`.

## 2. Install (Linux, no GPU needed, no root)
```bash
export RUSTLER_TOKEN=<token>
curl -fsSL -H "Authorization: Bearer $RUSTLER_TOKEN" -H "Accept: application/vnd.github.raw" \
  https://api.github.com/repos/krishna-notfound/rustler/contents/scripts/install.sh | bash
# installs ~/.rustler/app/rustler, puts it on PATH, runs `rustler setup run` (lite, ~0.5 GB)
rustler doctor run            # must say "ready": true
```
If no release binary works on that machine, build from source:
`git clone https://$RUSTLER_TOKEN@github.com/krishna-notfound/rustler && cd rustler && cargo build --release -p rustler-cli`
(needs a C compiler; see `docs/linux.md` for the no-root zig recipe), then `./target/release/rustler setup run`.

## 3. Teach Muse
Tell Muse to read, in this order: `rustler guide show start-here`, `rustler guide show workflow-youtube-longform`,
`rustler guide show finding-hooks`, then `rustler prompt show make_viral_clips --args '{"source":"<url>","count":"5"}'`.
If Muse speaks MCP: `rustler mcp serve` (stdio). Skill file: `skills/rustler/SKILL.md`.

## 4. Copy-paste prompt for Muse
```
You are my short-form clip editor. Use the `rustler` CLI (already installed; run `rustler help`).
Source: <YouTube URL>. Make 5 strong 9:16 clips (25–60 s each) for Shorts/Reels.
1. rustler project create --name job1 --dir ~/clips/job1 ; cd ~/clips/job1
2. rustler source info <url> ; rustler source fetch-transcript <url>   (first YouTube lookup ~3 min)
   — or fetch the captions with your own tools and `rustler transcript import <url> --file captions.vtt`
3. rustler highlights candidates <pending-id> --count 20 --min-len 25 --max-len 60 ; read the best with
   `rustler transcript read-window` and pick 5 standalone moments with a hook in the first 2 seconds.
4. rustler source import <url> --sections MM:SS-MM:SS ...   (only those sections)
5. rustler analyze quick <src> --transcribe auto   (reuses YouTube captions; no whisper needed)
6. Look at frames yourself: rustler see tiles <src> ; rustler see frame --source-id <src> --t 5 --grid true
   Wide shots with on-screen graphics → blur-fit; close-ups → center-crop + `rustler crop track-face`.
7. rustler clip create ... --start-unit/--end-unit --expect-text "<first words>" --caption-style hormozi-pop
   rustler overlay add-text <clip> --text "<hook ≤ 7 words>"
8. rustler render run <clip> ; rustler qc run <clip> ; LOOK at the contact sheet ; fix ; rustler export clip <clip>
Never publish anything. Send me the files in ~/clips/job1/exports/ with each clip's report.
```

## 5. Speed notes (measured)
- First YouTube lookup ~3 min (YouTube's JS challenge); captions + section downloads then reuse it (seconds).
- Section download depends on bandwidth (~4 min for 5 × 1-min 1080p sections at 0.74 MB/s).
- Render ~5–7 s per 20-s clip; analyze with reused captions: seconds.
