use anyhow::{ensure, Result};
use starfold_preview_protocol::{Input, Media, MediaAction, Presentation, Raster, Request};
use std::{
    os::unix::ffi::OsStringExt,
    path::PathBuf,
    sync::{atomic::AtomicBool, Arc},
};
fn main() -> Result<()> {
    let mut presentation = None;
    let mut pixels = vec![];
    starfold_preview_protocol::serve(
        "video",
        env!("CARGO_PKG_VERSION"),
        &["media", "raster", "input"],
        |request| {
            let mut actions = vec![];
            match request {
                Request::Open { path, .. } => {
                    let path = PathBuf::from(std::ffi::OsString::from_vec(path));
                    ensure!(
                        std::fs::metadata(&path)?.is_file(),
                        "Video source is not a regular file"
                    );
                    let poster = starkit::media::poster(&path, Arc::new(AtomicBool::new(false)))?;
                    pixels = poster.pixels.as_raw().clone();
                    presentation = Some(Presentation {
                        kind: "Video".into(),
                        notice: Some("Playback requires a graphical frontend".into()),
                        media: Some(Media {
                            duration: poster.duration,
                            width: poster.width,
                            height: poster.height,
                            audio: poster.audio,
                        }),
                        raster: Some(Raster {
                            width: poster.pixels.width(),
                            height: poster.pixels.height(),
                            page: 0,
                        }),
                        keys: [
                            "p", "P", "space", "left", "right", "+", "=", "-", "m", "M", "f", "F",
                            "e", "E", "a", "A", "s", "S", "o", "O",
                        ]
                        .map(str::to_string)
                        .to_vec(),
                        ..Default::default()
                    });
                }
                Request::Input { input } => {
                    let key = match input {
                        Input::Key { key } | Input::Action { action: key } => key,
                        _ => String::new(),
                    };
                    let action = match key.as_str() {
                        "p" | "P" | "space" | "play_pause" => Some(MediaAction::PlayPause),
                        "left" | "seek_backward" => Some(MediaAction::SeekBackward),
                        "right" | "seek_forward" => Some(MediaAction::SeekForward),
                        "+" | "=" | "volume_up" => Some(MediaAction::VolumeUp),
                        "-" | "volume_down" => Some(MediaAction::VolumeDown),
                        "m" | "M" | "mute" => Some(MediaAction::Mute),
                        "f" | "fullscreen" => Some(MediaAction::Fullscreen),
                        "F" | "window_fullscreen" => Some(MediaAction::WindowFullscreen),
                        "e" | "E" | "expand" => Some(MediaAction::Expand),
                        "a" | "A" | "audio_tracks" => Some(MediaAction::AudioTracks),
                        "s" | "S" | "subtitles" => Some(MediaAction::Subtitles),
                        "o" | "O" | "stream_mode" => Some(MediaAction::StreamMode),
                        "exit_fullscreen" => Some(MediaAction::ExitFullscreen),
                        "stop" => Some(MediaAction::Stop),
                        _ => None,
                    };
                    if let Some(action) = action {
                        actions.push(action);
                    }
                }
                _ => anyhow::bail!("Unsupported request"),
            }
            let mut p = presentation
                .clone()
                .ok_or_else(|| anyhow::anyhow!("No video session"))?;
            p.actions = actions;
            Ok((p, pixels.clone()))
        },
    )
}
