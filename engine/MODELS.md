# Engine models: licenses, sources, checksums

Rule: Rustler calls no AI APIs. Every model below runs locally in the Python engine and was
license-checked before it was added. Models live under `$RUSTLER_HOME/models/` (default
`~/.rustler/models/`) unless noted. Checked 2026-11-15.

| Model | Used by | License | Where it comes from |
|---|---|---|---|
| YAMNet (ONNX export) | `audio.events` | Apache-2.0 | downloaded on first use |
| PP-OCRv6 small det + rec (via RapidOCR) | `ocr.run` | Apache-2.0 | bundled inside the `rapidocr` wheel |
| PP-OCRv5 Devanagari rec mobile (ONNX) | `ocr.run` `lang: devanagari\|auto` | Apache-2.0 | downloaded on first use / `models.fetch {ocr_devanagari}` / `rustler setup` |
| YuNet `face_detection_yunet_2023mar` | `faces.track` | MIT | downloaded on first use |
| pyannote segmentation 3.0 (sherpa-onnx ONNX) | `speakers.diarize` | MIT | downloaded on first use |
| WeSpeaker ResNet34 (VoxCeleb, sherpa-onnx ONNX) | `speakers.diarize` | Apache-2.0 | downloaded on first use |
| faster-whisper models (`small` default) | `transcribe.run` | see the model card; not re-verified in this pass | Hugging Face cache (`HF_HOME`) |

## YAMNet (audio events: laughter, applause, cheering, music, speech, silence)

- Upstream model: Google YAMNet v1 (`google/yamnet` on Kaggle, formerly `tfhub.dev/google/yamnet/1`),
  code in `tensorflow/models` `research/audioset/yamnet`. **License: Apache-2.0**
  (Copyright 2022 Google LLC). The AudioSet ontology/labels are CC BY 4.0 (Google).
- ONNX export used: Hugging Face `audiomagic/yamnet-onnx` (no login needed), pinned to commit
  `f25b741c2f0bdc6d7e6db24b5fddda23347dbafd`. Repo license field `apache-2.0` and a full Apache-2.0
  `LICENSE` file are in the repo. The README states the export is a pure `tf2onnx` conversion
  (opset 15), no retraining/quantization, mel front-end baked into the graph. Upstream source archive
  sha256 per that README: `b80da2a1a56926fb0767205051a200dd7b3beaf3ea1ea126c42a53943996e5e0`.
- Files (downloaded into `$RUSTLER_HOME/models/yamnet/`, sha256 verified by the engine on every start):

  | File | URL | sha256 | Size |
  |---|---|---|---|
  | `yamnet.onnx` | https://huggingface.co/audiomagic/yamnet-onnx/resolve/f25b741c2f0bdc6d7e6db24b5fddda23347dbafd/yamnet.onnx | `d3835ffbbd4a1bb3e777f0ca217b5007907f5171dd5d17c4236b95b2af8f908e` | 16,093,355 |
  | `yamnet_class_map.csv` | https://huggingface.co/audiomagic/yamnet-onnx/resolve/f25b741c2f0bdc6d7e6db24b5fddda23347dbafd/yamnet_class_map.csv | `cdf24d193e196d9e95912a2667051ae203e92a2ba09449218ccb40ef787c6df2` | 14,096 |

  The same `yamnet.onnx` bytes (same sha256) are also published as `funground-hq/yamnet-onnx`
  (Apache-2.0), which cross-checks the file.
- Input: `waveform` float32 mono 16 kHz in [-1, 1]; outputs per-0.48 s frame (0.96 s window) scores for
  521 AudioSet classes. The engine folds classes into our labels by display name (see
  `group_indices` in `rustler_engine/audio_events.py`): laughter (Laughter, Giggle, Snicker, Belly
  laugh, Chuckle, Baby laughter), applause (Applause, Clapping), cheer (Cheering, Crowd x0.7), music
  (Music, Musical instrument, music genres, Singing), speech (Speech, Conversation, Narration, ...),
  silence (Silence, plus an RMS < -55 dBFS rule).
- Limits (verified on this machine): real applause (public-domain Wikimedia sample) is detected with
  score > 0.8; **synthetic** noise bursts are not recognised as applause by the model, so tests use a
  real sample. If the model cannot be downloaded the engine falls back to a spectral heuristic and
  reports `"model": "heuristic"`; the heuristic can only separate silence / speech / music / applause.

## RapidOCR (on-screen text)

- Package: `rapidocr` 3.10.0 (PyPI), **License-Expression: Apache-2.0**, source
  https://github.com/RapidAI/RapidOCR. Backend: `onnxruntime` 1.32.0 (MIT).
  (`rapidocr-onnxruntime` 1.4.4 was not used: it depends on non-headless `opencv-python`, which would
  clash with `opencv-python-headless`.)
- Models are shipped inside the wheel (`rapidocr/models/`), so there is no download step. The wheel's
  `MODEL_LICENSES.md` records them as PaddleOCR weights, **Apache-2.0**
  (https://github.com/PaddlePaddle/PaddleOCR/blob/main/LICENSE; model cards
  https://huggingface.co/PaddlePaddle/PP-OCRv6_small_det_onnx and
  https://huggingface.co/PaddlePaddle/PP-OCRv6_small_rec_onnx):

  | File (in the wheel) | sha256 |
  |---|---|
  | `PP-OCRv6_det_small.onnx` | `090f04abcd9d9a7498bc4ebf677e4cb9bdce1fe4197ddb7e529f1ef44e1ff94f` |
  | `PP-OCRv6_rec_small.onnx` | `6f327246b50388f3c176ae304bd95767ea6dc0c9ae92153ef8cbe210b3c14884` |
  | `ch_ppocr_mobile_v2.0_cls_mobile.onnx` (shipped, not used: `use_cls=false`) | `e47acedf663230f8863ff1ab0e64dd2d82b838fceb5957146dab185a89d6215c` |

- Scripts: the multilingual PP-OCRv6 small model reads Chinese, English and other Latin-script languages.
  **Devanagari (Hindi) on-screen text is not supported by this model**: use `lang: "devanagari"` or `"auto"`
  (next section); `analyze.ocr` says so when the Latin pass finds nothing.
- Speed on this laptop (CPU, 8 threads): about 0.5 s per frame at <= 1024 px wide. Unchanged frames
  are skipped by a thumbnail comparison.

## PaddleOCR Devanagari recognition model (Hindi on-screen text)

Used by `ocr.run` with `lang: "devanagari"` / `"auto"` (`analyze.ocr --lang ...`). Checked 2026-11-15.

- Model: PaddleOCR **PP-OCRv5 Devanagari mobile recognition** model, ONNX conversion maintained by the
  RapidOCR project (the same project and release as our `rapidocr` 3.10.0 runtime; the URL and hash below
  are the ones listed in the wheel's `default_models.yaml`). The detection stage stays the bundled PP-OCRv6
  detector. The character dictionary is embedded in the ONNX file's metadata (no separate dict file).
- **License: Apache-2.0.** Evidence: the upstream model card
  https://huggingface.co/PaddlePaddle/devanagari_PP-OCRv5_mobile_rec declares `license: apache-2.0`
  (public, no gate); PaddleOCR is Apache-2.0 (https://github.com/PaddlePaddle/PaddleOCR/blob/main/LICENSE);
  RapidOCR (code and the ModelScope model repository `RapidAI/RapidOCR`) is Apache-2.0 (ModelScope API
  field `License: "Apache License 2.0"`). Training-data terms were not separately audited.
- File (downloaded without login into `$RUSTLER_HOME/models/ocr/`, sha256 verified on every start):

  | File | URL | sha256 | Size |
  |---|---|---|---|
  | `devanagari_PP-OCRv5_rec_mobile.onnx` | https://www.modelscope.cn/models/RapidAI/RapidOCR/resolve/v3.10.0/onnx/PP-OCRv5/rec/devanagari_PP-OCRv5_rec_mobile.onnx | `d6f0a906580e3fa6b324a318718f1f31f268b6ea8ef985f91c2012a37f52c91e` | 7,940,361 |

  (The hash equals the `SHA256` recorded for this model in the installed `rapidocr` wheel and the bytes we
  downloaded.) The task brief mentioned PP-OCRv3; RapidOCR's zoo offers v4 and v5 Devanagari models, v5 is
  the newest and was used.
- Verified on this machine: a frame rendered with ffmpeg drawtext (Noto Sans Devanagari) reading
  `नमस्ते दोस्तों` is recognised as two lines `नमस्ते` (conf 0.95) and `दोस्तों` (conf 1.00) in about 1.5 s
  on CPU; the Latin model returns nothing for it. Mixed frames: `auto` keeps Latin lines from the Latin
  pass and replaces / adds Devanagari lines from the Devanagari pass.

## YuNet (face detection)

- `face_detection_yunet_2023mar.onnx`, OpenCV Zoo, **MIT**.
  URL https://github.com/opencv/opencv_zoo/raw/main/models/face_detection_yunet/face_detection_yunet_2023mar.onnx
  sha256 `8f2383e4dd3cfbb4553ea8718107fc0423210dc964f9f4280604804ed2552fa4` (232,589 bytes; hash of the file
  currently installed on the dev machine, not enforced by the downloader).
- Downloaded to `$RUSTLER_HOME/models/yunet/` on first use of `faces.track`.

## Speaker diarization (sherpa-onnx + pyannote segmentation + WeSpeaker embeddings)

Used by `speakers.diarize` (`analyze.speakers`). Runtime: `sherpa-onnx` 2.0.1 (PyPI wheels for
Windows/Linux/macOS, **Apache-2.0**, https://github.com/k2-fsa/sherpa-onnx; it ships its own onnxruntime
as `sherpa-onnx-core`). Models are downloaded into `$RUSTLER_HOME/models/diarization/` on first use and
sha256-verified on every start. Checked 2026-11-15.

| Role | Model | License | File and source | sha256 | Size |
|---|---|---|---|---|---|
| Segmentation | `pyannote/segmentation-3.0`, converted to ONNX by sherpa-onnx | **MIT** (Copyright 2022 CNRS) | `model.onnx` inside https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-segmentation-models/sherpa-onnx-pyannote-segmentation-3-0.tar.bz2 (stored as `pyannote-segmentation-3-0.onnx`) | archive `24615ee884c897d9d2ba09bb4d30da6bb1b15e685065962db5b02e76e4996488`; `model.onnx` `220ad67ca923bef2fa91f2390c786097bf305bceb5e261d4af67b38e938e1079` | 5,992,913 |
| Embedding | WeSpeaker ResNet34, VoxCeleb, sherpa-onnx export with metadata | **Apache-2.0** | https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-recongition-models/wespeaker_en_voxceleb_resnet34.onnx | `5ef208a9da1453335308a6b6f4e6dfbd7e183a38b604de0a57664f45d257fe94` (matches the release's `checksum.txt`) | 26,534,365 |

License evidence (read, not assumed):

- The segmentation tarball contains a `LICENSE` file with the MIT text (sha256
  `14d7016ad68e7394d6e6b78d96cc2ae431c905287b89674cfdf021e79e62b8ba`); the engine stores it next to the
  model as `pyannote-segmentation-3-0.LICENSE`. Its README says the model is converted from
  https://huggingface.co/pyannote/segmentation-3.0, whose card declares `license: mit`. The Hugging
  Face repo asks for a contact form (access gate); the sherpa-onnx release asset needs no login.
- The WeSpeaker model comes from the Hugging Face repo `Wespeaker/wespeaker-voxceleb-resnet34`
  (`license: apache-2.0`, public, no gate; revision `ff1ac5bca8ef11e90662b879aa923979e0bd277b`, file
  `voxceleb_resnet34.onnx`, sha256 `9fea6516d7ad6bf0a76c7689f5a49b65d330fad6dde96c91bb4435ffbfe056a1`).
  The sherpa-onnx asset is the same network with sherpa metadata added (238 bytes larger), so its hash
  differs. WeSpeaker code is Apache-2.0 (https://github.com/wenet-e2e/wespeaker).
- Training-data caveat: pyannote segmentation 3.0 was trained on public diarization corpora (AISHELL,
  AliMeeting, AMI, AVA-AVD, DIHARD, Ego4D, MSDWild, REPERE, VoxConverse) and WeSpeaker ResNet34 on
  VoxCeleb; the weights' stated licenses above are what we rely on, the corpora's own terms were not
  separately audited.
- Rejected: NeMo TitaNet (`nvidia/speakerverification_en_titanet_large`) is CC-BY-4.0 (usable with
  attribution, but needs a larger download for the same job); 3D-Speaker ERes2Net/CAM++ cards were not
  verified as clean here. Not used.

How it is used: the segmentation model finds up to 3 simultaneous speakers per 10 s window, the
embedding model describes each speech piece, and sherpa-onnx's fast agglomerative clustering
(cosine distance, threshold 0.5, or a fixed `num_speakers`) joins them into speakers. Minimum speech
piece 0.3 s, minimum pause 0.5 s (shorter pauses between the same speaker are bridged).

Verified on this machine (CPU): JFK sample alone = 1 speaker; JFK + FDR (Wikimedia Commons, public
domain US federal recording) + JFK = 2 speakers with the changes at the right second. English-trained
embeddings (VoxCeleb); other languages work less reliably and are untested here (Hindi/Hinglish not
tested). The numpy `heuristic` fallback (labelled `"model": "heuristic"`) separates clearly different
voices but can split one speaker in two.

## Adding a model

1. Verify the license from the upstream repo (not a mirror's tag alone) and that the file downloads without
   a login. 2. Pin the URL to a commit/tag and record the sha256 here and in the code. 3. Verify the hash in
   code before use. 4. Add a note to `THIRD_PARTY_NOTICES.md` and `docs/research/00-license-and-dependency-inventory.md`.
