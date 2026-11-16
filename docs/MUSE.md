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
# installs ~/.rustler/app/rustler, puts it on PATH, runs `rustler setup run` (lite, ~0.5 GB; latest release is v0.2.2, pin with `--version v0.2.2`; after upgrading run `rustler setup run` again)
rustler doctor run            # must say "ready": true
rustler doctor run --network  # probes YouTube through yt-dlp; tells you which fix below you need
```
If every yt-dlp call fails with `CERTIFICATE_VERIFY_FAILED` (the VM's proxy re-signs TLS with a private CA):
`rustler setup run --insecure-tls` (writes `~/.rustler/yt-dlp.conf`; no hand-made config needed). If it reports
`JS runtimes: none` / challenge errors: plain `rustler setup run` installs Deno into `~/.rustler/bin`, which
Rustler puts on `PATH` for every yt-dlp call.
If no release binary works on that machine, build from source:
`git clone https://$RUSTLER_TOKEN@github.com/krishna-notfound/rustler && cd rustler && cargo build --release -p rustler-cli`
(needs a C compiler; see `docs/linux.md` for the no-root zig recipe), then `./target/release/rustler setup run`.

## 3. Teach Muse
Tell Muse to read, in this order: `rustler guide show start-here` (the v2 recommended flow is in section 3),
`rustler guide show workflow-youtube-longform`, `rustler guide show finding-hooks`,
`rustler guide show templates-and-brand`, `rustler guide show director-retention`,
`rustler guide show campaigns-and-posting`, then
`rustler prompt show make_viral_clips --args '{"source":"<url>","count":"5"}'`.
Fast first pass over a whole source: `rustler clips auto --source-id <src> --count 5 --template podcast-viral --dry-run`, then without `--dry-run` (guide `clips-auto`; it never publishes and leaves the post copy for you to write).
Several clips with one look: `rustler prompt show apply_template_batch`. A paid campaign: `rustler prompt show direct_campaign`.
If Muse speaks MCP: `rustler mcp serve` (stdio). Skill file: `skills/rustler/SKILL.md`.

## 4. Copy-paste prompt for Muse
```
You are my short-form clip editor. Use the `rustler` CLI (already installed; run `rustler help`).
Read `rustler guide show start-here` first. Source: <YouTube URL>. Make 5 strong 9:16 clips (25-60 s each)
for Shorts/Reels. You do the thinking and you LOOK at the images; Rustler calls no AI APIs.
1. rustler project create --name job1 --dir ~/clips/job1 ; cd ~/clips/job1
2. rustler source info <url> ; rustler source fetch-transcript <url>   (first YouTube lookup ~3 min)
   - or fetch the captions with your own tools and `rustler transcript import <url> --file captions.vtt`
3. rustler highlights candidates <pending-id> --count 20 --min-len 25 --max-len 60 ; read the best with
   `rustler transcript read-window` and pick 5 standalone moments with a hook in the first 2 seconds.
4. rustler source import <url> --sections MM:SS-MM:SS ...   (only those sections)
5. rustler analyze quick <src> --transcribe auto   (reuses YouTube captions; no whisper needed)
6. Look at frames yourself: rustler see tiles <src> ; rustler see frame --source-id <src> --t 5 --grid true
7. SMART DEFAULTS: rustler clip create --name c1 --source-id <src> --start-unit A --end-unit B --expect-text "<first words>" --template podcast-viral
   It picks the layout per shot, tracks the face or active speaker, trims fillers and pauses, and applies the
   template (captions, hook look, progress bar, loudness; it never adds zooms). Read `smart.decisions`; check
   `rustler layout plan <clip>` and the preview; override only what looks wrong (clip set-layout, crop track-active).
   (Streamer with a webcam: --template streamer-split, and `rustler layout detect-webcam <src> --clip-id <clip>`.)
8. HOOK: rustler overlay add-text <clip> "<hook, 7 words max>" --style hook-big --start 0 --end 3
   (styles: `rustler overlay hook-styles`). Best line late in the moment? `rustler edit cold-open <clip> --start-unit A --end-unit B`.
   Optional: `rustler caption keywords <clip> --auto` with the keyword-highlight caption style.
9. rustler render preview-frame <clip> --out-t 1.0 ; LOOK. Then rustler render run <clip> ; rustler qc run <clip> ; LOOK at the contact sheet ; fix.
10. RETENTION CHECK: rustler clip retention <clip> ; fix the dead zones that matter with the fix YOU choose after looking
    (jump cut, text pop, cold open; a held, subtle punch-in only where a moment earns it, never constant zooming).
    Optional look: `rustler grade measure <clip>`, then `rustler grade set <clip> --preset natural-pop` (or warm/cinematic)
    and `rustler grade preview <clip> --out-t 5` ; LOOK. Re-render, check again. Drop a clip that stays weak.
11. COVER: rustler clip cover <clip> --auto ; LOOK at the image.
12. POST JSON: rustler clip meta-context <clip> ; write the title, description and hashtags for shorts/reels yourself ;
    save them with `rustler clip meta-set <clip> --input-json @meta.json` (guide `campaigns-and-posting`).
13. rustler export clip <clip> --name <slug>   (mp4 + report + srt + cover + post.json in exports/)
NEVER publish, upload or post anything. Send me the files in ~/clips/job1/exports/ with each clip's report,
what you saw on the contact sheet, and anything you could not verify.
```

## 5. Speed notes (measured)
- First YouTube lookup ~3 min (YouTube's JS challenge); captions + section downloads then reuse it (seconds).
- Section download depends on bandwidth (~4 min for 5 × 1-min 1080p sections at 0.74 MB/s).
- Render ~5–7 s per 20-s clip; analyze with reused captions: seconds.
