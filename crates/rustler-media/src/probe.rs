//! ffprobe wrapper. (Task 6)

use crate::proc::{self, argv};
use rustler_core::{ErrorCode, MediaProbe, RustlerError};
use rustler_tools::ToolPaths;
use serde_json::Value;
use std::path::Path;
use tokio_util::sync::CancellationToken;

/// `ffprobe -v error -print_format json -show_streams -show_format`; rotation from
/// side_data/tags; display dims swapped on +/-90.
pub async fn probe(paths: &ToolPaths, media: &Path) -> rustler_core::Result<MediaProbe> {
    if !media.exists() {
        return Err(RustlerError::new(
            ErrorCode::NotFound,
            "probe",
            format!("media file not found: {}", media.display()),
        ));
    }
    let mut args = argv!["-v", "error", "-print_format", "json", "-show_streams", "-show_format"];
    args.push("-i".into());
    args.push(media.as_os_str().to_owned());
    let out = proc::run(&paths.ffprobe, &args, None, &CancellationToken::new())
        .await
        .map_err(|mut e| {
            e.stage = "probe".into();
            e
        })?;
    let json: Value = serde_json::from_slice(&out.stdout).map_err(|e| {
        RustlerError::new(ErrorCode::Media, "probe", format!("unparseable ffprobe output: {e}"))
    })?;
    parse(&json).ok_or_else(|| {
        RustlerError::new(
            ErrorCode::Media,
            "probe",
            format!("no audio or video streams in {}", media.display()),
        )
    })
}

fn num(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

fn rational(s: &str) -> Option<f64> {
    let (n, d) = s.split_once('/')?;
    let (n, d): (f64, f64) = (n.parse().ok()?, d.parse().ok()?);
    (d != 0.0).then_some(n / d)
}

fn rotation_of(stream: &Value) -> i32 {
    if let Some(sd) = stream["side_data_list"].as_array() {
        for e in sd {
            if let Some(r) = e.get("rotation").and_then(num) {
                return r.round() as i32;
            }
        }
    }
    stream["tags"]["rotate"].as_str().and_then(|s| s.parse::<f64>().ok()).map_or(0, |r| r.round() as i32)
}

fn parse(j: &Value) -> Option<MediaProbe> {
    let streams = j["streams"].as_array()?;
    // Cover art in audio files shows up as an `attached_pic` video stream: not real video.
    let video = streams.iter().find(|s| {
        s["codec_type"] == "video" && s["disposition"]["attached_pic"].as_i64().unwrap_or(0) == 0
    });
    let audio = streams.iter().find(|s| s["codec_type"] == "audio");
    if video.is_none() && audio.is_none() {
        return None;
    }

    let mut p = MediaProbe {
        has_video: video.is_some(),
        has_audio: audio.is_some(),
        ..MediaProbe::default()
    };
    if let Some(v) = video {
        let w = v["width"].as_u64().unwrap_or(0) as u32;
        let h = v["height"].as_u64().unwrap_or(0) as u32;
        p.rotation = rotation_of(v);
        let swap = p.rotation.rem_euclid(180) == 90;
        (p.width, p.height) = if swap { (h, w) } else { (w, h) };
        p.fps = v["avg_frame_rate"]
            .as_str()
            .and_then(rational)
            .filter(|f| *f > 0.0)
            .or_else(|| v["r_frame_rate"].as_str().and_then(rational))
            .unwrap_or(0.0);
        p.video_codec = v["codec_name"].as_str().map(str::to_string);
    }
    if let Some(a) = audio {
        p.audio_codec = a["codec_name"].as_str().map(str::to_string);
        p.sample_rate = a["sample_rate"].as_str().and_then(|s| s.parse().ok());
        p.channels = a["channels"].as_u64().map(|c| c as u32);
    }
    p.duration = num(&j["format"]["duration"])
        .or_else(|| {
            streams
                .iter()
                .filter_map(|s| num(&s["duration"]))
                .fold(None, |acc: Option<f64>, d| Some(acc.map_or(d, |a| a.max(d))))
        })
        .unwrap_or(0.0);
    Some(p)
}
