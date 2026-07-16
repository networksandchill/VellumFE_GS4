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

use crate::core::layout_engine::scene::{SceneEdgeKind, Sheet};
use crate::core::map_service::DbState;
use crate::theme::AppTheme;

use super::crossterm_bridge;

/// Zoom bounds in columns per scene cell. Rows per cell are half the columns
/// (terminal characters are ~1:2), so the map keeps a square aspect.
const MIN_ZOOM: u16 = 2;
const MAX_ZOOM: u16 = 12;

/// Room glyphs for the smallest zoom tier (one character per room).
const GLYPH_ROOM: char = '■';
const GLYPH_NODE: char = '\u{F09D1}'; // nf-md-brain 󰧑 (needs a Nerd Font)
const GLYPH_SUPERNODE: char = '\u{F09D1}'; // nf-md-brain 󰧑, brighter blue than plain nodes
const GLYPH_ENTRANCE: char = '⌂';
const GLYPH_CURRENT: char = '◉';

/// Node rooms render pastel blue; supernodes a touch brighter.
const NODE_COLOR: Color = Color::Rgb(0xA7, 0xC7, 0xE7);
const SUPERNODE_COLOR: Color = Color::Rgb(0xBF, 0xE3, 0xFF);

pub struct MapPane {
    /// Columns per scene cell.
    zoom: u16,
    /// Room hit areas from the last render, absolute buffer coordinates.
    hits: Vec<(Rect, u32)>,
    /// Content area of the last render (wheel/click routing).
    last_area: Option<Rect>,
    /// Manual pan offset in character cells, applied on top of the
    /// current-room camera center. Reset when the character moves rooms.
    pan_x: i32,
    pan_y: i32,
    /// Active click-hold drag: (start x, start y, pan at start, max pointer
    /// displacement seen). Displacement distinguishes click from drag.
    drag: Option<(u16, u16, (i32, i32), u16)>,
    /// Room the camera last centered on; a change snaps the pan back.
    last_centered_room: Option<u32>,
}

impl Default for MapPane {
    fn default() -> Self {
        Self {
            zoom: 4,
            hits: Vec::new(),
            last_area: None,
            pan_x: 0,
            pan_y: 0,
            drag: None,
            last_centered_room: None,
        }
    }
}

/// Liang–Barsky clip of a segment to [0, w] x [0, h]. Returns the clipped
/// segment, or None when it lies entirely outside.
fn clip_segment(
    (x1, y1, x2, y2): (f64, f64, f64, f64),
    w: f64,
    h: f64,
) -> Option<(f64, f64, f64, f64)> {
    let (dx, dy) = (x2 - x1, y2 - y1);
    let mut t0: f64 = 0.0;
    let mut t1: f64 = 1.0;
    for (p, q) in [
        (-dx, x1),      // left:   x >= 0
        (dx, w - x1),   // right:  x <= w
        (-dy, y1),      // bottom: y >= 0
        (dy, h - y1),   // top:    y <= h
    ] {
        if p == 0.0 {
            if q < 0.0 {
                return None;
            }
            continue;
        }
        let r = q / p;
        if p < 0.0 {
            if r > t1 {
                return None;
            }
            t0 = t0.max(r);
        } else {
            if r < t0 {
                return None;
            }
            t1 = t1.min(r);
        }
    }
    Some((x1 + t0 * dx, y1 + t0 * dy, x1 + t1 * dx, y1 + t1 * dy))
}

impl MapPane {
    pub fn zoom_by(&mut self, delta: i16) {
        self.zoom = (self.zoom as i16 + delta).clamp(MIN_ZOOM as i16, MAX_ZOOM as i16) as u16;
    }

    /// Begin a click-hold drag at the given buffer position.
    pub fn begin_drag(&mut self, x: u16, y: u16) {
        self.drag = Some((x, y, (self.pan_x, self.pan_y), 0));
    }

    /// Update an active drag; returns true if a drag is in progress.
    /// Dragging moves the map with the pointer (grab-and-pull).
    pub fn drag_to(&mut self, x: u16, y: u16) -> bool {
        let Some((sx, sy, (opx, opy), moved)) = self.drag else {
            return false;
        };
        let dx = x as i32 - sx as i32;
        let dy = y as i32 - sy as i32;
        self.pan_x = opx + dx;
        self.pan_y = opy + dy;
        let dist = dx.unsigned_abs().max(dy.unsigned_abs()) as u16;
        self.drag = Some((sx, sy, (opx, opy), moved.max(dist)));
        true
    }

    /// Finish a drag. Returns Some(moved) when a drag was active — `moved`
    /// false means the pointer never left the click threshold, i.e. this
    /// release is a plain click.
    pub fn end_drag(&mut self) -> Option<bool> {
        self.drag.take().map(|(_, _, _, moved)| moved > 1)
    }

    pub fn dragging(&self) -> bool {
        self.drag.is_some()
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

        // Walking to a new room snaps any manual pan back to following the
        // character; panning is for looking around from where you stand.
        if current != self.last_centered_room {
            self.last_centered_room = current;
            self.pan_x = 0;
            self.pan_y = 0;
            self.drag = None;
        }

        // Cell -> absolute char coordinates. Rows per cell are half the
        // columns so distances read equally in both axes.
        let cw = self.zoom as i32;
        let ch = (self.zoom as i32 / 2).max(1);
        let ox = area.x as i32 + area.width as i32 / 2 - center.x * cw + self.pan_x;
        let oy = area.y as i32 + area.height as i32 / 2 - center.y * ch + self.pan_y;
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
                    // ratatui's canvas Line drops the whole segment when an
                    // endpoint is out of bounds, so clip to the pane here and
                    // always draw the visible portion.
                    let color = match edge.kind {
                        SceneEdgeKind::Directional => edge_color,
                        _ => connector_color,
                    };
                    let seg = (
                        (ax - area.x as i32) as f64 + 0.5,
                        flip_y(ay),
                        (bx - area.x as i32) as f64 + 0.5,
                        flip_y(by),
                    );
                    if let Some((x1, y1, x2, y2)) =
                        clip_segment(seg, area.width as f64, area.height as f64)
                    {
                        ctx.draw(&CanvasLine { x1, y1, x2, y2, color });
                    }
                }
            });
        canvas.render(area, buf);

        // Rooms. The footprint grows with zoom: a single glyph when tight,
        // bracketed glyph at mid zoom, rounded boxes (with the room id
        // inside at the top tier) when there's space.
        //
        // (box width, box height): odd width keeps the box centered on the
        // cell anchor where edges terminate.
        let (bw, bh): (i32, i32) = match self.zoom {
            0..=3 => (1, 1),
            4..=5 => (3, 1),
            6..=7 => (5, 2),
            _ => (7, 3),
        };
        let mut put = |buf: &mut Buffer, sx: i32, sy: i32, c: char, style: Style| {
            if in_area(sx, sy) {
                buf[(sx as u16, sy as u16)].set_char(c).set_style(style);
            }
        };
        for room in &sheet.rooms {
            if !group_visible(room.group) {
                continue;
            }
            let (sx, sy) = to_char(&room.cell);
            let x0 = sx - bw / 2;
            let y0 = sy - bh / 2;
            // Skip rooms with no visible part.
            if x0 + bw <= area.x as i32
                || x0 >= (area.x + area.width) as i32
                || y0 + bh <= area.y as i32
                || y0 >= (area.y + area.height) as i32
            {
                continue;
            }

            let is_current = current == Some(room.id);
            let mut style = if is_current {
                Style::default().fg(accent).add_modifier(Modifier::BOLD)
            } else if room.supernode {
                Style::default().fg(SUPERNODE_COLOR)
            } else if room.node {
                Style::default().fg(NODE_COLOR)
            } else if room.entrance {
                Style::default().fg(entrance_color)
            } else {
                Style::default().fg(room_color)
            };

            if bh == 1 && bw == 1 {
                let glyph = if is_current {
                    GLYPH_CURRENT
                } else if room.supernode {
                    GLYPH_SUPERNODE
                } else if room.node {
                    GLYPH_NODE
                } else if room.entrance {
                    GLYPH_ENTRANCE
                } else {
                    GLYPH_ROOM
                };
                put(buf, sx, sy, glyph, style);
            } else if bh == 1 {
                // Mid zoom: bracketed glyph, rounded by the parens.
                let glyph = if is_current {
                    GLYPH_CURRENT
                } else if room.supernode {
                    GLYPH_SUPERNODE
                } else if room.node {
                    GLYPH_NODE
                } else if room.entrance {
                    GLYPH_ENTRANCE
                } else {
                    GLYPH_ROOM
                };
                put(buf, sx - 1, sy, '(', style);
                put(buf, sx, sy, glyph, style);
                put(buf, sx + 1, sy, ')', style);
            } else {
                // Rounded box. Nodes keep their color on the outline.
                if is_current {
                    style = style.add_modifier(Modifier::BOLD);
                }
                let (x1, y1) = (x0 + bw - 1, y0 + bh - 1);
                put(buf, x0, y0, '╭', style);
                put(buf, x1, y0, '╮', style);
                put(buf, x0, y1, '╰', style);
                put(buf, x1, y1, '╯', style);
                for x in x0 + 1..x1 {
                    put(buf, x, y0, '─', style);
                    put(buf, x, y1, '─', style);
                }
                for y in y0 + 1..y1 {
                    put(buf, x0, y, '│', style);
                    put(buf, x1, y, '│', style);
                }
                // Interior: clear it so edge lines don't run through the
                // room, then center the room id (top tier) or a marker.
                for y in y0 + 1..y1 {
                    for x in x0 + 1..x1 {
                        put(buf, x, y, ' ', style);
                    }
                }
                // Marker in the box center (3-row tier) or on the top edge
                // (2-row tier, which has no middle row).
                if is_current || room.entrance || room.supernode || room.node {
                    let glyph = if is_current {
                        GLYPH_CURRENT
                    } else if room.supernode {
                        GLYPH_SUPERNODE
                    } else if room.node {
                        GLYPH_NODE
                    } else {
                        GLYPH_ENTRANCE
                    };
                    let gy = if bh >= 3 { y0 + bh / 2 } else { y0 };
                    put(buf, sx, gy, glyph, style);
                }
            }

            let clip_x0 = x0.max(area.x as i32);
            let clip_y0 = y0.max(area.y as i32);
            let clip_x1 = (x0 + bw).min((area.x + area.width) as i32);
            let clip_y1 = (y0 + bh).min((area.y + area.height) as i32);
            self.hits.push((
                Rect {
                    x: clip_x0 as u16,
                    y: clip_y0 as u16,
                    width: (clip_x1 - clip_x0) as u16,
                    height: (clip_y1 - clip_y0) as u16,
                },
                room.id,
            ));
        }

        // Connector labels ("stairway", "arch", ...) at the edge midpoint,
        // only at the top zoom tier where there's room for text.
        if self.zoom >= 8 {
            let label_style = Style::default()
                .fg(connector_color)
                .add_modifier(Modifier::ITALIC);
            for edge in &sheet.edges {
                let Some(label) = &edge.label else { continue };
                if edge.kind == SceneEdgeKind::Stub || !group_visible(edge.group) {
                    continue;
                }
                let (ax, ay) = to_char(&edge.a);
                let (bx, by) = to_char(&edge.b);
                let (mx, my) = ((ax + bx) / 2, (ay + by) / 2);
                let start = mx - label.chars().count() as i32 / 2;
                for (i, c) in label.chars().enumerate() {
                    put(buf, start + i as i32, my, c, label_style);
                }
            }
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
