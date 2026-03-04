use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, Paragraph, Widget},
};

#[cfg(feature = "images")]
use ratatui::widgets::StatefulWidget;

use crate::srs::scheduler::CardState;

pub struct CardWidget<'a> {
    card: &'a CardState,
    flipped: bool,
    card_index: usize,
    total_cards: usize,
    timer_fraction: f64,
    speed_ms: u32,
    #[cfg(feature = "images")]
    image_state: Option<&'a mut Box<dyn ratatui_image::protocol::StatefulProtocol>>,
}

impl<'a> CardWidget<'a> {
    #[cfg(feature = "images")]
    pub fn new(
        card: &'a CardState,
        flipped: bool,
        card_index: usize,
        total_cards: usize,
        timer_fraction: f64,
        speed_ms: u32,
        image_state: Option<&'a mut Box<dyn ratatui_image::protocol::StatefulProtocol>>,
    ) -> Self {
        Self {
            card,
            flipped,
            card_index,
            total_cards,
            timer_fraction,
            speed_ms,
            image_state,
        }
    }

    #[cfg(not(feature = "images"))]
    pub const fn new(
        card: &'a CardState,
        flipped: bool,
        card_index: usize,
        total_cards: usize,
        timer_fraction: f64,
        speed_ms: u32,
    ) -> Self {
        Self {
            card,
            flipped,
            card_index,
            total_cards,
            timer_fraction,
            speed_ms,
        }
    }
}

impl Widget for CardWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(format!(
                " Card {} / {} ",
                self.card_index.saturating_add(1),
                self.total_cards
            ))
            .title_alignment(Alignment::Center);

        let inner = block.inner(area);
        block.render(area, buf);

        #[cfg(feature = "images")]
        let has_image = self.image_state.is_some();
        #[cfg(not(feature = "images"))]
        let has_image = false;

        let image_height = if has_image { 10 } else { 0 };

        let chunks = Layout::vertical([
            Constraint::Length(2),
            Constraint::Length(u16::try_from(image_height).unwrap_or(0)),
            Constraint::Min(3),
            Constraint::Length(2),
            Constraint::Length(3),
            Constraint::Length(1),
        ])
        .split(inner);

        let foreign_style = Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD);
        let foreign_line = Line::from(Span::styled(&self.card.to_phrase, foreign_style));
        Paragraph::new(foreign_line)
            .alignment(Alignment::Center)
            .render(chunks[0], buf);

        #[cfg(feature = "images")]
        if let Some(state) = self.image_state {
            let image_widget = ratatui_image::StatefulImage::new(None);
            image_widget.render(chunks[1], buf, state);
        }

        if self.flipped {
            let native_style = Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD);
            let native_line = Line::from(Span::styled(&self.card.from_phrase, native_style));
            Paragraph::new(native_line)
                .alignment(Alignment::Center)
                .render(chunks[2], buf);
        } else {
            let hint = Line::from(Span::styled(
                "[Space] to reveal",
                Style::default().fg(Color::DarkGray),
            ));
            Paragraph::new(hint)
                .alignment(Alignment::Center)
                .render(chunks[2], buf);
        }

        let timer_color = if self.timer_fraction > 0.5 {
            Color::Green
        } else if self.timer_fraction > 0.2 {
            Color::Yellow
        } else {
            Color::Red
        };

        Gauge::default()
            .gauge_style(Style::default().fg(timer_color))
            .ratio(self.timer_fraction)
            .label(format!("{:.1}s", f64::from(self.speed_ms) / 1000.0))
            .render(chunks[3], buf);

        if self.flipped {
            let rating_lines = vec![Line::from(vec![
                Span::styled("[1] Again  ", Style::default().fg(Color::Red)),
                Span::styled("[2] Hard   ", Style::default().fg(Color::Yellow)),
                Span::styled("[3] Good   ", Style::default().fg(Color::Green)),
                Span::styled("[4] Easy", Style::default().fg(Color::Cyan)),
            ])];
            Paragraph::new(rating_lines)
                .alignment(Alignment::Center)
                .render(chunks[4], buf);
        }

        let footer = Line::from(vec![
            Span::styled("[+/-] Speed  ", Style::default().fg(Color::DarkGray)),
            Span::styled("[p] Pause  ", Style::default().fg(Color::DarkGray)),
            Span::styled("[q] Quit", Style::default().fg(Color::DarkGray)),
        ]);
        Paragraph::new(footer)
            .alignment(Alignment::Center)
            .render(chunks[5], buf);
    }
}
