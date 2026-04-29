use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Clear, Paragraph, Widget, Wrap},
};

#[derive(Debug, Clone)]
pub enum GlossDisplay<'a> {
    Loading,
    Error(&'a str),
    Ready(&'a str),
}

pub struct GlossModal<'a> {
    pub display: GlossDisplay<'a>,
}

impl<'a> Widget for GlossModal<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        Clear.render(area, buf);
        let title = " Gloss (g/Esc to close) ";
        let block = Block::default()
            .borders(Borders::ALL)
            .title(title)
            .title_style(
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            );
        let inner = block.inner(area);
        block.render(area, buf);
        let (text, style) = match self.display {
            GlossDisplay::Loading => (
                "Generating gloss…".to_owned(),
                Style::default().fg(Color::Gray),
            ),
            GlossDisplay::Error(e) => (format!("Error: {e}"), Style::default().fg(Color::Red)),
            GlossDisplay::Ready(g) => (g.to_owned(), Style::default()),
        };
        Paragraph::new(text)
            .style(style)
            .wrap(Wrap { trim: false })
            .render(inner, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_without_panic_for_each_state() {
        let area = Rect::new(0, 0, 40, 20);
        let mut buf = Buffer::empty(area);
        GlossModal {
            display: GlossDisplay::Loading,
        }
        .render(area, &mut buf);
        GlossModal {
            display: GlossDisplay::Error("boom"),
        }
        .render(area, &mut buf);
        GlossModal {
            display: GlossDisplay::Ready("line1\nline2\nline3\nline4"),
        }
        .render(area, &mut buf);
    }
}
