//! Vital/stat style progress bar used throughout the HUD.
//!
//! Provides configurable borders, text, and fill colors so it matches the theme
//! chosen by the user.

use super::colors::parse_color_to_ratatui;
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    widgets::{Block, BorderType, Borders, Clear, Widget},
};

/// A progress bar widget for displaying vitals (health, mana, stamina, spirit)
pub struct ProgressBar {
    label: String,
    current: u32,
    max: u32,
    custom_text: Option<String>,
    show_border: bool,
    border_style: Option<String>,
    border_color: Option<Color>,
    /// Title color override; None paints the title in the border color.
    title_color: Option<String>,
    border_sides: crate::config::BorderSides,
    bar_fill: Option<Color>,
    bar_background: Option<Color>,
    window_background: Option<Color>,
    transparent_background: bool,
    text_color: Option<Color>,
    text_align_left: bool,
    /// Pill style: rounded end caps with a visible track. Effect-list rows
    /// (ScrollableContainer) turn this off — they pad text to full width.
    pill: bool,
}

impl ProgressBar {
    pub fn new(label: &str) -> Self {
        Self {
            label: label.to_string(),
            current: 0,
            max: 100,
            custom_text: None,
            show_border: false,
            border_style: None,
            border_color: None,
            title_color: None,
            border_sides: crate::config::BorderSides::default(),
            bar_fill: Some(super::colors::rgb_to_ratatui_color(0, 255, 0)), // Green by default
            bar_background: None,
            window_background: None,
            transparent_background: true,
            text_color: Some(Color::White),
            text_align_left: false,
            pill: true,
        }
    }

    pub fn set_pill(&mut self, pill: bool) {
        self.pill = pill;
    }

    /// Set the title color; None makes the title follow the border color.
    pub fn set_title_color(&mut self, title_color: Option<String>) {
        self.title_color = title_color;
    }

    pub fn set_border_config(
        &mut self,
        show_border: bool,
        border_style: Option<String>,
        border_color: Option<String>,
        border_sides: crate::config::BorderSides,
    ) {
        self.show_border = show_border;
        self.border_style = border_style;
        self.border_color = border_color.and_then(|c| Self::parse_color(&c));
        self.border_sides = border_sides;
    }

    pub fn set_title(&mut self, title: String) {
        self.label = title;
    }

    pub fn set_colors(&mut self, bar_fill: Option<String>, bar_background: Option<String>) {
        if let Some(fill) = bar_fill.and_then(|c| Self::parse_color(&c)) {
            self.bar_fill = Some(fill);
        }
        if let Some(bg) = bar_background.and_then(|c| Self::parse_color(&c)) {
            self.bar_background = Some(bg);
        }
    }

    pub fn set_value(&mut self, current: u32, max: u32) {
        self.current = current;
        self.max = max;
        self.custom_text = None;
    }

    pub fn set_value_with_text(&mut self, current: u32, max: u32, custom_text: Option<String>) {
        self.current = current;
        self.max = max;
        self.custom_text = custom_text;
    }

    pub fn set_transparent_background(&mut self, transparent: bool) {
        self.transparent_background = transparent;
    }

    pub fn set_text_color(&mut self, color: Option<String>) {
        self.text_color = color.and_then(|c| Self::parse_color(&c));
    }

    /// Control whether the overlay text is left aligned (default: centered).
    pub fn set_text_align_left(&mut self, align_left: bool) {
        self.text_align_left = align_left;
    }

    pub fn set_background_color(&mut self, color: Option<String>) {
        self.window_background = color.and_then(|c| Self::parse_color(&c));
    }

    fn parse_color(input: &str) -> Option<Color> {
        parse_color_to_ratatui(input)
    }

    fn luminance(color: Color) -> f32 {
        match color {
            Color::Rgb(r, g, b) => {
                let to_lin = |c: u8| {
                    let c = c as f32 / 255.0;
                    if c <= 0.03928 {
                        c / 12.92
                    } else {
                        ((c + 0.055) / 1.055).powf(2.4)
                    }
                };
                0.2126 * to_lin(r) + 0.7152 * to_lin(g) + 0.0722 * to_lin(b)
            }
            Color::Black => 0.0,
            Color::White => 1.0,
            _ => 0.5, // reasonable mid fallback
        }
    }

    fn ensure_contrast(text: Color, bg: Option<Color>) -> Color {
        let Some(bg) = bg else { return text };
        let l_text = Self::luminance(text);
        let l_bg = Self::luminance(bg);
        let (max, min) = if l_text > l_bg {
            (l_text, l_bg)
        } else {
            (l_bg, l_text)
        };
        let contrast = (max + 0.05) / (min + 0.05);

        if contrast >= 3.0 {
            text
        } else if l_bg > 0.5 {
            Color::Black
        } else {
            Color::White
        }
    }

    pub fn render(&self, area: Rect, buf: &mut Buffer) {
        let mut min_width = 1;
        if self.show_border {
            if self.border_sides.left {
                min_width += 1;
            }
            if self.border_sides.right {
                min_width += 1;
            }
            // If a top or bottom border exists, make sure we reserve at least
            // two columns so the border characters have room.
            if self.border_sides.top || self.border_sides.bottom {
                min_width = min_width.max(2);
            }
        }

        if area.width < min_width || area.height < 1 {
            return;
        }

        if !self.show_border && area.width == 0 {
            return;
        }

        Clear.render(area, buf);

        if !self.transparent_background {
            let bg_color = self
                .window_background
                .or(self.bar_background)
                .unwrap_or(Color::Reset);
            for row in 0..area.height {
                for col in 0..area.width {
                    let x = area.x + col;
                    let y = area.y + row;
                    if x < buf.area().width && y < buf.area().height {
                        buf[(x, y)].set_bg(bg_color);
                    }
                }
            }
        }

        let inner_area = if self.show_border {
            let mut borders = Borders::NONE;
            if self.border_sides.left {
                borders |= Borders::LEFT;
            }
            if self.border_sides.right {
                borders |= Borders::RIGHT;
            }
            if self.border_sides.top {
                borders |= Borders::TOP;
            }
            if self.border_sides.bottom {
                borders |= Borders::BOTTOM;
            }

            let mut block = Block::default().borders(borders);

            if let Some(ref style) = self.border_style {
                let border_type = match style.as_str() {
                    "double" => BorderType::Double,
                    "rounded" => BorderType::Rounded,
                    "thick" => BorderType::Thick,
                    _ => BorderType::Plain,
                };
                block = block.border_type(border_type);
            }

            if let Some(color) = self.border_color {
                block = block.border_style(Style::default().fg(color));
            }

            // Only set title if label is non-empty (avoids empty title affecting layout)
            if !self.label.is_empty() {
                block = block.title(self.label.as_str());
                if let Some(color) = self
                    .title_color
                    .as_deref()
                    .and_then(super::colors::parse_color_to_ratatui)
                {
                    block = block.title_style(Style::default().fg(color));
                }
            }

            let inner = block.inner(area);
            use ratatui::widgets::Widget;
            block.render(area, buf);
            // If inner area collapsed to zero, keep borders visible but skip content
            // (previously fell back to full area which overwrote borders)
            inner
        } else {
            area
        };

        if inner_area.width == 0 || inner_area.height == 0 {
            return;
        }

        // Calculate percentage
        let percentage = if self.max > 0 {
            (self.current as f64 / self.max as f64 * 100.0) as u32
        } else {
            0
        };

        // Build display text
        let display_text = if let Some(ref custom) = self.custom_text {
            custom.clone()
        } else {
            format!("{}/{}", self.current, self.max)
        };

        let text_width = display_text.len() as u16;
        let available_width = inner_area.width;

        let bar_color = self.bar_fill.unwrap_or(Color::Green);
        let bar_bg_color = self
            .bar_background
            .or(self.window_background)
            .unwrap_or(Color::Reset);

        let y = inner_area.y;
        let fraction = if self.max > 0 {
            (self.current as f64 / self.max as f64).clamp(0.0, 1.0)
        } else {
            0.0
        };

        // Pill style: rounded end caps with the fill/track between them.
        // Needs three columns; narrower bars fall back to the flat style.
        if self.pill && available_width >= 3 {
            let track = if self.transparent_background {
                dim_color(bar_color)
            } else {
                self.bar_background
                    .or(self.window_background)
                    .unwrap_or_else(|| dim_color(bar_color))
            };
            let text_fg = Self::ensure_contrast(
                self.text_color.unwrap_or(Color::White),
                Some(if fraction > 0.0 { bar_color } else { track }),
            );
            render_pill_bar(
                buf,
                inner_area.x,
                y,
                available_width,
                fraction,
                bar_color,
                track,
                &display_text,
                text_fg,
                self.text_align_left,
            );
            return;
        }

        // Calculate split point based on percentage
        let split_position = ((percentage as f64 / 100.0) * available_width as f64) as u16;

        // Render the bar background
        if y < buf.area().height {
            for i in 0..available_width {
                let x = inner_area.x + i;
                if x < buf.area().width {
                    buf[(x, y)].set_char(' ');
                    if i < split_position {
                        buf[(x, y)].set_bg(bar_color);
                    } else if !self.transparent_background {
                        buf[(x, y)].set_bg(bar_bg_color);
                    }
                }
            }
        }

        // Render text centered on the bar
        if text_width > 0 && text_width <= available_width {
            let text_start_x = if self.text_align_left {
                inner_area.x
            } else {
                inner_area.x + (available_width.saturating_sub(text_width)) / 2
            };
            let text_fg_base = self.text_color.unwrap_or(Color::White);
            // Check contrast against ACTUAL background where text appears
            // When split_position == 0 (no visible bar), text is on bar_bg_color, not bar_color
            let contrast_bg = if split_position > 0 {
                Some(bar_color)
            } else if self.transparent_background {
                None // Skip contrast check - preserve original color
            } else {
                Some(bar_bg_color)
            };
            let text_fg = Self::ensure_contrast(text_fg_base, contrast_bg);

            for (i, c) in display_text.chars().enumerate() {
                let x = text_start_x + i as u16;
                if x < inner_area.x + inner_area.width && x < buf.area().width {
                    let char_position = x - inner_area.x;

                    buf[(x, y)].set_char(c);
                    buf[(x, y)].set_fg(text_fg);

                    if char_position < split_position {
                        buf[(x, y)].set_bg(bar_color);
                    } else if !self.transparent_background {
                        buf[(x, y)].set_bg(bar_bg_color);
                    }
                }
            }
        }
    }

}

/// Powerline half-circle end caps (Nerd Font, U+E0B6 / U+E0B4).
const PILL_LEFT: char = '\u{e0b6}';
const PILL_RIGHT: char = '\u{e0b4}';

/// Dim a color toward black for the unfilled pill track.
pub(crate) fn dim_color(c: Color) -> Color {
    match c {
        Color::Rgb(r, g, b) => Color::Rgb(r / 4, g / 4, b / 4),
        _ => Color::DarkGray,
    }
}

/// Draw a one-row pill-style bar: rounded caps colored to match the fill (or
/// track when empty/partial), the body as background fill with the text over
/// it. Caps leave the cell background untouched so the round ends blend with
/// whatever is behind the widget.
#[allow(clippy::too_many_arguments)]
pub(crate) fn render_pill_bar(
    buf: &mut Buffer,
    x: u16,
    y: u16,
    width: u16,
    fraction: f64,
    fill: Color,
    track: Color,
    text: &str,
    text_fg: Color,
    text_align_left: bool,
) {
    if width < 3 || y >= buf.area().height {
        return;
    }
    let bar_w = width - 2;
    let split = (fraction.clamp(0.0, 1.0) * bar_w as f64).round() as u16;

    let left_fg = if split > 0 { fill } else { track };
    let right_fg = if split >= bar_w { fill } else { track };
    if x < buf.area().width {
        buf[(x, y)].set_char(PILL_LEFT).set_fg(left_fg);
    }
    let rx = x + width - 1;
    if rx < buf.area().width {
        buf[(rx, y)].set_char(PILL_RIGHT).set_fg(right_fg);
    }

    for i in 0..bar_w {
        let cx = x + 1 + i;
        if cx >= buf.area().width {
            break;
        }
        buf[(cx, y)]
            .set_char(' ')
            .set_bg(if i < split { fill } else { track });
    }

    let tw = text.len() as u16;
    if tw > 0 && tw <= bar_w {
        let start = if text_align_left {
            x + 1
        } else {
            x + 1 + (bar_w - tw) / 2
        };
        for (i, c) in text.chars().enumerate() {
            let cx = start + i as u16;
            if cx >= buf.area().width || cx >= x + 1 + bar_w {
                break;
            }
            let filled = cx - (x + 1) < split;
            buf[(cx, y)]
                .set_char(c)
                .set_fg(text_fg)
                .set_bg(if filled { fill } else { track });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::BorderSides;

    fn buffer_line(buf: &Buffer, y: u16, width: u16) -> String {
        let mut line = String::new();
        for x in 0..width {
            line.push_str(buf[(x, y)].symbol());
        }
        line
    }

    #[test]
    fn draws_side_borders_when_only_left_right_enabled() {
        let mut bar = ProgressBar::new("Test");
        bar.set_border_config(
            true,
            Some("single".to_string()),
            Some("#ffffff".to_string()),
            BorderSides {
                left: true,
                right: true,
                top: false,
                bottom: false,
            },
        );

        let area = Rect::new(0, 0, 6, 1);
        let mut buf = Buffer::empty(area);
        bar.render(area, &mut buf);

        assert_eq!(buf[(0, 0)].symbol(), "│");
        assert_eq!(buf[(5, 0)].symbol(), "│");
    }

    #[test]
    fn renders_left_aligned_custom_text() {
        let mut bar = ProgressBar::new("");
        bar.set_border_config(false, None, None, BorderSides::default());
        bar.set_value_with_text(10, 100, Some("HP".to_string()));
        bar.set_text_align_left(true);

        let area = Rect::new(0, 0, 10, 1);
        let mut buf = Buffer::empty(area);
        bar.render(area, &mut buf);

        // Pill style: column 0 is the rounded cap, text starts at 1
        assert_eq!(buf[(1, 0)].symbol(), "H");
        assert_eq!(buf[(2, 0)].symbol(), "P");
    }

    #[test]
    fn renders_centered_text_by_default() {
        let mut bar = ProgressBar::new("");
        bar.set_border_config(false, None, None, BorderSides::default());
        bar.set_value_with_text(10, 100, Some("HP".to_string()));

        let area = Rect::new(0, 0, 10, 1);
        let mut buf = Buffer::empty(area);
        bar.render(area, &mut buf);

        let line = buffer_line(&buf, 0, area.width);
        assert_eq!(line.chars().nth(4).unwrap(), 'H');
        assert_eq!(line.chars().nth(5).unwrap(), 'P');
    }
}
