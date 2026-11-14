# Engine models: licenses, sources, checksums

Rule: Rustler calls no AI APIs. Every model below runs locally in the Python engine and was
license-checked before it was added. Models live under `$RUSTLER_HOME/models/` (default
`~/.rustler/models/`) unless noted. Checked 2026-11-15.

| Model | Used by | License | Where it comes from |
|---|---|---|---|
| YAMNet (ONNX export) | `audio.events` | Apache-2.0 | downloaded on first use |
| PP-OCRv6 small det + rec (via RapidOCR) | `ocr.run` | Apache-2.0 | bundled inside the `rapidocr` wheel |
| YuNet `face_detection_yunet_2023mar` | `faces.track` | MIT | downloaded on first use |
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
  **Devanagari (Hindi) on-screen text is not supported by this model**; `analyze.ocr` says so when it
  finds nothing.
- Speed on this laptop (CPU, 8 threads): about 0.5 s per frame at <= 1024 px wide. Unchanged frames
  are skipped by a thumbnail comparison.

## YuNet (face detection)

- `face_detection_yunet_2023mar.onnx`, OpenCV Zoo, **MIT**.
  URL https://github.com/opencv/opencv_zoo/raw/main/models/face_detection_yunet/face_detection_yunet_2023mar.onnx
  sha256 `8f2383e4dd3cfbb4553ea8718107fc0423210dc964f9f4280604804ed2552fa4` (232,589 bytes; hash of the file
  currently installed on the dev machine, not enforced by the downloader).
- Downloaded to `$RUSTLER_HOME/models/yunet/` on first use of `faces.track`.

## Adding a model

1. Verify the license from the upstream repo (not a mirror's tag alone) and that the file downloads without
   a login. 2. Pin the URL to a commit/tag and record the sha256 here and in the code. 3. Verify the hash in
   code before use. 4. Add a note to `THIRD_PARTY_NOTICES.md` and `docs/research/00-license-and-dependency-inventory.md`.
