use crate::fold::preview::model::Document;
use lofty::{
    file::{AudioFile, TaggedFileExt},
    tag::Accessor,
};
use std::{fs::File, path::Path};
pub fn read(path: &Path) -> anyhow::Result<Document> {
    let ext = path
        .extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_ascii_lowercase();
    if matches!(ext.as_str(), "mkv" | "webm") {
        return matroska(path);
    }
    if matches!(ext.as_str(), "mp4" | "mov" | "m4v") {
        return mp4(path);
    }
    let f = lofty::probe::Probe::open(path)?
        .guess_file_type()?
        .options(lofty::config::ParseOptions::new().read_cover_art(false))
        .read()?;
    let mut d = Document::new("Audio");
    for tag in f.tags() {
        for (key, value) in [
            ("Title", tag.title()),
            ("Artist", tag.artist()),
            ("Album", tag.album()),
            ("Genre", tag.genre()),
        ] {
            if let Some(v) = value {
                d.field(key, v);
            }
        }
        if let Some(v) = tag.track() {
            d.field("Track", v);
        }
        if let Some(v) = tag.disk() {
            d.field("Disc", v);
        }
        if let Some(v) = tag.year() {
            d.field("Year", v);
        }
        for item in tag.items() {
            if let Some(v) = item.value().text() {
                let key = format!("{:?}", item.key());
                if !d.fields.iter().any(|f| f.value == v) {
                    d.field(key, v);
                }
            }
        }
    }
    let p = f.properties();
    d.field("Duration", duration(p.duration().as_secs_f64()));
    if let Some(v) = p.audio_bitrate() {
        d.field("Bitrate", format!("{v} kb/s"));
    }
    if let Some(v) = p.sample_rate() {
        d.field("Sample rate", format!("{v} Hz"));
    }
    if let Some(v) = p.channels() {
        d.field("Channels", v);
    }
    Ok(d)
}
fn duration(s: f64) -> String {
    if !s.is_finite() || s < 0.0 {
        return "unknown".into();
    }
    let s = s as u64;
    format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
}
fn matroska(path: &Path) -> anyhow::Result<Document> {
    let f = matroska_demuxer::MatroskaFile::open(File::open(path)?)?;
    let mut d = Document::new("Matroska / WebM");
    if let Some(t) = f.info().title() {
        d.field("Title", t);
    }
    if let Some(t) = f.info().duration() {
        d.field(
            "Duration",
            duration(t * f.info().timestamp_scale().get() as f64 / 1e9),
        );
    }
    for track in f.tracks().iter().take(32) {
        d.field(
            format!("Track {}", track.track_number()),
            format!("{:?} · {}", track.track_type(), track.codec_id()),
        );
        if let Some(l) = track.language_bcp47().or(track.language()) {
            d.field("Language", l);
        }
        if let Some(v) = track.video() {
            d.field(
                "Dimensions",
                format!("{} × {}", v.pixel_width(), v.pixel_height()),
            );
        }
        if let Some(a) = track.audio() {
            d.field(
                "Audio",
                format!("{} Hz · {} channels", a.sampling_frequency(), a.channels()),
            );
        }
    }
    if let Some(tags) = f.tags() {
        for tag in tags.iter().take(128) {
            for t in tag.simple_tags().iter().take(128) {
                if let Some(s) = t.string() {
                    d.field(t.name(), s);
                }
            }
        }
    }
    Ok(d)
}
fn mp4(path: &Path) -> anyhow::Result<Document> {
    // mp4parse reads mdat linearly. Feed only bounded metadata boxes, seeking
    // over media payloads so a multi-GB movie does not cost a multi-GB read.
    use std::io::{Read, Seek, SeekFrom};
    let mut f = File::open(path)?;
    let len = f.metadata()?.len();
    let mut boxes = vec![];
    while f.stream_position()? < len {
        let start = f.stream_position()?;
        let mut h = [0; 8];
        f.read_exact(&mut h)?;
        let mut size = u32::from_be_bytes(h[..4].try_into()?) as u64;
        let mut header = h.to_vec();
        if size == 1 {
            let mut wide = [0; 8];
            f.read_exact(&mut wide)?;
            size = u64::from_be_bytes(wide);
            header.extend(wide);
        }
        if size == 0 {
            size = len - start;
        }
        anyhow::ensure!(
            size >= header.len() as u64 && size <= len - start,
            "Invalid MP4 box size"
        );
        if &h[4..] == b"moov" || &h[4..] == b"ftyp" {
            anyhow::ensure!(
                size <= 32 * 1024 * 1024 && boxes.len() as u64 + size <= 32 * 1024 * 1024,
                "MP4 metadata exceeds preview limit"
            );
            boxes.extend_from_slice(&header);
            f.by_ref()
                .take(size - header.len() as u64)
                .read_to_end(&mut boxes)?;
        } else {
            f.seek(SeekFrom::Start(start + size))?;
        }
    }
    let c = mp4parse::read_mp4(&mut &boxes[..])?;
    let mut d = Document::new("MP4 / QuickTime");
    if let Some(Ok(u)) = c.userdata {
        if let Some(m) = u.meta {
            for (k, v) in [
                ("Title", m.title),
                ("Artist", m.artist),
                ("Album", m.album),
                ("Year", m.year),
                ("Comment", m.comment),
            ] {
                if let Some(v) = v {
                    d.field(k, String::from_utf8_lossy(&v));
                }
            }
        }
    }
    for t in c.tracks.iter().take(32) {
        d.field("Track", format!("{:?}", t.track_type));
        if let (Some(n), Some(scale)) = (t.duration, t.timescale) {
            if scale.0 > 0 {
                d.field("Duration", duration(n.0 as f64 / scale.0 as f64));
            }
        }
        if let Some(h) = &t.tkhd {
            if h.width > 0 {
                d.field(
                    "Dimensions",
                    format!("{} × {}", h.width >> 16, h.height >> 16),
                );
            }
        }
        if let Some(s) = &t.stsd {
            for entry in s.descriptions.iter().take(8) {
                match entry {
                    mp4parse::SampleEntry::Audio(a) => {
                        d.field("Codec", format!("{:?}", a.codec_type));
                        d.field("Channels", a.channelcount);
                        d.field("Sample rate", a.samplerate);
                    }
                    mp4parse::SampleEntry::Video(v) => {
                        d.field("Codec", format!("{:?}", v.codec_type))
                    }
                    _ => {}
                }
            }
        }
    }
    Ok(d)
}
