//! Preview executable protocol. No terminal escapes or dynamically loaded code.
use anyhow::{ensure, Result};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
pub use starkit::native_surface::Surface;
use std::io::{Read, Write};

pub const VERSION: u32 = 1;
pub const MAX_CONTROL: usize = 1024 * 1024;
pub const MAX_BINARY: usize = 32 * 1024 * 1024;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Limits {
    pub text_bytes: usize,
    pub cache_bytes: usize,
    pub image_dimension: u32,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Viewport {
    pub width: u32,
    pub height: u32,
    pub foreground: String,
    pub background: String,
    /// Exact cell grid for a cell presentation; omitted by older hosts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cells: Option<[u16; 2]>,
}
impl Default for Viewport {
    fn default() -> Self {
        Self {
            cells: None,
            width: 800,
            height: 600,
            foreground: "#ffffff".into(),
            background: "#000000".into(),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Input {
    Key {
        key: String,
    },
    Paste {
        text: String,
    },
    Action {
        action: String,
    },
    Pointer {
        action: String,
        x: u32,
        y: u32,
        button: u8,
    },
    Viewport {
        viewport: Viewport,
    },
    Page {
        page: u32,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    Hello {
        version: u32,
        capabilities: Vec<String>,
    },
    Open {
        path: Vec<u8>,
        limits: Limits,
        viewport: Viewport,
    },
    Input {
        input: Input,
    },
    Cancel,
    Close,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Envelope<T> {
    pub session: u64,
    pub generation: u64,
    pub sequence: u64,
    pub message: T,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextPage {
    pub number: u32,
    pub text: String,
    pub truncated: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Raster {
    pub width: u32,
    pub height: u32,
    pub page: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Media {
    pub duration: f64,
    pub width: u32,
    pub height: u32,
    pub audio: bool,
}
/// Host media services retain transport and platform ownership.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaAction {
    PlayPause,
    SeekForward,
    SeekBackward,
    VolumeUp,
    VolumeDown,
    Mute,
    Fullscreen,
    WindowFullscreen,
    Expand,
    AudioTracks,
    Subtitles,
    StreamMode,
    ExitFullscreen,
    Stop,
}
/// A bounded editor grid. Empty symbols are wide-character continuation cells.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StyledCell {
    pub symbol: String,
    pub foreground: [u8; 3],
    pub background: [u8; 3],
    /// Bold, italic, underline and strikethrough, respectively.
    pub modifiers: u8,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CellGrid {
    pub columns: u16,
    pub rows: u16,
    pub cells: Vec<StyledCell>,
    pub cursor: Option<[u16; 2]>,
}
impl CellGrid {
    pub fn validate(&self) -> Result<()> {
        let count = usize::from(self.columns) * usize::from(self.rows);
        ensure!(
            self.columns > 0 && self.rows > 0 && count <= 8192 && self.cells.len() == count,
            "invalid cell grid dimensions"
        );
        ensure!(
            self.cells.iter().all(|c| c.symbol.len() <= 128
                && !c.symbol.chars().any(|c| c.is_control()
                    || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'))
                && c.modifiers & !15 == 0
                && starkit::wrap::width_of(&c.symbol) <= 2),
            "invalid styled cell"
        );
        for (index, cell) in self.cells.iter().enumerate() {
            if starkit::wrap::width_of(&cell.symbol) == 2 {
                ensure!(
                    index % usize::from(self.columns) + 1 < usize::from(self.columns)
                        && self.cells[index + 1].symbol.is_empty(),
                    "wide cell has no continuation"
                );
            }
        }
        ensure!(
            self.cursor
                .is_none_or(|[x, y]| x < self.columns && y < self.rows),
            "cursor outside cell grid"
        );
        Ok(())
    }
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Presentation {
    pub kind: String,
    pub fields: Vec<(String, String)>,
    pub pages: Vec<TextPage>,
    pub total_pages: Option<u32>,
    pub next_page: Option<u32>,
    pub notice: Option<String>,
    /// Exactly one RGBA8 attachment when present.
    pub raster: Option<Raster>,
    /// A cell-compatible explanation must accompany a native surface.
    pub surface: Option<Surface>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cells: Option<CellGrid>,
    pub media: Option<Media>,
    pub actions: Vec<MediaAction>,
    /// Keys consumed by the extension while Preview has focus.
    pub keys: Vec<String>,
    /// An editor session pins Preview and owns focused keyboard input.
    #[serde(default)]
    pub interactive: bool,
    #[serde(default)]
    pub modified: bool,
}
impl Presentation {
    pub fn validate(&self, bytes: &[u8]) -> Result<()> {
        ensure!(
            self.fields.len() <= 128
                && self.pages.len() <= 24
                && self.keys.len() <= 64
                && self.actions.len() <= 16,
            "presentation exceeds limits"
        );
        if let Some(r) = &self.raster {
            ensure!(
                r.width > 0 && r.height > 0 && r.width <= 4096 && r.height <= 4096,
                "invalid raster dimensions"
            );
            let size = u64::from(r.width) * u64::from(r.height) * 4;
            ensure!(
                size <= MAX_BINARY as u64 && size == bytes.len() as u64,
                "invalid raster attachment"
            );
        } else {
            ensure!(bytes.is_empty(), "unexpected binary attachment");
        }
        if let Some(cells) = &self.cells {
            cells.validate()?;
        }
        if let Some(surface) = &self.surface {
            ensure!(
                !self.pages.is_empty() || !self.fields.is_empty() || self.notice.is_some(),
                "Native surface requires cell presentation"
            );
            surface.validate()?;
        }
        if let Some(m) = &self.media {
            ensure!(
                m.duration.is_finite()
                    && m.duration >= 0.0
                    && m.width > 0
                    && m.height > 0
                    && m.width <= 32768
                    && m.height <= 32768,
                "invalid media descriptor"
            );
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Reply {
    Hello {
        version: u32,
        provider: String,
        revision: String,
        capabilities: Vec<String>,
    },
    Content {
        presentation: Box<Presentation>,
    },
    Closed,
    Error {
        message: String,
    },
}
/// An eight-byte big-endian header precedes JSON and the optional raw attachment.
pub fn write_frame<T: Serialize>(out: &mut impl Write, value: &T, bytes: &[u8]) -> Result<()> {
    let json = serde_json::to_vec(value)?;
    ensure!(
        json.len() <= MAX_CONTROL && bytes.len() <= MAX_BINARY,
        "frame exceeds limits"
    );
    out.write_all(&(json.len() as u32).to_be_bytes())?;
    out.write_all(&(bytes.len() as u32).to_be_bytes())?;
    out.write_all(&json)?;
    out.write_all(bytes)?;
    out.flush()?;
    Ok(())
}
pub fn read_frame<T: DeserializeOwned>(input: &mut impl Read) -> Result<Option<(T, Vec<u8>)>> {
    let mut header = [0; 8];
    match input.read(&mut header[..1])? {
        0 => return Ok(None),
        _ => input.read_exact(&mut header[1..])?,
    }
    let control = u32::from_be_bytes(header[..4].try_into()?) as usize;
    let binary = u32::from_be_bytes(header[4..].try_into()?) as usize;
    ensure!(
        control <= MAX_CONTROL && binary <= MAX_BINARY,
        "frame exceeds limits"
    );
    let mut json = vec![0; control];
    input.read_exact(&mut json)?;
    let value = serde_json::from_slice(&json)?;
    let mut bytes = vec![0; binary];
    input.read_exact(&mut bytes)?;
    Ok(Some((value, bytes)))
}
/// A synchronous helper can rely on the host killing it for cancellation/deadlines.
pub fn serve(
    provider: &str,
    revision: &str,
    capabilities: &[&str],
    mut handle: impl FnMut(Request) -> Result<(Presentation, Vec<u8>)>,
) -> Result<()> {
    let mut input = std::io::stdin().lock();
    let mut output = std::io::stdout().lock();
    let mut negotiated = false;
    let mut session = None;
    while let Some((request, bytes)) = read_frame::<Envelope<Request>>(&mut input)? {
        ensure!(bytes.is_empty(), "requests cannot contain attachments");
        let mut attachment = vec![];
        let closing = matches!(request.message, Request::Close | Request::Cancel);
        let result = match request.message {
            Request::Hello {
                version,
                capabilities: host_capabilities,
            } => {
                ensure!(
                    !capabilities.contains(&"editor")
                        || host_capabilities.iter().any(|c| c == "editor"),
                    "Host does not support protected editor sessions"
                );
                ensure!(
                    !negotiated && version == VERSION,
                    "unsupported protocol version"
                );
                negotiated = true;
                session = Some(request.session);
                Reply::Hello {
                    version: VERSION,
                    provider: provider.into(),
                    revision: revision.into(),
                    capabilities: capabilities.iter().map(|s| (*s).into()).collect(),
                }
            }
            Request::Cancel | Request::Close => Reply::Closed,
            message => {
                ensure!(
                    negotiated && session == Some(request.session),
                    "handshake required"
                );
                match handle(message) {
                    Ok((presentation, bytes)) => {
                        presentation.validate(&bytes)?;
                        attachment = bytes;
                        Reply::Content {
                            presentation: Box::new(presentation),
                        }
                    }
                    Err(e) => Reply::Error {
                        message: e.to_string(),
                    },
                }
            }
        };
        write_frame(
            &mut output,
            &Envelope {
                session: request.session,
                generation: request.generation,
                sequence: request.sequence,
                message: result,
            },
            &attachment,
        )?;
        if closing {
            break;
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    #[test]
    fn cell_grids_validate_dimensions_cursor_and_foreign_symbols() {
        let mut grid = CellGrid {
            columns: 2,
            rows: 1,
            cursor: Some([1, 0]),
            cells: vec![
                StyledCell {
                    symbol: "x".into(),
                    foreground: [1; 3],
                    background: [2; 3],
                    modifiers: 3
                };
                2
            ],
        };
        grid.validate().unwrap();
        grid.cursor = Some([2, 0]);
        assert!(grid.validate().is_err());
        grid.cursor = None;
        grid.cells[0].symbol = "\x1b[2J".into();
        assert!(grid.validate().is_err());
        grid.cells[0].symbol = "x".into();
        grid.cells.pop();
        assert!(grid.validate().is_err());
        let old: Viewport = serde_json::from_str(
            r##"{"width":80,"height":40,"foreground":"#fff","background":"#000"}"##,
        )
        .unwrap();
        assert_eq!(old.cells, None);
    }
    proptest::proptest! {
        #[test]
        fn untrusted_cell_grid_validation_is_bounded(cols in 0u16..300, rows in 0u16..140, symbol in ".{0,140}") {
            let grid = CellGrid { columns:cols,rows,cursor:None,cells:vec![StyledCell {symbol,foreground:[0;3],background:[0;3],modifiers:0}] };
            let _ = grid.validate();
        }
    }
    #[test]
    fn rejects_oversized_header_before_allocation() {
        let bytes = [u32::MAX.to_be_bytes(), 0u32.to_be_bytes()].concat();
        assert!(read_frame::<Request>(&mut &bytes[..]).is_err());
    }
    #[test]
    fn validates_raster_and_surface() {
        let p = Presentation {
            raster: Some(Raster {
                width: 2,
                height: 3,
                page: 1,
            }),
            ..Default::default()
        };
        assert!(p.validate(&[0; 24]).is_ok());
        assert!(p.validate(&[0; 23]).is_err());
    }
    proptest! {
        #[test] fn arbitrary_frames_never_panic(bytes in prop::collection::vec(any::<u8>(), 0..2048)) { let _ = read_frame::<Envelope<Reply>>(&mut &bytes[..]); }
        #[test] fn binary_roundtrip(bytes in prop::collection::vec(any::<u8>(), 0..4096), key in ".{0,80}") {
            let value = Envelope { session: 1, generation: 2, sequence: 3, message: Request::Input { input: Input::Key { key } } };
            let mut frame = vec![]; write_frame(&mut frame, &value, &bytes).unwrap();
            let (got, binary) = read_frame::<Envelope<Request>>(&mut &frame[..]).unwrap().unwrap();
            prop_assert_eq!(got.sequence, 3); prop_assert_eq!(binary, bytes);
        }
    }
}
