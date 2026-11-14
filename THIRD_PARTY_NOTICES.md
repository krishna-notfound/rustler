# Third-party notices

Rustler may include code copied or adapted from the MIT-licensed projects below, and
fonts fetched at setup time from upstream google/fonts under the SIL Open Font License.
Every copied snippet gets an entry here (project, file, commit, what was taken).

## Code (MIT)

_None copied yet._ Allowed sources: openshorts, autoclip, bridgeclip, opensource-clipping.
supoclip, ViralCutter and hypit are ideas-only (no code).

## Fonts (SIL OFL 1.1)

Downloaded by `rustler setup` into `~/.rustler/fonts/<family>/` together with each
family's `OFL.txt` (not bundled in this repository):

- Anton (google/fonts `ofl/anton`)
- Montserrat (google/fonts `ofl/montserrat`)
- Noto Sans Devanagari (google/fonts `ofl/notosansdevanagari`)

## Adapted code (MIT)
- Render filtergraph approach (frame-grid allocation, audio sync, blur-fit backdrop) — adapted from
  BridgeClip (MIT, (c) BridgeMind LLC): https://github.com/bridge-mind/bridgeclip
- Music ducking filter chain — adapted from opensource-clipping (MIT, (c) NaufalRizqullah):
  https://github.com/NaufalRizqullah/opensource-clipping
- Crop-path emission via `sendcmd` — idea from OpenShorts (MIT, (c) mutonby): https://github.com/mutonby/openshorts
- Face-path smoothing (One-Euro, dead zone, velocity clamp) — ported from autoclip (MIT, (c) artbyjazi):
  https://github.com/artbyjazi/autoclip
- Face detector model YuNet `face_detection_yunet_2023mar.onnx` — MIT, (c) 2020 Shiqi Yu, fetched at runtime
  from opencv_zoo.
- Speaker diarization (`speakers.diarize`): runtime sherpa-onnx (Apache-2.0, (c) the k2-fsa authors); models
  pyannote segmentation 3.0 as converted by sherpa-onnx (MIT, (c) 2022 CNRS) and WeSpeaker ResNet34
  (Apache-2.0), fetched at runtime into `~/.rustler/models/diarization/`. URLs, hashes and license evidence:
  `engine/MODELS.md`.

## Fonts (SIL Open Font License 1.1, fetched from google/fonts at setup with OFL.txt)
Anton, Poppins (ExtraBold, Bold), Noto Sans Devanagari.

## Sound effects (CC0, fetched at setup into `~/.rustler/assets/sfx/`)
`rustler setup` (skip with `--no-assets`) downloads two packs by Kenney (www.kenney.nl) and keeps a curated
subset of 26 files. Both packs ship a `License.txt` that states "Creative Commons Zero, CC0"
(http://creativecommons.org/publicdomain/zero/1.0/); setup refuses to register a pack whose License.txt
does not say so. Credit to Kenney is appreciated but not required. Each file has a manifest entry
(`license: CC0-1.0`, `source_url`, `sha256`) in `~/.rustler/assets/manifest.json`.
- Interface Sounds 1.0 - https://kenney.nl/assets/interface-sounds
- Impact Sounds 1.0 - https://kenney.nl/assets/impact-sounds

No music is bundled or fetched; user-added assets carry the license the user declares via `asset add --license`.
