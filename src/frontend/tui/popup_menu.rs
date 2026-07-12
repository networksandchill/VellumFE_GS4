//! Shared popup menu used for contextual actions in the TUI.
//!
//! Provides keyboard navigation, click hit-testing, and theme-aware rendering.

use crate::frontend::tui::crossterm_bridge;
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Widget},
};

/// A menu item with display text and command to execute
#[derive(Clone, Debug)]
pub struct MenuItem {
    pub text: String,
}

/// Popup menu widget for navigable menus
pub struct PopupMenu {
    items: Vec<MenuItem>,
    selected: usize,
    position: (u16, u16), // (col, row)
}

impl PopupMenu {
    /// Create a new PopupMenu with a specific selected index
    pub fn with_selected(items: Vec<MenuItem>, position: (u16, u16), selected: usize) -> Self {
        Self {
            items,
            selected,
            position,
        }
    }

    /// Render the menu at its position
    pub fn render(&self, area: Rect, buf: &mut Buffer, theme: &crate::theme::AppTheme) {
        // Calculate menu dimensions
        let max_width = self
            .items
            .iter()
            .map(|item| item.text.len())
            .max()
            .unwrap_or(20)
            .min(60);

        let width = (max_width + 4) as u16; // +4 for borders and padding
        let height = (self.items.len() + 2) as u16; // +2 for borders

        // Position the menu
        let x = self.position.0.min(area.width.saturating_sub(width));
        let y = self.position.1.min(area.height.saturating_sub(height));

        let menu_rect = Rect {
            x,
            y,
            width,
            height,
        };

        // The corner cells keep the background that was underneath the menu:
        // filling them with the menu background puts a solid square behind
        // the rounded ╭╮╰╯ glyphs and reads as hard corners.
        let corners = [
            (menu_rect.x, menu_rect.y),
            (menu_rect.x + menu_rect.width - 1, menu_rect.y),
            (menu_rect.x, menu_rect.y + menu_rect.height - 1),
            (
                menu_rect.x + menu_rect.width - 1,
                menu_rect.y + menu_rect.height - 1,
            ),
        ];
        let saved_bg: Vec<_> = corners
            .iter()
            .map(|&(cx, cy)| {
                (cx < buf.area().width && cy < buf.area().height)
                    .then(|| buf[(cx, cy)].style().bg)
                    .flatten()
            })
            .collect();

        // Clear the area behind the menu
        Clear.render(menu_rect, buf);

        // Build menu lines (theme menu_* colors, not the browser palette)
        let mut lines = Vec::new();
        for (idx, item) in self.items.iter().enumerate() {
            let style = if idx == self.selected {
                Style::default()
                    .fg(crossterm_bridge::to_ratatui_color(theme.menu_item_selected))
                    .bg(crossterm_bridge::to_ratatui_color(theme.menu_item_focused))
            } else {
                Style::default()
                    .fg(crossterm_bridge::to_ratatui_color(theme.menu_item_normal))
                    .bg(crossterm_bridge::to_ratatui_color(theme.menu_background))
            };

            let line = Line::from(vec![
                Span::raw(" "),
                Span::styled(item.text.clone(), style),
                Span::raw(" "),
            ]);
            lines.push(line);
        }

        // Create block with rounded border
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(ratatui::widgets::BorderType::Rounded)
            .border_style(
                Style::default()
                    .fg(crossterm_bridge::to_ratatui_color(theme.menu_border))
                    .bg(crossterm_bridge::to_ratatui_color(theme.menu_background)),
            )
            .style(
                Style::default().bg(crossterm_bridge::to_ratatui_color(theme.menu_background)),
            );

        let paragraph = Paragraph::new(lines).block(block);

        ratatui::widgets::Widget::render(paragraph, menu_rect, buf);

        // Restore the underlying background outside the corner arcs.
        for (&(cx, cy), saved) in corners.iter().zip(saved_bg) {
            if cx < buf.area().width && cy < buf.area().height {
                match saved {
                    Some(bg) => {
                        buf[(cx, cy)].set_bg(bg);
                    }
                    None => {
                        buf[(cx, cy)].bg = ratatui::style::Color::Reset;
                    }
                }
            }
        }
    }
}
