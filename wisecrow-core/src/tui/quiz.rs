use std::path::Path;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::{
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    DefaultTerminal, Frame,
};

use crate::errors::WisecrowError;
use crate::grammar::pdf;
use crate::grammar::quiz::{shuffle_options, ClozeQuiz, MultipleChoiceQuiz, QuizGenerator};

use super::TICK_RATE_MS;

#[derive(Debug, Clone)]
enum QuizItem {
    Cloze(ClozeQuiz),
    MultipleChoice(MultipleChoiceQuiz),
}

struct QuizApp {
    items: Vec<QuizItem>,
    current_index: usize,
    answered: bool,
    selected_option: Option<usize>,
    correct_count: usize,
    total_answered: usize,
    should_quit: bool,
    show_answer: bool,
}

impl QuizApp {
    fn new(items: Vec<QuizItem>) -> Self {
        Self {
            items,
            current_index: 0,
            answered: false,
            selected_option: None,
            correct_count: 0,
            total_answered: 0,
            should_quit: false,
            show_answer: false,
        }
    }

    fn current_item(&self) -> Option<&QuizItem> {
        self.items.get(self.current_index)
    }

    fn is_complete(&self) -> bool {
        self.current_index >= self.items.len()
    }

    fn handle_key(&mut self, key: KeyCode) {
        if self.is_complete() {
            if key == KeyCode::Char('q') {
                self.should_quit = true;
            }
            return;
        }

        match key {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char(' ') if self.answered => {
                self.current_index = self.current_index.saturating_add(1);
                self.answered = false;
                self.selected_option = None;
                self.show_answer = false;
            }
            KeyCode::Char('h') if !self.answered => {
                self.show_answer = true;
            }
            KeyCode::Char(c) if !self.answered => {
                if let Some(digit) = c.to_digit(10) {
                    let raw_idx = digit.saturating_sub(1);
                    let idx = usize::try_from(raw_idx).unwrap_or(usize::MAX);

                    let mc_info = if let Some(QuizItem::MultipleChoice(quiz)) = self.current_item()
                    {
                        if idx < quiz.options.len() {
                            Some(quiz.correct_index)
                        } else {
                            None
                        }
                    } else {
                        None
                    };

                    if let Some(correct_idx) = mc_info {
                        self.selected_option = Some(idx);
                        self.answered = true;
                        self.total_answered = self.total_answered.saturating_add(1);
                        if idx == correct_idx {
                            self.correct_count = self.correct_count.saturating_add(1);
                        }
                    }
                }
            }
            KeyCode::Enter if !self.answered => {
                let is_cloze = matches!(self.current_item(), Some(QuizItem::Cloze(_)));
                if is_cloze {
                    self.answered = true;
                    self.total_answered = self.total_answered.saturating_add(1);
                    self.show_answer = true;
                }
            }
            _ => {}
        }
    }

    fn draw_complete(&self, frame: &mut Frame<'_>, area: Rect) {
        let pct = if self.total_answered > 0 {
            self.correct_count
                .saturating_mul(100)
                .checked_div(self.total_answered)
                .unwrap_or(0)
        } else {
            0
        };
        let msg = format!(
            "Quiz complete! {}/{} correct ({}%). Press [q] to exit.",
            self.correct_count, self.total_answered, pct
        );
        let widget = Paragraph::new(msg).alignment(Alignment::Center);
        frame.render_widget(widget, area);
    }

    fn draw(&self, frame: &mut Frame<'_>) {
        let area = frame.area();

        if self.is_complete() {
            self.draw_complete(frame, area);
            return;
        }

        let chunks = Layout::vertical([
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(3),
        ])
        .split(area);

        let header = Paragraph::new(format!(
            "Question {} / {} | Score: {}/{}",
            self.current_index.saturating_add(1),
            self.items.len(),
            self.correct_count,
            self.total_answered,
        ))
        .alignment(Alignment::Center)
        .block(Block::default().borders(Borders::BOTTOM));
        frame.render_widget(header, chunks[0]);

        if let Some(item) = self.current_item() {
            match item {
                QuizItem::Cloze(cloze) => self.draw_cloze(frame, chunks[1], cloze),
                QuizItem::MultipleChoice(mc) => self.draw_mc(frame, chunks[1], mc),
            }
        }

        let footer_text = if self.answered {
            "[Space] Next question  [q] Quit"
        } else {
            "[h] Show hint  [Enter] Reveal answer  [q] Quit"
        };
        let footer = Paragraph::new(footer_text)
            .alignment(Alignment::Center)
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(footer, chunks[2]);
    }

    fn draw_cloze(&self, frame: &mut Frame<'_>, area: Rect, cloze: &ClozeQuiz) {
        let block = Block::default()
            .title(" Fill in the blank ")
            .borders(Borders::ALL);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let rows = Layout::vertical([
            Constraint::Length(2),
            Constraint::Length(2),
            Constraint::Min(1),
        ])
        .split(inner);

        let sentence = Paragraph::new(Line::from(Span::styled(
            &cloze.sentence_with_blank,
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )))
        .alignment(Alignment::Center);
        frame.render_widget(sentence, rows[0]);

        if self.show_answer || self.answered {
            let answer = Paragraph::new(Line::from(vec![
                Span::styled("Answer: ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    &cloze.answer,
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                ),
            ]))
            .alignment(Alignment::Center);
            frame.render_widget(answer, rows[1]);
        } else if let Some(hint) = &cloze.hint {
            let hint_line = Paragraph::new(Line::from(Span::styled(
                hint,
                Style::default().fg(Color::Yellow),
            )))
            .alignment(Alignment::Center);
            frame.render_widget(hint_line, rows[1]);
        }
    }

    fn draw_mc(&self, frame: &mut Frame<'_>, area: Rect, mc: &MultipleChoiceQuiz) {
        let block = Block::default()
            .title(" Multiple Choice ")
            .borders(Borders::ALL);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let mut constraints = vec![Constraint::Length(2)];
        for _ in &mc.options {
            constraints.push(Constraint::Length(2));
        }
        constraints.push(Constraint::Min(0));

        let rows = Layout::vertical(constraints).split(inner);

        let question = Paragraph::new(Line::from(Span::styled(
            &mc.question,
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )))
        .alignment(Alignment::Center);
        frame.render_widget(question, rows[0]);

        for (i, option) in mc.options.iter().enumerate() {
            let num = i.saturating_add(1);
            let (color, prefix) = if self.answered {
                if i == mc.correct_index {
                    (Color::Green, "✓")
                } else if self.selected_option == Some(i) {
                    (Color::Red, "✗")
                } else {
                    (Color::DarkGray, " ")
                }
            } else {
                (Color::White, " ")
            };

            let line = Paragraph::new(Line::from(Span::styled(
                format!("{prefix} [{num}] {option}"),
                Style::default().fg(color),
            )));
            frame.render_widget(line, rows[i.saturating_add(1)]);
        }
    }
}

/// Runs the quiz TUI for a given PDF file.
///
/// # Errors
///
/// Returns an error if the PDF cannot be parsed, quizzes cannot be
/// generated, or terminal operations fail.
pub fn run_quiz(pdf_path: &Path, num_questions: u32) -> Result<(), WisecrowError> {
    let content = pdf::extract(pdf_path)?;

    let cloze_quizzes = QuizGenerator::cloze_from_examples(
        &content
            .sections
            .iter()
            .flat_map(|s| s.examples.iter().cloned())
            .collect::<Vec<_>>(),
    );

    let mc_quizzes =
        QuizGenerator::multiple_choice_from_rules(&content.sections).unwrap_or_default();

    let mut items: Vec<QuizItem> = Vec::new();

    for (i, mc) in mc_quizzes.into_iter().enumerate() {
        items.push(QuizItem::MultipleChoice(shuffle_options(&mc, i)));
    }
    for cloze in cloze_quizzes {
        items.push(QuizItem::Cloze(cloze));
    }

    let limit = usize::try_from(num_questions).unwrap_or(usize::MAX);
    items.truncate(limit);

    if items.is_empty() {
        return Err(WisecrowError::QuizGenerationError(
            "No quizzes could be generated from the PDF content".to_owned(),
        ));
    }

    let mut terminal = ratatui::init();
    let result = run_quiz_app(&mut terminal, items);
    ratatui::restore();
    result
}

fn run_quiz_app(terminal: &mut DefaultTerminal, items: Vec<QuizItem>) -> Result<(), WisecrowError> {
    let mut app = QuizApp::new(items);
    let tick_rate = Duration::from_millis(TICK_RATE_MS);

    loop {
        terminal.draw(|frame| app.draw(frame))?;

        if app.should_quit {
            return Ok(());
        }

        if event::poll(tick_rate)? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    app.handle_key(key.code);
                }
            }
        }
    }
}
