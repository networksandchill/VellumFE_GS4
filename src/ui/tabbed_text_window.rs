use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Tabs, Widget},
};
use super::{StyledText, TextWindow};

#[derive(Clone, Debug)]
pub enum TabBarPosition {
    Top,
    Bottom,
}

/// Split a tab stream spec into individual stream names.
/// A single tab may subscribe to multiple streams via a comma-separated
/// spec (e.g. "speech,whisper"). Whitespace around each name is trimmed
/// and empty entries are dropped, so "speech" yields `["speech"]`.
pub fn split_streams(spec: &str) -> Vec<String> {
    spec.split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}

/// Direction in which a split tab's panes are arranged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitDirection {
    /// Panes stacked top-to-bottom (each spans the full content width).
    Vertical,
    /// Panes placed side-by-side (each gets a slice of the width).
    Horizontal,
}

impl SplitDirection {
    pub fn parse(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "horizontal" | "h" | "cols" | "columns" | "side" => SplitDirection::Horizontal,
            _ => SplitDirection::Vertical,
        }
    }
}

/// One pane inside a split tab: an independent scrollback bound to its own stream(s).
struct TabPane {
    streams: Vec<String>,
    window: TextWindow,
    title: Option<String>,
    weight: u16,
}

struct TabInfo {
    name: String,
    /// Streams for a normal (single-window) tab. Empty for split tabs.
    streams: Vec<String>,
    /// The single text window for a normal tab. Unused (dummy) for split tabs.
    window: TextWindow,
    /// Non-empty => this is a split tab; `panes` are rendered instead of `window`.
    panes: Vec<TabPane>,
    split: SplitDirection,
    has_unread: bool,
    unread_count: usize,
}

impl TabInfo {
    fn is_split(&self) -> bool {
        !self.panes.is_empty()
    }

    /// The window used for search/selection/link detection on this tab
    /// (the first pane for a split tab, otherwise the single window).
    fn primary_window(&self) -> &TextWindow {
        if self.is_split() {
            &self.panes[0].window
        } else {
            &self.window
        }
    }

    fn primary_window_mut(&mut self) -> &mut TextWindow {
        if self.is_split() {
            &mut self.panes[0].window
        } else {
            &mut self.window
        }
    }
}

/// Divide `total` cells across `weights`, giving the last slice any remainder
/// so the sizes always sum back to `total`.
fn weighted_sizes(total: u16, weights: &[u16]) -> Vec<u16> {
    let norm: Vec<u32> = weights.iter().map(|&w| w.max(1) as u32).collect();
    let sum: u32 = norm.iter().sum();
    if sum == 0 {
        return vec![0; weights.len()];
    }
    let mut sizes = Vec::with_capacity(weights.len());
    let mut used: u32 = 0;
    for (i, &w) in norm.iter().enumerate() {
        if i + 1 == norm.len() {
            sizes.push((total as u32).saturating_sub(used) as u16);
        } else {
            let s = total as u32 * w / sum;
            used += s;
            sizes.push(s as u16);
        }
    }
    sizes
}

pub struct TabbedTextWindow {
    tabs: Vec<TabInfo>,
    active_tab_index: usize,
    tab_bar_position: TabBarPosition,
    // Border and styling
    show_border: bool,
    border_style: Option<String>,
    border_color: Option<String>,
    title: String,
    // Background
    transparent_background: bool,
    background_color: Option<String>,
    // Tab styling
    tab_active_color: Option<String>,
    tab_inactive_color: Option<String>,
    tab_unread_color: Option<String>,
    tab_unread_prefix: String,
}

impl TabbedTextWindow {
    /// Create a new empty tabbed window (tabs can be added later)
    pub fn new(
        title: impl Into<String>,
        tab_bar_position: TabBarPosition,
    ) -> Self {
        Self {
            tabs: Vec::new(),
            active_tab_index: 0,
            tab_bar_position,
            show_border: true,
            border_style: None,
            border_color: None,
            title: title.into(),
            transparent_background: false,
            background_color: None,
            tab_active_color: None,
            tab_inactive_color: None,
            tab_unread_color: None,
            tab_unread_prefix: "* ".to_string(),
        }
    }

    /// Create a new tabbed window with initial tabs
    pub fn with_tabs(
        title: impl Into<String>,
        tabs: Vec<(String, String)>, // (tab_name, stream_name)
        max_lines_per_tab: usize,
    ) -> Self {
        let tab_infos: Vec<TabInfo> = tabs
            .into_iter()
            .map(|(name, stream)| TabInfo {
                name: name.clone(),
                streams: split_streams(&stream),
                window: TextWindow::new(name.clone(), max_lines_per_tab),
                panes: Vec::new(),
                split: SplitDirection::Vertical,
                has_unread: false,
                unread_count: 0,
            })
            .collect();

        Self {
            tabs: tab_infos,
            active_tab_index: 0,
            tab_bar_position: TabBarPosition::Top,
            show_border: true,
            border_style: None,
            border_color: None,
            title: title.into(),
            transparent_background: false,
            background_color: None,
            tab_active_color: None,
            tab_inactive_color: None,
            tab_unread_color: None,
            tab_unread_prefix: "* ".to_string(),
        }
    }

    pub fn with_border_config(
        mut self,
        show_border: bool,
        border_style: Option<String>,
        border_color: Option<String>,
    ) -> Self {
        self.show_border = show_border;
        self.border_style = border_style;
        self.border_color = border_color;
        self
    }

    pub fn with_tab_bar_position(mut self, position: TabBarPosition) -> Self {
        self.tab_bar_position = position;
        self
    }

    pub fn with_tab_colors(
        mut self,
        active: Option<String>,
        inactive: Option<String>,
        unread: Option<String>,
    ) -> Self {
        self.tab_active_color = active;
        self.tab_inactive_color = inactive;
        self.tab_unread_color = unread;
        self
    }

    pub fn with_unread_prefix(mut self, prefix: String) -> Self {
        self.tab_unread_prefix = prefix;
        self
    }

    pub fn set_title(&mut self, title: String) {
        self.title = title;
    }

    pub fn has_border(&self) -> bool {
        self.show_border
    }

    /// Check if tabs are positioned at the bottom
    pub fn has_bottom_tabs(&self) -> bool {
        matches!(self.tab_bar_position, TabBarPosition::Bottom)
    }

    pub fn set_border_config(
        &mut self,
        show_border: bool,
        border_style: Option<String>,
        border_color: Option<String>,
    ) {
        self.show_border = show_border;
        self.border_style = border_style;
        self.border_color = border_color;
    }

    pub fn set_tab_bar_position(&mut self, position: TabBarPosition) {
        self.tab_bar_position = position;
    }

    pub fn set_tab_active_color(&mut self, color: String) {
        self.tab_active_color = Some(color);
    }

    pub fn set_tab_inactive_color(&mut self, color: String) {
        self.tab_inactive_color = Some(color);
    }

    pub fn set_tab_unread_color(&mut self, color: String) {
        self.tab_unread_color = Some(color);
    }

    pub fn set_unread_prefix(&mut self, prefix: String) {
        self.tab_unread_prefix = prefix;
    }

    pub fn set_transparent_background(&mut self, transparent: bool) {
        self.transparent_background = transparent;
    }

    pub fn set_background_color(&mut self, color: Option<String>) {
        // Handle three-state: None = transparent, Some("-") = transparent, Some(value) = use value
        self.background_color = match color {
            Some(ref s) if s == "-" => None,  // "-" means explicitly transparent
            other => other,
        };
    }

    /// Add a new tab dynamically
    pub fn add_tab(&mut self, name: String, stream: String, max_lines: usize, show_timestamps: bool) {
        let mut window = TextWindow::new(name.clone(), max_lines);
        window.set_show_timestamps(show_timestamps);
        self.tabs.push(TabInfo {
            name,
            streams: split_streams(&stream),
            window,
            panes: Vec::new(),
            split: SplitDirection::Vertical,
            has_unread: false,
            unread_count: 0,
        });
    }

    /// Add a split tab: its content area is divided into panes, each pane an
    /// independent scrollback bound to its own comma-separated stream spec.
    /// `panes` entries are `(stream_spec, title, weight, show_timestamps)`.
    pub fn add_split_tab(
        &mut self,
        name: String,
        direction: SplitDirection,
        panes: Vec<(String, Option<String>, u16, bool)>,
        max_lines: usize,
    ) {
        let pane_infos: Vec<TabPane> = panes
            .into_iter()
            .map(|(spec, title, weight, show_ts)| {
                let mut window = TextWindow::new(name.clone(), max_lines);
                window.set_show_timestamps(show_ts);
                TabPane {
                    streams: split_streams(&spec),
                    window,
                    title,
                    weight: weight.max(1),
                }
            })
            .collect();
        self.tabs.push(TabInfo {
            name: name.clone(),
            streams: Vec::new(),
            window: TextWindow::new(name, max_lines),
            panes: pane_infos,
            split: direction,
            has_unread: false,
            unread_count: 0,
        });
    }

    /// Add text to a specific stream (will route to correct tab and set unread if needed)
    pub fn add_text_to_stream(&mut self, stream: &str, styled: StyledText) {
        // Locate the target tab (and pane within a split tab) before mutating,
        // so we don't hold a borrow while touching unread state.
        if let Some((idx, pane_idx)) = self.locate_stream(stream) {
            let active = self.active_tab_index;
            let tab = &mut self.tabs[idx];
            match pane_idx {
                Some(p) => tab.panes[p].window.add_text(styled),
                None => tab.window.add_text(styled),
            }
            if idx != active {
                tab.has_unread = true;
                tab.unread_count += 1;
            }
        }
    }

    /// Finish line for a specific stream
    pub fn finish_line_for_stream(&mut self, stream: &str, width: u16) {
        if let Some((idx, pane_idx)) = self.locate_stream(stream) {
            let tab = &mut self.tabs[idx];
            match pane_idx {
                Some(p) => tab.panes[p].window.finish_line(width),
                None => tab.window.finish_line(width),
            }
        }
    }

    /// Find which tab (and, for split tabs, which pane) owns a stream.
    /// Returns `(tab_index, Some(pane_index))` for split tabs or
    /// `(tab_index, None)` for normal tabs.
    fn locate_stream(&self, stream: &str) -> Option<(usize, Option<usize>)> {
        for (idx, tab) in self.tabs.iter().enumerate() {
            if tab.is_split() {
                if let Some(p) = tab
                    .panes
                    .iter()
                    .position(|pane| pane.streams.iter().any(|s| s == stream))
                {
                    return Some((idx, Some(p)));
                }
            } else if tab.streams.iter().any(|s| s == stream) {
                return Some((idx, None));
            }
        }
        None
    }

    /// Get the active tab's primary stream name
    pub fn get_active_stream(&self) -> Option<&str> {
        self.tabs
            .get(self.active_tab_index)
            .and_then(|t| {
                if t.is_split() {
                    t.panes.first().and_then(|p| p.streams.first())
                } else {
                    t.streams.first()
                }
            })
            .map(|s| s.as_str())
    }

    /// Get the active tab's text window (for selection/link detection).
    /// For split tabs this is the first pane.
    pub fn get_active_window(&self) -> Option<&TextWindow> {
        self.tabs.get(self.active_tab_index).map(|t| t.primary_window())
    }

    /// Get the active tab's text window mutably (for clipboard copy).
    /// For split tabs this is the first pane.
    pub fn get_active_window_mut(&mut self) -> Option<&mut TextWindow> {
        self.tabs.get_mut(self.active_tab_index).map(|t| t.primary_window_mut())
    }

    /// Get all stream names for this tabbed window (flattened across tabs and panes)
    pub fn get_all_streams(&self) -> Vec<String> {
        let mut out = Vec::new();
        for tab in &self.tabs {
            out.extend(tab.streams.iter().cloned());
            for pane in &tab.panes {
                out.extend(pane.streams.iter().cloned());
            }
        }
        out
    }

    /// Switch to a specific tab by index
    pub fn switch_to_tab(&mut self, index: usize) {
        if index < self.tabs.len() {
            self.active_tab_index = index;
            // Clear unread flag
            self.tabs[index].has_unread = false;
            self.tabs[index].unread_count = 0;
        }
    }

    /// Switch to tab by name
    pub fn switch_to_tab_by_name(&mut self, name: &str) {
        if let Some(idx) = self.tabs.iter().position(|t| t.name == name) {
            self.switch_to_tab(idx);
        }
    }

    /// Remove a tab by name
    pub fn remove_tab(&mut self, name: &str) -> bool {
        if self.tabs.len() <= 1 {
            return false; // Can't remove last tab
        }

        if let Some(idx) = self.tabs.iter().position(|t| t.name == name) {
            self.tabs.remove(idx);

            // Adjust active index if needed
            if self.active_tab_index >= self.tabs.len() {
                self.active_tab_index = self.tabs.len() - 1;
            }

            true
        } else {
            false
        }
    }

    /// Rename a tab
    pub fn rename_tab(&mut self, old_name: &str, new_name: String) -> bool {
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.name == old_name) {
            tab.name = new_name.clone();
            tab.window.set_title(new_name);
            true
        } else {
            false
        }
    }

    /// Get list of current tab names
    pub fn get_tab_names(&self) -> Vec<String> {
        self.tabs.iter().map(|t| t.name.clone()).collect()
    }

    /// Reorder tabs to match a given order of tab names
    pub fn reorder_tabs(&mut self, new_order: &[String]) {
        let mut new_tabs = Vec::new();

        // Build new tab list in the specified order
        for name in new_order {
            if let Some(pos) = self.tabs.iter().position(|t| &t.name == name) {
                new_tabs.push(self.tabs.remove(pos));
            }
        }

        // Add any remaining tabs that weren't in the new order (shouldn't happen, but be safe)
        new_tabs.append(&mut self.tabs);

        self.tabs = new_tabs;

        // Adjust active index if needed
        if self.active_tab_index >= self.tabs.len() {
            self.active_tab_index = self.tabs.len().saturating_sub(1);
        }
    }

    /// Scroll the active tab (all panes together for a split tab)
    pub fn scroll_up(&mut self, lines: usize) {
        if let Some(tab) = self.tabs.get_mut(self.active_tab_index) {
            if tab.is_split() {
                for pane in &mut tab.panes {
                    pane.window.scroll_up(lines);
                }
            } else {
                tab.window.scroll_up(lines);
            }
        }
    }

    pub fn scroll_down(&mut self, lines: usize) {
        if let Some(tab) = self.tabs.get_mut(self.active_tab_index) {
            if tab.is_split() {
                for pane in &mut tab.panes {
                    pane.window.scroll_down(lines);
                }
            } else {
                tab.window.scroll_down(lines);
            }
        }
    }

    /// Update inner width for all tabs. Split-tab panes get their own width:
    /// full content width when stacked vertically, or a weighted slice when
    /// arranged horizontally (matching the render layout).
    pub fn update_inner_width(&mut self, width: u16) {
        for tab in &mut self.tabs {
            if !tab.is_split() {
                tab.window.update_inner_width(width);
                continue;
            }
            match tab.split {
                SplitDirection::Vertical => {
                    for pane in &mut tab.panes {
                        pane.window.update_inner_width(width);
                    }
                }
                SplitDirection::Horizontal => {
                    let weights: Vec<u16> = tab.panes.iter().map(|p| p.weight).collect();
                    let widths = weighted_sizes(width, &weights);
                    for (pane, w) in tab.panes.iter_mut().zip(widths) {
                        pane.window.update_inner_width(w.max(1));
                    }
                }
            }
        }
    }

    /// Parse color string (hex or named)
    fn parse_color(color_str: &str) -> Option<Color> {
        if color_str.starts_with('#') && color_str.len() == 7 {
            // Hex color
            let r = u8::from_str_radix(&color_str[1..3], 16).ok()?;
            let g = u8::from_str_radix(&color_str[3..5], 16).ok()?;
            let b = u8::from_str_radix(&color_str[5..7], 16).ok()?;
            Some(Color::Rgb(r, g, b))
        } else {
            // Named color
            match color_str.to_lowercase().as_str() {
                "black" => Some(Color::Black),
                "red" => Some(Color::Red),
                "green" => Some(Color::Green),
                "yellow" => Some(Color::Yellow),
                "blue" => Some(Color::Blue),
                "magenta" => Some(Color::Magenta),
                "cyan" => Some(Color::Cyan),
                "gray" | "grey" => Some(Color::Gray),
                "white" => Some(Color::White),
                "darkgray" | "darkgrey" => Some(Color::DarkGray),
                _ => None,
            }
        }
    }

    /// Render the tab bar
    fn render_tab_bar(&self, area: Rect, buf: &mut Buffer, focused_border_color: &str) {
        let tab_titles: Vec<Line> = self
            .tabs
            .iter()
            .enumerate()
            .map(|(idx, tab)| {
                if idx == self.active_tab_index {
                    // Active tab: use focused border color (or tab_active_color as fallback)
                    let color = if let Some(color) = Self::parse_color(focused_border_color) {
                        color
                    } else {
                        self.tab_active_color
                            .as_ref()
                            .and_then(|c| Self::parse_color(c))
                            .unwrap_or(Color::Yellow)
                    };

                    Line::from(Span::styled(
                        tab.name.clone(),
                        Style::default().fg(color),
                    ))
                } else if tab.has_unread {
                    // Inactive with unread: indicator prefix + color
                    let color = self
                        .tab_unread_color
                        .as_ref()
                        .and_then(|c| Self::parse_color(c))
                        .unwrap_or(Color::White);

                    Line::from(Span::styled(
                        format!("{}{}", self.tab_unread_prefix, tab.name),
                        Style::default().fg(color).add_modifier(Modifier::BOLD),
                    ))
                } else {
                    // Inactive, no unread: dim
                    let color = self
                        .tab_inactive_color
                        .as_ref()
                        .and_then(|c| Self::parse_color(c))
                        .unwrap_or(Color::DarkGray);

                    Line::from(Span::styled(
                        tab.name.clone(),
                        Style::default().fg(color),
                    ))
                }
            })
            .collect();

        // Use focused border color for selected tab highlight (bold, no background)
        let highlight_color = Self::parse_color(focused_border_color).unwrap_or(Color::Yellow);
        let highlight_style = Style::default()
            .fg(highlight_color)
            .add_modifier(Modifier::BOLD);

        let tabs_widget = Tabs::new(tab_titles)
            .select(self.active_tab_index)
            .highlight_style(highlight_style)
            .divider("|");

        tabs_widget.render(area, buf);
    }

    /// Update highlights for all tabs (and every pane of split tabs)
    pub fn set_highlights(&mut self, highlights: Vec<crate::config::HighlightPattern>) {
        for tab in &mut self.tabs {
            tab.window.set_highlights(highlights.clone());
            for pane in &mut tab.panes {
                pane.window.set_highlights(highlights.clone());
            }
        }
    }

    /// Render with focus indicator
    pub fn render_with_focus(
        &mut self,
        area: Rect,
        buf: &mut Buffer,
        focused: bool,
        selection_state: Option<&crate::selection::SelectionState>,
        selection_bg_color: &str,
        window_index: usize,
        focused_border_color: &str,
    ) {
        // Create border block
        let mut block = Block::default();

        if self.show_border {
            let borders = if focused {
                Borders::ALL
            } else {
                Borders::ALL
            };
            block = block.borders(borders);

            // Apply border style
            if let Some(style_str) = &self.border_style {
                let border_type = match style_str.as_str() {
                    "double" => BorderType::Double,
                    "rounded" => BorderType::Rounded,
                    "thick" => BorderType::Thick,
                    "quadrant_inside" => BorderType::QuadrantInside,
                    "quadrant_outside" => BorderType::QuadrantOutside,
                    _ => BorderType::Plain,
                };
                block = block.border_type(border_type);
            }

            // Apply border color
            let border_style = if focused {
                // Use global focused border color when focused
                let color = Self::parse_color(focused_border_color).unwrap_or(Color::Yellow);
                Style::default().fg(color).add_modifier(Modifier::BOLD)
            } else if let Some(color_str) = &self.border_color {
                // Use window's border color when not focused
                if let Some(color) = Self::parse_color(color_str) {
                    Style::default().fg(color)
                } else {
                    Style::default()
                }
            } else {
                Style::default()
            };
            block = block.border_style(border_style);

            // Only set title when borders are shown
            block = block.title(self.title.clone());
        }

        let inner = block.inner(area);
        block.render(area, buf);

        // Calculate tab bar and content areas
        let (tab_bar_area, content_area) = match self.tab_bar_position {
            TabBarPosition::Top => {
                let tab_bar = Rect {
                    x: inner.x,
                    y: inner.y,
                    width: inner.width,
                    height: 1,
                };
                let content = Rect {
                    x: inner.x,
                    y: inner.y + 1,
                    width: inner.width,
                    height: inner.height.saturating_sub(1),
                };
                (tab_bar, content)
            }
            TabBarPosition::Bottom => {
                let content = Rect {
                    x: inner.x,
                    y: inner.y,
                    width: inner.width,
                    height: inner.height.saturating_sub(1),
                };
                let tab_bar = Rect {
                    x: inner.x,
                    y: inner.y + content.height,
                    width: inner.width,
                    height: 1,
                };
                (tab_bar, content)
            }
        };

        // Fill background if not transparent
        if !self.transparent_background {
            let bg_color = if let Some(ref color_str) = self.background_color {
                Self::parse_color(color_str).unwrap_or(Color::Black)
            } else {
                Color::Black
            };

            // Fill with space characters and background color
            for y in inner.y..inner.y + inner.height {
                for x in inner.x..inner.x + inner.width {
                    if x < buf.area.width && y < buf.area.height {
                        buf[(x, y)].set_char(' ').set_bg(bg_color);
                    }
                }
            }
        }

        // Render tab bar
        self.render_tab_bar(tab_bar_area, buf, focused_border_color);

        // Render active tab's content
        if let Some(tab) = self.tabs.get_mut(self.active_tab_index) {
            if !tab.is_split() {
                tab.window.render_with_focus(content_area, buf, focused, selection_state, selection_bg_color, window_index, focused_border_color);
            } else {
                let axis_total = match tab.split {
                    SplitDirection::Vertical => content_area.height,
                    SplitDirection::Horizontal => content_area.width,
                };
                let weights: Vec<u16> = tab.panes.iter().map(|p| p.weight).collect();
                let sizes = weighted_sizes(axis_total, &weights);
                let mut offset: u16 = 0;
                for (pane, size) in tab.panes.iter_mut().zip(sizes) {
                    let pane_rect = match tab.split {
                        SplitDirection::Vertical => Rect {
                            x: content_area.x,
                            y: content_area.y + offset,
                            width: content_area.width,
                            height: size,
                        },
                        SplitDirection::Horizontal => Rect {
                            x: content_area.x + offset,
                            y: content_area.y,
                            width: size,
                            height: content_area.height,
                        },
                    };
                    offset += size;

                    if pane_rect.width == 0 || pane_rect.height == 0 {
                        continue;
                    }

                    // Optional 1-row pane header label, leaving the rest for the window.
                    let win_rect = if let Some(ref title) = pane.title {
                        let label = format!(" {} ", title);
                        let style = Style::default()
                            .fg(Color::DarkGray)
                            .add_modifier(Modifier::BOLD);
                        buf.set_stringn(pane_rect.x, pane_rect.y, &label, pane_rect.width as usize, style);
                        Rect {
                            x: pane_rect.x,
                            y: pane_rect.y + 1,
                            width: pane_rect.width,
                            height: pane_rect.height.saturating_sub(1),
                        }
                    } else {
                        pane_rect
                    };

                    if win_rect.height > 0 {
                        pane.window.render_with_focus(win_rect, buf, focused, selection_state, selection_bg_color, window_index, focused_border_color);
                    }
                }
            }
        }
    }

    /// Calculate which tab was clicked based on X position in tab bar
    pub fn get_tab_at_position(&self, x: u16, tab_bar_rect: Rect) -> Option<usize> {
        if x < tab_bar_rect.x || x >= tab_bar_rect.x + tab_bar_rect.width {
            return None;
        }

        // Calculate actual tab widths based on text content
        // Each tab is: " TabName " (1 space before, 1 space after)
        // Plus divider "|" between tabs (1 char)
        let relative_x = x - tab_bar_rect.x;
        let mut current_x = 0u16;

        for (idx, tab) in self.tabs.iter().enumerate() {
            // Calculate the display text for this tab
            let display_text = if idx == self.active_tab_index {
                tab.name.clone()
            } else if tab.has_unread {
                format!("{}{}", self.tab_unread_prefix, tab.name)
            } else {
                tab.name.clone()
            };

            // Tab width = 1 space + text + 1 space = text.len() + 2
            let tab_width = (display_text.len() + 2) as u16;

            // Add divider width if not the first tab (1 char for "|")
            let divider_width = if idx > 0 { 1 } else { 0 };
            let total_width = divider_width + tab_width;

            // Check if click is within this tab's bounds
            if relative_x >= current_x && relative_x < current_x + total_width {
                return Some(idx);
            }

            current_x += total_width;
        }

        None
    }

    /// Get tab bar rect for mouse detection
    pub fn get_tab_bar_rect(&self, window_rect: Rect) -> Rect {
        // Use the same logic as render_with_focus to calculate inner area
        let mut block = Block::default();
        if self.show_border {
            block = block.borders(Borders::ALL);
            // Only set title when borders are shown (matching render_with_focus)
            block = block.title(self.title.clone());
        }
        let inner = block.inner(window_rect);

        match self.tab_bar_position {
            TabBarPosition::Top => Rect {
                x: inner.x,
                y: inner.y,
                width: inner.width,
                height: 1,
            },
            TabBarPosition::Bottom => Rect {
                x: inner.x,
                y: inner.y + inner.height.saturating_sub(1),
                width: inner.width,
                height: 1,
            },
        }
    }

    /// Start search in the active tab's text window
    pub fn start_search(&mut self, pattern: &str) -> Result<usize, regex::Error> {
        if let Some(tab) = self.tabs.get_mut(self.active_tab_index) {
            tab.primary_window_mut().start_search(pattern)
        } else {
            Ok(0)
        }
    }

    /// Clear search from the active tab's text window
    pub fn clear_search(&mut self) {
        if let Some(tab) = self.tabs.get_mut(self.active_tab_index) {
            tab.primary_window_mut().clear_search();
        }
    }

    /// Clear all text from all tabs (and panes)
    pub fn clear_all(&mut self) {
        for tab in &mut self.tabs {
            tab.window.clear();
            for pane in &mut tab.panes {
                pane.window.clear();
            }
        }
    }

    /// Clear text from a specific stream's tab/pane only
    pub fn clear_stream(&mut self, stream: &str) {
        if let Some((idx, pane_idx)) = self.locate_stream(stream) {
            let tab = &mut self.tabs[idx];
            match pane_idx {
                Some(p) => tab.panes[p].window.clear(),
                None => tab.window.clear(),
            }
        }
    }

    /// Go to next match in the active tab's text window
    pub fn next_match(&mut self) -> bool {
        if let Some(tab) = self.tabs.get_mut(self.active_tab_index) {
            tab.primary_window_mut().next_match()
        } else {
            false
        }
    }

    /// Go to previous match in the active tab's text window
    pub fn prev_match(&mut self) -> bool {
        if let Some(tab) = self.tabs.get_mut(self.active_tab_index) {
            tab.primary_window_mut().prev_match()
        } else {
            false
        }
    }

    /// Get search info (current match, total matches) from the active tab's text window
    pub fn search_info(&self) -> Option<(usize, usize)> {
        if let Some(tab) = self.tabs.get(self.active_tab_index) {
            tab.primary_window().search_info()
        } else {
            None
        }
    }
}
