//! Terminal renderer for generated map scenes.
//!
//! The GUI paints scenes with egui (`gui/map_view.rs`); this is the same
//! presentation model projected onto character cells: edges drawn as braille
//! lines through a ratatui Canvas, rooms as block glyphs stamped over them,
//! the current room highlighted in the theme accent. The pane keeps a room
//! hit map from the last render so mouse clicks can resolve to a room id for
//! click-to-travel (`.go2 <id>`).

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols::Marker;
use ratatui::widgets::canvas::{Canvas, Line as CanvasLine};
use ratatui::widgets::Widget;
use std::collections::HashSet;

use crate::core::layout_engine::scene::{SceneEdgeKind, Sheet, SheetScene};
use crate::core::map_service::DbState;
use crate::theme::AppTheme;

use super::crossterm_bridge;

/// Zoom bounds in columns per scene cell. Rows per cell are half the columns
/// (terminal characters are ~1:2), so the map keeps a square aspect.
const MIN_ZOOM: u16 = 2;
const MAX_ZOOM: u16 = 12;

pub struct MapPane {
    /// Columns per scene cell.
    zoom: u16,
    /// Room hit areas from the last render, absolute buffer coordinates.
    hits: Vec<(Rect, u32)>,
    /// Content area of the last render (wheel/click routing).
    last_area: Option<Rect>,
}

impl Default for MapPane {
    fn default() -> Self {
        Self {
            zoom: 4,
            hits: Vec::new(),
            last_area: None,
        }
    }
}

impl MapPane {
    pub fn zoom_by(&mut self, delta: i16) {
        self.zoom = (self.zoom as i16 + delta).clamp(MIN_ZOOM as i16, MAX_ZOOM as i16) as u16;
    }

    /// Room under the given absolute buffer position from the last render.
    pub fn room_at(&self, x: u16, y: u16) -> Option<u32> {
        self.hits
            .iter()
            .find(|(rect, _)| {
                x >= rect.x
                    && x < rect.x + rect.width
                    && y >= rect.y
                    && y < rect.y + rect.height
            })
            .map(|&(_, id)| id)
    }

    /// Whether the position falls inside the last-rendered map content area.
    pub fn contains(&self, x: u16, y: u16) -> bool {
        self.last_area.is_some_and(|a| {
            x >= a.x && x < a.x + a.width && y >= a.y && y < a.y + a.height
        })
    }

    pub fn render(
        &mut self,
        app_core: &crate::core::AppCore,
        area: Rect,
        buf: &mut Buffer,
        theme: &AppTheme,
    ) {
        self.hits.clear();
        self.last_area = Some(area);
        if area.width < 4 || area.height < 2 {
            return;
        }

        let map = &app_core.map;
        match map.db_state() {
            DbState::NotLoaded => {
                self.hint(area, buf, theme, "No map data — run .mapdb download");
                return;
            }
            DbState::Loading => {
                self.hint(area, buf, theme, "Loading map database...");
                return;
            }
            DbState::Failed => {
                let msg = map
                    .db_error
                    .clone()
                    .unwrap_or_else(|| "mapdb load failed".to_string());
                self.hint(area, buf, theme, &format!("Map unavailable: {msg}"));
                return;
            }
            DbState::Loaded => {}
        }
        let Some(scene) = map.current_scene() else {
            let msg = if map.current_location.is_some() {
                "Generating map..."
            } else {
                "Waiting for a mapped room..."
            };
            self.hint(area, buf, theme, msg);
            return;
        };

        let current = map.current_room_id;

        // Sheet, camera center, and (indoors) the building's group cluster —
        // same selection logic as the GUI mini map.
        let (sheet_kind, center, group_filter): (
            Sheet,
            crate::core::layout_engine::positioner::Cell,
            Option<HashSet<usize>>,
        ) = match current.and_then(|id| scene.room(id)) {
            Some((sheet, room)) => (
                sheet,
                room.cell,
                (sheet == Sheet::Interiors).then(|| scene.cluster_groups(room.group)),
            ),
            None => {
                let b = &scene.outdoor;
                (
                    Sheet::Outdoor,
                    crate::core::layout_engine::positioner::Cell {
                        x: (b.min.x + b.max.x) / 2,
                        y: (b.min.y + b.max.y) / 2,
                    },
                    None,
                )
            }
        };
        let sheet = scene.sheet(sheet_kind);

        // Cell -> absolute char coordinates. Rows per cell are half the
        // columns so distances read equally in both axes.
        let cw = self.zoom as i32;
        let ch = (self.zoom as i32 / 2).max(1);
        let ox = area.x as i32 + area.width as i32 / 2 - center.x * cw;
        let oy = area.y as i32 + area.height as i32 / 2 - center.y * ch;
        let to_char = |cell: &crate::core::layout_engine::positioner::Cell| -> (i32, i32) {
            (ox + cell.x * cw, oy + cell.y * ch)
        };
        let in_area = |sx: i32, sy: i32| -> bool {
            sx >= area.x as i32
                && sx < (area.x + area.width) as i32
                && sy >= area.y as i32
                && sy < (area.y + area.height) as i32
        };
        let group_visible =
            |group: usize| group_filter.as_ref().map_or(true, |f| f.contains(&group));

        let edge_color = crossterm_bridge::to_ratatui_color(theme.text_secondary);
        let connector_color = crossterm_bridge::to_ratatui_color(theme.text_disabled);
        let room_color = crossterm_bridge::to_ratatui_color(theme.text_primary);
        let entrance_color = Color::Yellow;
        let accent = crossterm_bridge::to_ratatui_color(theme.window_border_focused);

        // Edges first (braille canvas), rooms stamped over them after.
        // Canvas y grows upward; buffer rows grow downward.
        let flip_y = |sy: i32| area.height as f64 - (sy - area.y as i32) as f64 - 0.5;
        let canvas = Canvas::default()
            .marker(Marker::Braille)
            .x_bounds([0.0, area.width as f64])
            .y_bounds([0.0, area.height as f64])
            .paint(|ctx| {
                for edge in &sheet.edges {
                    if edge.kind == SceneEdgeKind::Stub || !group_visible(edge.group) {
                        continue;
                    }
                    let (ax, ay) = to_char(&edge.a);
                    let (bx, by) = to_char(&edge.b);
                    // Draw when either endpoint is visible; clipping is the
                    // canvas's job.
                    if !in_area(ax, ay) && !in_area(bx, by) {
                        continue;
                    }
                    let color = match edge.kind {
                        SceneEdgeKind::Directional => edge_color,
                        _ => connector_color,
                    };
                    ctx.draw(&CanvasLine {
                        x1: (ax - area.x as i32) as f64 + 0.5,
                        y1: flip_y(ay),
                        x2: (bx - area.x as i32) as f64 + 0.5,
                        y2: flip_y(by),
                        color,
                    });
                }
            });
        canvas.render(area, buf);

        // Rooms.
        for room in &sheet.rooms {
            if !group_visible(room.group) {
                continue;
            }
            let (sx, sy) = to_char(&room.cell);
            if !in_area(sx, sy) {
                continue;
            }
            let is_current = current == Some(room.id);
            let (glyph, style) = if is_current {
                (
                    '◉',
                    Style::default().fg(accent).add_modifier(Modifier::BOLD),
                )
            } else if room.entrance {
                ('⌂', Style::default().fg(entrance_color))
            } else {
                ('■', Style::default().fg(room_color))
            };
            buf[(sx as u16, sy as u16)].set_char(glyph).set_style(style);
            // Generous hit box: the glyph plus one column either side.
            self.hits.push((
                Rect {
                    x: (sx - 1).max(area.x as i32) as u16,
                    y: sy as u16,
                    width: 3.min((area.x + area.width) as i32 - (sx - 1).max(area.x as i32)) as u16,
                    height: 1,
                },
                room.id,
            ));
        }

        // Status line: current room title (or hovered target hint) in the
        // bottom-left corner of the pane.
        let status = current
            .and_then(|id| scene.room(id))
            .map(|(_, room)| format!(" {} [{}] ", room.title, room.id));
        if let Some(status) = status {
            let y = area.y + area.height - 1;
            let style = Style::default()
                .fg(crossterm_bridge::to_ratatui_color(theme.text_secondary));
            for (i, ch) in status.chars().enumerate() {
                let x = area.x + i as u16;
                if x >= area.x + area.width {
                    break;
                }
                buf[(x, y)].set_char(ch).set_style(style);
            }
        }
    }

    fn hint(&self, area: Rect, buf: &mut Buffer, theme: &AppTheme, text: &str) {
        let style = Style::default()
            .fg(crossterm_bridge::to_ratatui_color(theme.text_secondary));
        let y = area.y + area.height / 2;
        let start = area.x + area.width.saturating_sub(text.len() as u16) / 2;
        for (i, ch) in text.chars().enumerate() {
            let x = start + i as u16;
            if x >= area.x + area.width {
                break;
            }
            buf[(x, y)].set_char(ch).set_style(style);
        }
    }
}
