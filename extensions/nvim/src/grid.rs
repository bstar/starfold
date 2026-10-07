use anyhow::{ensure, Result};
use rmpv::Value;
use starfold_preview_protocol::{Surface, Viewport};
use starkit::native_surface::{PixelRect, Primitive};
use std::collections::BTreeMap;
#[derive(Clone)]
struct Cell {
    text: String,
    highlight: u64,
}
impl Default for Cell {
    fn default() -> Self {
        Self {
            text: " ".into(),
            highlight: 0,
        }
    }
}
#[derive(Default)]
pub struct Grid {
    pub columns: usize,
    pub rows: usize,
    cells: Vec<Cell>,
    highlights: BTreeMap<u64, Value>,
    cursor: (usize, usize),
    foreground: Option<String>,
    background: Option<String>,
    pub modified: bool,
    pub mode: String,
    pub blocking: bool,
}
pub fn map_get<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    v.as_map()?
        .iter()
        .find(|(k, _)| k.as_str() == Some(key))
        .map(|(_, v)| v)
}
fn number(a: &[Value], index: usize) -> usize {
    a.get(index).and_then(Value::as_u64).unwrap_or(0) as usize
}
impl Grid {
    pub fn notification(&mut self, value: &Value) -> Result<()> {
        let Some(a) = value.as_array() else {
            return Ok(());
        };
        if a.first().and_then(Value::as_u64) != Some(2) {
            return Ok(());
        }
        if a.get(1).and_then(Value::as_str) == Some("starfold_editor") {
            self.modified = a
                .get(2)
                .and_then(Value::as_array)
                .and_then(|a| a.first())
                .and_then(Value::as_bool)
                .unwrap_or(self.modified);
            return Ok(());
        }
        if a.get(1).and_then(Value::as_str) != Some("redraw") {
            return Ok(());
        }
        for event in a.get(2).and_then(Value::as_array).into_iter().flatten() {
            let Some(event) = event.as_array() else {
                continue;
            };
            let Some(name) = event.first().and_then(Value::as_str) else {
                continue;
            };
            for args in &event[1..] {
                if let Some(args) = args.as_array() {
                    self.event(name, args)?;
                }
            }
        }
        Ok(())
    }
    fn event(&mut self, name: &str, a: &[Value]) -> Result<()> {
        match name {
            "default_colors_set" => {
                self.foreground = a
                    .first()
                    .and_then(Value::as_u64)
                    .map(|v| format!("#{:06x}", v & 0xffffff));
                self.background = a
                    .get(1)
                    .and_then(Value::as_u64)
                    .map(|v| format!("#{:06x}", v & 0xffffff));
            }

            "grid_resize" if number(a, 0) == 1 => {
                let (cols, rows) = (number(a, 1), number(a, 2));
                ensure!(
                    cols > 0 && rows > 0 && cols <= 256 && rows <= 113 && cols * rows <= 8192,
                    "Neovim grid exceeds limits"
                );
                self.columns = cols;
                self.rows = rows;
                self.cells = vec![Cell::default(); cols * rows];
            }
            "grid_clear" if number(a, 0) == 1 => self.cells.fill(Cell::default()),
            "grid_cursor_goto" if number(a, 0) == 1 => self.cursor = (number(a, 1), number(a, 2)),
            "hl_attr_define" => {
                ensure!(self.highlights.len() < 4096, "Too many Neovim highlights");
                if let Some(v) = a.get(1) {
                    self.highlights.insert(number(a, 0) as u64, v.clone());
                }
            }
            "mode_change" => {
                self.mode = a.first().and_then(Value::as_str).unwrap_or("normal").into()
            }
            "grid_line" if number(a, 0) == 1 => {
                let row = number(a, 1);
                let mut col = number(a, 2);
                let mut highlight = 0;
                ensure!(
                    row < self.rows && col < self.columns,
                    "Neovim line outside grid"
                );
                if let Some(cells) = a.get(3).and_then(Value::as_array) {
                    for cell in cells {
                        let Some(cell) = cell.as_array() else {
                            continue;
                        };
                        let text = cell.first().and_then(Value::as_str).unwrap_or("");
                        ensure!(text.len() <= 128, "Neovim cell too large");
                        if let Some(id) = cell.get(1).and_then(Value::as_u64) {
                            highlight = id;
                        }
                        let repeat = cell.get(2).and_then(Value::as_u64).unwrap_or(1) as usize;
                        ensure!(
                            repeat <= self.columns && col + repeat <= self.columns,
                            "Neovim repeated cell outside grid"
                        );
                        for _ in 0..repeat {
                            self.cells[row * self.columns + col] = Cell {
                                text: text.into(),
                                highlight,
                            };
                            col += 1;
                        }
                    }
                }
            }
            "grid_scroll" if number(a, 0) == 1 => {
                let (top, bottom, left, right) =
                    (number(a, 1), number(a, 2), number(a, 3), number(a, 4));
                ensure!(
                    top <= bottom && bottom <= self.rows && left <= right && right <= self.columns,
                    "Neovim scroll outside grid"
                );
                let dr = a.get(5).and_then(Value::as_i64).unwrap_or(0);
                let dc = a.get(6).and_then(Value::as_i64).unwrap_or(0);
                let old = self.cells.clone();
                for row in top..bottom {
                    for col in left..right {
                        let sr = row as i64 + dr;
                        let sc = col as i64 + dc;
                        self.cells[row * self.columns + col] = if sr >= top as i64
                            && sr < bottom as i64
                            && sc >= left as i64
                            && sc < right as i64
                        {
                            old[sr as usize * self.columns + sc as usize].clone()
                        } else {
                            Cell::default()
                        };
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
    pub fn text(&self) -> String {
        self.cells
            .chunks(self.columns.max(1))
            .map(|row| {
                row.iter()
                    .map(|c| c.text.as_str())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
    pub fn cell_grid(&self, viewport: &Viewport) -> Result<starfold_preview_protocol::CellGrid> {
        use starfold_preview_protocol::{CellGrid, StyledCell};
        let rgb = |attrs: Option<&Value>, key: &str, fallback: &str| {
            let n = attrs
                .and_then(|a| map_get(a, key))
                .and_then(Value::as_u64)
                .unwrap_or_else(|| {
                    u64::from_str_radix(fallback.trim_start_matches('#'), 16).unwrap_or(0)
                });
            [(n >> 16) as u8, (n >> 8) as u8, n as u8]
        };
        let fg = self.foreground.as_deref().unwrap_or(&viewport.foreground);
        let bg = self.background.as_deref().unwrap_or(&viewport.background);
        let cells = self
            .cells
            .iter()
            .map(|c| {
                let attrs = self.highlights.get(&c.highlight);
                let flag = |name| {
                    attrs
                        .and_then(|a| map_get(a, name))
                        .and_then(Value::as_bool)
                        == Some(true)
                };
                let mut foreground = rgb(attrs, "foreground", fg);
                let mut background = rgb(attrs, "background", bg);
                if flag("reverse") {
                    std::mem::swap(&mut foreground, &mut background);
                }
                StyledCell {
                    symbol: c.text.clone(),
                    foreground,
                    background,
                    modifiers: u8::from(flag("bold"))
                        | u8::from(flag("italic")) << 1
                        | u8::from(flag("underline") || flag("undercurl")) << 2
                        | u8::from(flag("strikethrough")) << 3,
                }
            })
            .collect();
        let grid = CellGrid {
            columns: self.columns as u16,
            rows: self.rows as u16,
            cells,
            cursor: (self.cursor.0 < self.rows && self.cursor.1 < self.columns)
                .then_some([self.cursor.1 as u16, self.cursor.0 as u16]),
        };
        grid.validate()?;
        Ok(grid)
    }
    pub fn surface(&self, viewport: &Viewport) -> Result<Surface> {
        let width = viewport.width.clamp(1, 2048) as u16;
        let height = viewport.height.clamp(1, 2048) as u16;
        let foreground = self.foreground.as_deref().unwrap_or(&viewport.foreground);
        let background = self.background.as_deref().unwrap_or(&viewport.background);
        let mut s = Surface::new(width, height, background.into());
        s.cell_size = Some([8, 18]);
        let color = |attrs: Option<&Value>, key: &str, default: &str| {
            attrs
                .and_then(|v| map_get(v, key))
                .and_then(Value::as_u64)
                .map(|v| format!("#{:06x}", v & 0xffffff))
                .unwrap_or_else(|| default.into())
        };
        for row in 0..self.rows {
            if row * 18 >= usize::from(height) {
                break;
            }
            let mut col = 0;
            while col < self.columns {
                let cell = &self.cells[row * self.columns + col];
                let start = col;
                let mut text = String::new();
                while col < self.columns
                    && self.cells[row * self.columns + col].highlight == cell.highlight
                {
                    let c = &self.cells[row * self.columns + col];
                    text.push_str(&c.text);
                    col += 1;
                }
                let attrs = self.highlights.get(&cell.highlight);
                let mut fg = color(attrs, "foreground", foreground);
                let mut bg = color(attrs, "background", background);
                if attrs
                    .and_then(|v| map_get(v, "reverse"))
                    .and_then(Value::as_bool)
                    == Some(true)
                {
                    std::mem::swap(&mut fg, &mut bg);
                }
                if start * 8 >= usize::from(width) {
                    break;
                }
                let x = (start * 8) as u16;
                let right = (col * 8).min(usize::from(width)) as u16;
                let y = (row * 18) as u16;
                let bottom = (((row + 1) * 18).min(usize::from(height))) as u16;
                let rect = PixelRect::new(x, y, right - x, bottom - y);
                if bg != background {
                    s.fill(rect, &bg, 0);
                }
                if !text.is_empty() {
                    s.text(
                        rect,
                        text,
                        &fg,
                        13,
                        attrs
                            .and_then(|v| map_get(v, "bold"))
                            .and_then(Value::as_bool)
                            == Some(true),
                    );
                    if let Some(Primitive::Text { mono, .. }) = s.nodes.last_mut() {
                        *mono = true;
                    }
                }
            }
        }
        let (row, col) = self.cursor;
        if row < self.rows
            && col < self.columns
            && row * 18 < usize::from(height)
            && col * 8 < usize::from(width)
        {
            let x = (col * 8) as u16;
            let right = (((col + 1) * 8).min(usize::from(width))) as u16;
            let y = (row * 18) as u16;
            let bottom = (((row + 1) * 18).min(usize::from(height))) as u16;
            s.nodes.push(Primitive::Border {
                rect: PixelRect::new(x, y, right - x, bottom - y),
                color: foreground.into(),
                radius: 0,
            });
        }
        s.validate()?;
        Ok(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn redraw(name: &str, args: Vec<Value>) -> Value {
        Value::Array(vec![
            2.into(),
            "redraw".into(),
            Value::Array(vec![Value::Array(vec![name.into(), Value::Array(args)])]),
        ])
    }
    #[test]
    fn repeated_cells_wide_text_and_scroll_remain_bounded() {
        let mut g = Grid::default();
        g.notification(&redraw("grid_resize", vec![1.into(), 8.into(), 3.into()]))
            .unwrap();
        g.notification(&redraw(
            "grid_line",
            vec![
                1.into(),
                0.into(),
                0.into(),
                Value::Array(vec![
                    Value::Array(vec!["界".into(), 0.into()]),
                    Value::Array(vec!["".into()]),
                    Value::Array(vec!["x".into(), 0.into(), 3.into()]),
                ]),
            ],
        ))
        .unwrap();
        assert!(g.text().starts_with("界xxx"));
        g.surface(&Viewport::default()).unwrap().validate().unwrap();
        assert!(g
            .notification(&redraw(
                "grid_line",
                vec![1.into(), 9.into(), 0.into(), Value::Array(vec![])]
            ))
            .is_err());
    }
    #[test]
    fn wide_editor_keeps_cell_size_and_configured_colors_on_resize() {
        let mut g = Grid::default();
        g.notification(&redraw(
            "grid_resize",
            vec![1.into(), 160.into(), 25.into()],
        ))
        .unwrap();
        g.notification(&redraw(
            "default_colors_set",
            vec![0xabcdefu64.into(), 0x123456u64.into()],
        ))
        .unwrap();
        g.notification(&redraw(
            "grid_line",
            vec![
                1.into(),
                0.into(),
                140.into(),
                Value::Array(vec![Value::Array(vec!["hello".into(), 1.into()])]),
            ],
        ))
        .unwrap();
        let viewport = Viewport {
            width: 1440,
            height: 500,
            ..Default::default()
        };
        let surface = g.surface(&viewport).unwrap();
        assert_eq!(surface.background, "#123456");
        assert_eq!(surface.cell_size, Some([8, 18]));
        let text = surface
            .nodes
            .iter()
            .find_map(|node| match node {
                Primitive::Text {
                    text,
                    rect,
                    size,
                    color,
                    ..
                } if text == "hello" => Some((rect, size, color)),
                _ => None,
            })
            .unwrap();
        assert_eq!(text.0.x, 140 * 8);
        assert_eq!(*text.1, 13);
        assert_eq!(text.2, "#abcdef");
        // Old grid events may still be queued when the viewport shrinks.
        g.surface(&Viewport {
            width: 480,
            height: 240,
            ..viewport
        })
        .unwrap()
        .validate()
        .unwrap();
    }
    proptest::proptest! {
        #[test]
        fn foreign_grid_updates_do_not_escape_surface(row in 0usize..80,col in 0usize..160,repeat in 0usize..200,text in ".{0,20}") {
            let mut g=Grid::default();g.notification(&redraw("grid_resize",vec![1.into(),60.into(),20.into()])).unwrap();
            let _=g.notification(&redraw("grid_line",vec![1.into(),(row as u64).into(),(col as u64).into(),Value::Array(vec![Value::Array(vec![text.into(),0.into(),(repeat as u64).into()])])]));
            g.surface(&Viewport::default()).unwrap().validate().unwrap();
        }
    }
}
