use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Widget},
};

pub struct StatsWidget {
    cards_seen: usize,
    total_cards: usize,
    streak: usize,
    speed_ms: u32,
    paused: bool,
}

impl StatsWidget {
    pub const fn new(
        cards_seen: usize,
        total_cards: usize,
        streak: usize,
        speed_ms: u32,
        paused: bool,
    ) -> Self {
        Self {
            cards_seen,
            total_cards,
            streak,
            speed_ms,
            paused,
        }
    }
}

impl Widget for StatsWidget {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Session ")
            .title_alignment(Alignment::Center);

        let inner = block.inner(area);
        block.render(area, buf);

        let pct = if self.total_cards > 0 {
            (self.cards_seen * 100) / self.total_cards
        } else {
            0
        };

        let status = if self.paused { "PAUSED" } else { "ACTIVE" };
        let status_color = if self.paused {
            Color::Yellow
        } else {
            Color::Green
        };

        let lines = vec![
            Line::from(vec![
                Span::raw("Progress: "),
                Span::styled(
                    format!("{}/{} ({}%)", self.cards_seen, self.total_cards, pct),
                    Style::default().fg(Color::Cyan),
                ),
            ]),
            Line::from(vec![
                Span::raw("Streak:   "),
                Span::styled(
                    format!("{}", self.streak),
                    Style::default().fg(Color::Green),
                ),
            ]),
            Line::from(vec![
                Span::raw("Speed:    "),
                Span::styled(
                    format!("{:.1}s", f64::from(self.speed_ms) / 1000.0),
                    Style::default().fg(Color::White),
                ),
            ]),
            Line::from(vec![
                Span::raw("Status:   "),
                Span::styled(status, Style::default().fg(status_color)),
            ]),
        ];

        Paragraph::new(lines).render(inner, buf);
    }
}
