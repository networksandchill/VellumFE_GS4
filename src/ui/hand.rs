use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    widgets::{Block, BorderType},
};
use regex::Regex;
use aho_corasick::{AhoCorasick, AhoCorasickBuilder, MatchKind};

/// Individual hand widget for left/right/spell hand
/// Shows just icon + text for a single hand
pub struct Hand {
    label: String,
    hand_type: HandType,
    content: String,
    icon: String,  // Configurable icon (e.g., "L:", "R:", "S:")
    show_border: bool,
    border_style: Option<String>,
    border_color: Option<String>,
    border_sides: Option<Vec<String>>,
    text_color: Option<String>,
    background_color: Option<String>,
    transparent_background: bool,
    // Highlight support
    highlights: Vec<crate::config::HighlightPattern>,
    highlight_regexes: Vec<Option<Regex>>,
    fast_matcher: Option<AhoCorasick>,
    fast_pattern_map: Vec<usize>,
}

#[derive(Debug, Clone, Copy)]
pub enum HandType {
    Left,
    Right,
    Spell,
}

impl Hand {
    pub fn new(label: &str, hand_type: HandType) -> Self {
        let default_icon = match hand_type {
            HandType::Left => "L:",
            HandType::Right => "R:",
            HandType::Spell => "S:",
        };

        Self {
            label: label.to_string(),
            hand_type,
            content: String::new(),
            icon: default_icon.to_string(),
            show_border: false,
            border_style: None,
            border_color: None,
            border_sides: None,
            text_color: None,  // Will use global default
            background_color: None,
            transparent_background: true,  // Default to transparent
            highlights: Vec::new(),
            highlight_regexes: Vec::new(),
            fast_matcher: None,
            fast_pattern_map: Vec::new(),
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

    pub fn set_border_sides(&mut self, border_sides: Option<Vec<String>>) {
        self.border_sides = border_sides;
    }

    pub fn set_title(&mut self, title: String) {
        self.label = title;
    }

    pub fn set_icon(&mut self, icon: String) {
        self.icon = icon;
    }

    pub fn set_content(&mut self, content: String) {
        // Truncate to 24 characters
        self.content = if content.len() > 24 {
            content.chars().take(24).collect()
        } else {
            content
        };
    }

    pub fn set_text_color(&mut self, color: Option<String>) {
        self.text_color = color;
    }

    pub fn set_background_color(&mut self, color: Option<String>) {
        // Handle three-state: None = transparent, Some("-") = transparent, Some(value) = use value
        self.background_color = match color {
            Some(ref s) if s == "-" => None,  // "-" means explicitly transparent
            other => other,
        };
    }

    pub fn set_transparent_background(&mut self, transparent: bool) {
        self.transparent_background = transparent;
    }

    pub fn set_highlights(&mut self, highlights: Vec<crate::config::HighlightPattern>) {
        // Separate fast_parse patterns from regex patterns
        let mut fast_patterns: Vec<String> = Vec::new();
        let mut fast_map: Vec<usize> = Vec::new();

        // Build regex list and collect fast_parse patterns
        self.highlight_regexes = highlights.iter()
            .enumerate()
            .map(|(i, h)| {
                if h.fast_parse {
                    // Split pattern on | and add to Aho-Corasick
                    for literal in h.pattern.split('|') {
                        let literal = literal.trim();
                        if !literal.is_empty() {
                            fast_patterns.push(literal.to_string());
                            fast_map.push(i);  // Map this pattern back to highlight index
                        }
                    }
                    None  // Don't compile as regex
                } else {
                    // Regular regex pattern
                    Regex::new(&h.pattern).ok()
                }
            })
            .collect();

        // Build Aho-Corasick matcher for fast_parse patterns
        if !fast_patterns.is_empty() {
            self.fast_matcher = AhoCorasickBuilder::new()
                .match_kind(MatchKind::Standard)
                .build(&fast_patterns)
                .ok();
            self.fast_pattern_map = fast_map;
        } else {
            self.fast_matcher = None;
            self.fast_pattern_map.clear();
        }

        self.highlights = highlights;
    }

    fn parse_hex_color(hex: &str) -> Option<Color> {
        if !hex.starts_with('#') || hex.len() != 7 {
            return None;
        }

        let r = u8::from_str_radix(&hex[1..3], 16).ok()?;
        let g = u8::from_str_radix(&hex[3..5], 16).ok()?;
        let b = u8::from_str_radix(&hex[5..7], 16).ok()?;

        Some(Color::Rgb(r, g, b))
    }

    /// Apply highlights to text and return styled segments (text, fg, bg, bold)
    fn apply_highlights_to_text(&self, text: &str) -> Vec<(String, Option<Color>, Option<Color>, bool)> {
        if self.highlights.is_empty() || text.is_empty() {
            return vec![(text.to_string(), None, None, false)];
        }

        // Find all matches
        let mut matches: Vec<(usize, usize, Option<Color>, Option<Color>, bool)> = Vec::new();

        // Try Aho-Corasick fast patterns
        if let Some(ref matcher) = self.fast_matcher {
            for mat in matcher.find_iter(text) {
                let start = mat.start();
                let end = mat.end();

                if let Some(&highlight_idx) = self.fast_pattern_map.get(mat.pattern().as_usize()) {
                    if let Some(highlight) = self.highlights.get(highlight_idx) {
                        let fg = highlight.fg.as_ref().and_then(|h| Self::parse_hex_color(h));
                        let bg = highlight.bg.as_ref().and_then(|h| Self::parse_hex_color(h));
                        matches.push((start, end, fg, bg, highlight.bold));
                    }
                }
            }
        }

        // Try regex patterns
        for (i, highlight) in self.highlights.iter().enumerate() {
            if highlight.fast_parse {
                continue;  // Already handled by Aho-Corasick
            }

            if let Some(Some(regex)) = self.highlight_regexes.get(i) {
                if let Some(captures) = regex.captures(text) {
                    if let Some(m) = captures.get(0) {
                        let fg = highlight.fg.as_ref().and_then(|h| Self::parse_hex_color(h));
                        let bg = highlight.bg.as_ref().and_then(|h| Self::parse_hex_color(h));
                        matches.push((m.start(), m.end(), fg, bg, highlight.bold));
                    }
                }
            }
        }

        if matches.is_empty() {
            return vec![(text.to_string(), None, None, false)];
        }

        // Sort matches by start position
        matches.sort_by_key(|(start, _, _, _, _)| *start);

        // Build segments
        let mut segments = Vec::new();
        let mut pos = 0;

        for (start, end, fg, bg, bold) in matches {
            // Add any text before this match
            if pos < start {
                segments.push((text[pos..start].to_string(), None, None, false));
            }

            // Add the matched text with highlighting
            segments.push((text[start..end].to_string(), fg, bg, bold));
            pos = end;
        }

        // Add any remaining text
        if pos < text.len() {
            segments.push((text[pos..].to_string(), None, None, false));
        }

        segments
    }

    fn parse_color(hex: &str) -> Color {
        if !hex.starts_with('#') || hex.len() != 7 {
            return Color::White;
        }

        let r = u8::from_str_radix(&hex[1..3], 16).unwrap_or(255);
        let g = u8::from_str_radix(&hex[3..5], 16).unwrap_or(255);
        let b = u8::from_str_radix(&hex[5..7], 16).unwrap_or(255);

        Color::Rgb(r, g, b)
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        // Determine which borders to show
        let borders = if self.show_border {
            crate::config::parse_border_sides(&self.border_sides)
        } else {
            ratatui::widgets::Borders::NONE
        };

        let border_color = self.border_color.as_ref()
            .map(|c| Self::parse_color(c))
            .unwrap_or(Color::White);

        // Check if we only have left/right borders (no top/bottom)
        let only_horizontal_borders = self.show_border &&
            (borders.contains(ratatui::widgets::Borders::LEFT) || borders.contains(ratatui::widgets::Borders::RIGHT)) &&
            !borders.contains(ratatui::widgets::Borders::TOP) &&
            !borders.contains(ratatui::widgets::Borders::BOTTOM);

        let inner_area: Rect;

        if only_horizontal_borders {
            // For left/right only borders, we'll manually render them on the content row
            let has_left = borders.contains(ratatui::widgets::Borders::LEFT);
            let has_right = borders.contains(ratatui::widgets::Borders::RIGHT);
            let border_width = (if has_left { 1 } else { 0 }) + (if has_right { 1 } else { 0 });

            inner_area = Rect {
                x: area.x + (if has_left { 1 } else { 0 }),
                y: area.y,
                width: area.width.saturating_sub(border_width),
                height: area.height,
            };
            // We'll render the borders later after content
        } else if self.show_border {
            // Use Block widget for all other border combinations
            let mut block = Block::default().borders(borders);

            if let Some(ref style) = self.border_style {
                let border_type = match style.as_str() {
                    "double" => BorderType::Double,
                    "rounded" => BorderType::Rounded,
                    "thick" => BorderType::Thick,
                    "quadrant_inside" => BorderType::QuadrantInside,
                    "quadrant_outside" => BorderType::QuadrantOutside,
                    _ => BorderType::Plain,
                };
                block = block.border_type(border_type);
            }

            block = block.border_style(Style::default().fg(border_color));
            block = block.title(self.label.as_str());

            inner_area = block.inner(area);
            use ratatui::widgets::Widget;
            block.render(area, buf);
        } else {
            inner_area = area;
        }

        if inner_area.width == 0 || inner_area.height == 0 {
            return;
        }

        // Fill entire area with background color if not transparent
        if !self.transparent_background {
            let bg_color = self.background_color
                .as_ref()
                .map(|c| Self::parse_color(c))
                .unwrap_or(Color::Reset);

            for row in 0..inner_area.height {
                for col in 0..inner_area.width {
                    let x = inner_area.x + col;
                    let y = inner_area.y + row;
                    buf[(x, y)].set_char(' ');
                    buf[(x, y)].set_bg(bg_color);
                }
            }
        }

        // Trust that text_color is always set by window manager from config resolution
        let text_color = self.text_color
            .as_ref()
            .map(|c| Self::parse_color(c))
            .unwrap_or(Color::Reset);  // Fallback to terminal default (should never happen)

        let y = inner_area.y;

        // Render icon using configurable icon field
        for (i, ch) in self.icon.chars().enumerate() {
            let x = inner_area.x + i as u16;
            if x < inner_area.x + inner_area.width {
                buf[(x, y)].set_char(ch);
                buf[(x, y)].set_fg(text_color);
                if !self.transparent_background {
                    let bg_color = self.background_color
                        .as_ref()
                        .map(|c| Self::parse_color(c))
                        .unwrap_or(Color::Reset);
                    buf[(x, y)].set_bg(bg_color);
                }
            }
        }

        // Render content after icon (+ 1 space) with highlights
        let start_col = self.icon.len() as u16 + 1;
        let segments = self.apply_highlights_to_text(&self.content);
        let mut col_offset = 0;

        for (segment_text, seg_fg, seg_bg, seg_bold) in segments {
            for ch in segment_text.chars() {
                let x = inner_area.x + start_col + col_offset;
                if x < inner_area.x + inner_area.width {
                    buf[(x, y)].set_char(ch);

                    // Use highlight color if available, otherwise use default text_color
                    let fg = seg_fg.or_else(|| Some(text_color)).unwrap_or(Color::Reset);
                    buf[(x, y)].set_fg(fg);

                    // Use highlight background if available, otherwise use widget background
                    if let Some(highlight_bg) = seg_bg {
                        buf[(x, y)].set_bg(highlight_bg);
                    } else if !self.transparent_background {
                        let bg_color = self.background_color
                            .as_ref()
                            .map(|c| Self::parse_color(c))
                            .unwrap_or(Color::Reset);
                        buf[(x, y)].set_bg(bg_color);
                    }

                    // Apply bold if highlight specifies it
                    if seg_bold {
                        let current_style = buf[(x, y)].style();
                        buf[(x, y)].set_style(current_style.add_modifier(ratatui::style::Modifier::BOLD));
                    }

                    col_offset += 1;
                } else {
                    break;
                }
            }
        }

        // If we have left/right only borders, render them manually on the content row
        if only_horizontal_borders {
            let content_y = inner_area.y; // Hand widgets always render at y=0 of inner area
            if content_y < buf.area().height {
                let has_left = borders.contains(ratatui::widgets::Borders::LEFT);
                let has_right = borders.contains(ratatui::widgets::Borders::RIGHT);

                // Render left border
                if has_left && area.x < buf.area().width {
                    buf[(area.x, content_y)].set_char('│');
                    buf[(area.x, content_y)].set_fg(border_color);
                }
                // Render right border
                if has_right {
                    let right_x = area.x + area.width.saturating_sub(1);
                    if right_x < buf.area().width {
                        buf[(right_x, content_y)].set_char('│');
                        buf[(right_x, content_y)].set_fg(border_color);
                    }
                }
            }
        }
    }

    pub fn render_with_focus(&self, area: Rect, buf: &mut Buffer, _focused: bool) {
        self.render(area, buf);
    }
}
