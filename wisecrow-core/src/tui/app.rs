use std::sync::Arc;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::{
    layout::{Constraint, Layout},
    DefaultTerminal, Frame,
};
use sqlx::PgPool;
use tokio::sync::mpsc;

use crate::errors::WisecrowError;
use crate::llm::LlmProvider;
use crate::media::MediaContext;
use crate::srs::scheduler::ReviewRating;
use crate::srs::session::{Session, SessionManager};
use crate::tui::speed::SpeedController;
use crate::tui::widgets::card::CardWidget;
use crate::tui::widgets::gloss::{GlossDisplay, GlossModal};
use crate::tui::widgets::stats::StatsWidget;

use super::TICK_RATE_MS;

pub struct GlossContext {
    pub provider: Arc<dyn LlmProvider>,
    pub pool: PgPool,
}

#[derive(Debug)]
pub enum GlossOutcome {
    Ready(String),
    Error(String),
}

pub struct GlossState {
    ctx: Option<Arc<GlossContext>>,
    rx: Option<mpsc::UnboundedReceiver<GlossOutcome>>,
    tx_template: Option<mpsc::UnboundedSender<GlossOutcome>>,
    current: Option<GlossOutcome>,
    loading: bool,
}

impl GlossState {
    #[must_use]
    pub fn new() -> Self {
        Self {
            ctx: None,
            rx: None,
            tx_template: None,
            current: None,
            loading: false,
        }
    }

    #[must_use]
    pub fn with_ctx(ctx: GlossContext) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self {
            ctx: Some(Arc::new(ctx)),
            rx: Some(rx),
            tx_template: Some(tx),
            current: None,
            loading: false,
        }
    }

    #[must_use]
    pub fn is_loading(&self) -> bool {
        self.loading
    }

    #[must_use]
    pub fn current(&self) -> Option<&GlossOutcome> {
        self.current.as_ref()
    }

    pub fn fetch(&mut self, sentence: String, lang_code: String, lang_name: String) {
        let (Some(ctx), Some(tx)) = (self.ctx.as_ref(), self.tx_template.as_ref()) else {
            return;
        };
        self.loading = true;
        self.current = None;
        let ctx = Arc::clone(ctx); // clone: Arc shared into spawned task
        let tx = tx.clone(); // clone: sender into spawned task
        tokio::spawn(async move {
            let result = crate::grammar::gloss::generate_or_lookup(
                &ctx.pool,
                ctx.provider.as_ref(),
                &sentence,
                &lang_code,
                &lang_name,
            )
            .await;
            let outcome = match result {
                Ok(g) => GlossOutcome::Ready(g),
                Err(e) => GlossOutcome::Error(e.to_string()),
            };
            if let Err(e) = tx.send(outcome) {
                tracing::warn!("Failed to send gloss outcome: {e}");
            }
        });
    }

    pub fn drain(&mut self) -> bool {
        let Some(rx) = self.rx.as_mut() else {
            return false;
        };
        let mut changed = false;
        while let Ok(outcome) = rx.try_recv() {
            self.current = Some(outcome);
            self.loading = false;
            changed = true;
        }
        changed
    }

    pub fn clear(&mut self) {
        self.current = None;
        self.loading = false;
    }
}

impl Default for GlossState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(any(feature = "audio", feature = "images"))]
mod media_support {
    #[cfg(feature = "audio")]
    use std::path::PathBuf;
    use std::sync::Arc;

    use tokio::sync::mpsc;

    use crate::media::MediaContext;

    pub enum MediaResult {
        #[cfg(feature = "audio")]
        Audio(PathBuf),
        #[cfg(feature = "images")]
        Image(Box<dyn ratatui_image::protocol::StatefulProtocol>),
    }

    pub struct MediaState {
        pub ctx: Arc<MediaContext>,
        pub rx: mpsc::UnboundedReceiver<MediaResult>,
        pub tx: mpsc::UnboundedSender<MediaResult>,
        #[cfg(feature = "audio")]
        pub audio_path: Option<PathBuf>,
        #[cfg(feature = "images")]
        pub image_protocol: Option<Box<dyn ratatui_image::protocol::StatefulProtocol>>,
    }

    impl MediaState {
        pub fn new(ctx: MediaContext) -> Self {
            let (tx, rx) = mpsc::unbounded_channel();
            Self {
                ctx: Arc::new(ctx),
                rx,
                tx,
                #[cfg(feature = "audio")]
                audio_path: None,
                #[cfg(feature = "images")]
                image_protocol: None,
            }
        }

        /// Drains pending media results. Returns `true` if new audio arrived
        /// during this drain (used to trigger auto-play).
        pub fn drain(&mut self) -> bool {
            let mut new_audio = false;
            while let Ok(result) = self.rx.try_recv() {
                match result {
                    #[cfg(feature = "audio")]
                    MediaResult::Audio(path) => {
                        let had_audio = self.audio_path.is_some();
                        self.audio_path = Some(path);
                        if !had_audio {
                            new_audio = true;
                        }
                    }
                    #[cfg(feature = "images")]
                    MediaResult::Image(protocol) => self.image_protocol = Some(protocol),
                }
            }
            new_audio
        }

        pub fn clear(&mut self) {
            #[cfg(feature = "audio")]
            {
                self.audio_path = None;
            }
            #[cfg(feature = "images")]
            {
                self.image_protocol = None;
            }
        }

        pub fn fetch_for_card(&self, translation_id: i32, to_phrase: &str) {
            let to_phrase = to_phrase.to_owned();
            let ctx = Arc::clone(&self.ctx); // clone: Arc shared ownership for async task
            let tx = self.tx.clone(); // clone: sender for async task

            tokio::spawn(async move {
                #[cfg(feature = "audio")]
                {
                    let lang = ctx.foreign_lang.clone(); // clone: need owned copy for closure
                    let word = to_phrase.clone(); // clone: need owned copy for closure
                    let result = ctx
                        .cache
                        .get_or_fetch(translation_id, crate::media::MediaType::Audio, || {
                            crate::media::audio::generate_tts(&word, &lang)
                        })
                        .await;
                    match result {
                        Ok(path) => {
                            if let Err(e) = tx.send(MediaResult::Audio(path)) {
                                tracing::warn!("Failed to send audio result: {e}");
                            }
                        }
                        Err(e) => tracing::debug!("Audio fetch failed: {e}"),
                    }
                }

                #[cfg(feature = "images")]
                {
                    let Some(ref api_key) = ctx.unsplash_api_key else {
                        return;
                    };
                    let client = ctx.http_client.clone(); // clone: reqwest::Client is Arc-based
                    let api_key = api_key.expose().to_owned(); // expose: SecureString -> String at point of HTTP use
                    let word = to_phrase;
                    let result = ctx
                        .cache
                        .get_or_fetch(translation_id, crate::media::MediaType::Image, || async {
                            crate::media::images::fetch_image(&client, &word, &api_key).await
                        })
                        .await;
                    match result {
                        Ok(path) => match crate::media::images::load_image_for_display(&path) {
                            Ok(protocol) => {
                                if let Err(e) = tx.send(MediaResult::Image(protocol)) {
                                    tracing::warn!("Failed to send image result: {e}");
                                }
                            }
                            Err(e) => tracing::debug!("Image display load failed: {e}"),
                        },
                        Err(e) => tracing::debug!("Image fetch failed: {e}"),
                    }
                }
            });
        }

        pub fn play_audio(&self) {
            #[cfg(feature = "audio")]
            if let Some(ref path) = self.audio_path {
                if let Err(e) = crate::media::audio::play_audio(path) {
                    tracing::debug!("Audio playback failed: {e}");
                }
            }
        }
    }
}

struct App {
    pool: PgPool,
    session: Session,
    current_index: usize,
    flipped: bool,
    speed: SpeedController,
    streak: usize,
    should_quit: bool,
    foreign_lang_name: String,
    gloss_state: GlossState,
    gloss_modal_open: bool,
    #[cfg(any(feature = "audio", feature = "images"))]
    media: Option<media_support::MediaState>,
}

impl App {
    fn new(
        pool: PgPool,
        session: Session,
        media_ctx: Option<MediaContext>,
        foreign_lang_name: String,
    ) -> Self {
        let speed_ms = u32::try_from(session.speed_ms).unwrap_or(3000);
        let current_index = usize::try_from(session.current_index).unwrap_or(0);
        #[cfg(any(feature = "audio", feature = "images"))]
        let media = media_ctx.map(media_support::MediaState::new);
        #[cfg(not(any(feature = "audio", feature = "images")))]
        let _ = media_ctx;
        Self {
            pool,
            session,
            current_index,
            flipped: false,
            speed: SpeedController::new(speed_ms),
            streak: 0,
            should_quit: false,
            foreign_lang_name,
            gloss_state: GlossState::new(),
            gloss_modal_open: false,
            #[cfg(any(feature = "audio", feature = "images"))]
            media,
        }
    }

    fn with_gloss_ctx(mut self, ctx: GlossContext) -> Self {
        self.gloss_state = GlossState::with_ctx(ctx);
        self
    }

    fn is_session_complete(&self) -> bool {
        self.current_index >= self.session.cards.len()
    }

    fn fetch_media_for_current_card(&self) {
        #[cfg(any(feature = "audio", feature = "images"))]
        if let Some(ref media) = self.media {
            if let Some(card) = self.session.cards.get(self.current_index) {
                media.fetch_for_card(card.translation_id, &card.to_phrase);
            }
        }
    }

    /// Drains pending media results. Returns `true` if new audio arrived.
    fn drain_media(&mut self) -> bool {
        #[cfg(any(feature = "audio", feature = "images"))]
        if let Some(ref mut media) = self.media {
            return media.drain();
        }
        false
    }

    fn clear_media(&mut self) {
        #[cfg(any(feature = "audio", feature = "images"))]
        if let Some(ref mut media) = self.media {
            media.clear();
        }
    }

    fn play_audio(&self) {
        #[cfg(any(feature = "audio", feature = "images"))]
        if let Some(ref media) = self.media {
            media.play_audio();
        }
    }

    async fn handle_rating(&mut self, rating: ReviewRating) -> Result<(), WisecrowError> {
        if !self.flipped {
            return Ok(());
        }

        let Some(card) = self.session.cards.get(self.current_index).cloned() else {
            // clone: need owned copy because answer_card takes &CardState while we mutate self
            return Ok(());
        };

        SessionManager::answer_card(&self.pool, self.session.id, &card, rating).await?;

        if rating == ReviewRating::Again {
            self.streak = 0;
        } else {
            self.streak = self.streak.saturating_add(1);
        }

        self.current_index = self.current_index.saturating_add(1);
        self.flipped = false;
        self.speed.reset();
        self.clear_media();
        self.fetch_media_for_current_card();

        if self.is_session_complete() {
            SessionManager::complete(&self.pool, self.session.id).await?;
        }

        Ok(())
    }

    async fn handle_timeout(&mut self) -> Result<(), WisecrowError> {
        if self.flipped {
            self.handle_rating(ReviewRating::Again).await?;
        } else {
            self.flipped = true;
            self.speed.reset();
            self.play_audio();
        }
        Ok(())
    }

    async fn handle_key(&mut self, key: KeyCode) -> Result<(), WisecrowError> {
        match key {
            KeyCode::Char('q') => {
                if !self.is_session_complete() {
                    SessionManager::pause(&self.pool, self.session.id).await?;
                }
                self.should_quit = true;
            }
            KeyCode::Char('g') => self.toggle_gloss_modal(),
            KeyCode::Esc if self.gloss_modal_open => self.close_gloss_modal(),
            KeyCode::Char(' ') if !self.flipped => {
                self.flipped = true;
                self.speed.reset();
                self.play_audio();
            }
            KeyCode::Char('1') => self.handle_rating(ReviewRating::Again).await?,
            KeyCode::Char('2') => self.handle_rating(ReviewRating::Hard).await?,
            KeyCode::Char('3') => self.handle_rating(ReviewRating::Good).await?,
            KeyCode::Char('4') => self.handle_rating(ReviewRating::Easy).await?,
            KeyCode::Char('+') | KeyCode::Char('=') => self.speed.slow_down(),
            KeyCode::Char('-') => self.speed.speed_up(),
            KeyCode::Char('p') => {
                if self.speed.is_paused() {
                    self.speed.unpause();
                } else {
                    self.speed.pause();
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn toggle_gloss_modal(&mut self) {
        self.gloss_modal_open = !self.gloss_modal_open;
        if self.gloss_modal_open {
            if let Some(card) = self.session.cards.get(self.current_index) {
                self.gloss_state.fetch(
                    card.to_phrase.clone(),            // clone: owned arg for spawned task
                    self.session.foreign_lang.clone(), // clone: owned arg for spawned task
                    self.foreign_lang_name.clone(),    // clone: owned arg for spawned task
                );
            }
        } else {
            self.gloss_state.clear();
        }
    }

    fn close_gloss_modal(&mut self) {
        self.gloss_modal_open = false;
        self.gloss_state.clear();
    }

    fn tick_auto_advance(&mut self, elapsed: Duration) -> bool {
        if self.gloss_modal_open {
            return false;
        }
        let elapsed_ms = u32::try_from(elapsed.as_millis()).unwrap_or(u32::MAX);
        self.speed.tick(elapsed_ms)
    }

    fn draw(&mut self, frame: &mut Frame<'_>) {
        let area = frame.area();

        if self.is_session_complete() {
            let msg = ratatui::widgets::Paragraph::new(format!(
                "Session complete! {} cards reviewed. Press [q] to exit.",
                self.session.cards.len()
            ))
            .alignment(ratatui::layout::Alignment::Center);
            frame.render_widget(msg, area);
            return;
        }

        let chunks = Layout::horizontal([Constraint::Min(40), Constraint::Length(30)]).split(area);

        let card_index = self.current_index;
        let total_cards = self.session.cards.len();
        let flipped = self.flipped;
        let timer_fraction = self.speed.remaining_fraction();
        let speed_ms = self.speed.interval_ms();

        if let Some(card) = self.session.cards.get(card_index) {
            #[cfg(feature = "images")]
            let image_state = self.media.as_mut().and_then(|m| m.image_protocol.as_mut());

            #[cfg(feature = "images")]
            let card_widget = CardWidget::new(
                card,
                flipped,
                card_index,
                total_cards,
                timer_fraction,
                speed_ms,
                image_state,
            );
            #[cfg(not(feature = "images"))]
            let card_widget = CardWidget::new(
                card,
                flipped,
                card_index,
                total_cards,
                timer_fraction,
                speed_ms,
            );
            frame.render_widget(card_widget, chunks[0]);
        }

        let stats = StatsWidget::new(
            card_index,
            total_cards,
            self.streak,
            speed_ms,
            self.speed.is_paused(),
        );
        frame.render_widget(stats, chunks[1]);

        if self.gloss_modal_open {
            let modal_area = centered_rect(80, 60, area);
            let display = match self.gloss_state.current() {
                Some(GlossOutcome::Ready(g)) => GlossDisplay::Ready(g),
                Some(GlossOutcome::Error(e)) => GlossDisplay::Error(e),
                None => GlossDisplay::Loading,
            };
            frame.render_widget(GlossModal { display }, modal_area);
        }
    }
}

fn centered_rect(
    percent_x: u16,
    percent_y: u16,
    area: ratatui::layout::Rect,
) -> ratatui::layout::Rect {
    use ratatui::layout::{Constraint, Direction, Layout};
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

/// Runs the TUI event loop for a learning session.
///
/// # Errors
///
/// Returns an error if terminal setup/restore fails or a database operation
/// fails during the session.
pub async fn run_tui(
    pool: PgPool,
    session: Session,
    media_ctx: Option<MediaContext>,
    gloss_ctx: Option<GlossContext>,
    foreign_lang_name: String,
) -> Result<(), WisecrowError> {
    let mut terminal = ratatui::init();
    let result = run_app(
        &mut terminal,
        pool,
        session,
        media_ctx,
        gloss_ctx,
        foreign_lang_name,
    )
    .await;
    ratatui::restore();
    result
}

async fn run_app(
    terminal: &mut DefaultTerminal,
    pool: PgPool,
    session: Session,
    media_ctx: Option<MediaContext>,
    gloss_ctx: Option<GlossContext>,
    foreign_lang_name: String,
) -> Result<(), WisecrowError> {
    let mut app = App::new(pool, session, media_ctx, foreign_lang_name);
    if let Some(ctx) = gloss_ctx {
        app = app.with_gloss_ctx(ctx);
    }
    let tick_rate = Duration::from_millis(TICK_RATE_MS);

    app.fetch_media_for_current_card();

    loop {
        let audio_arrived = app.drain_media();
        if audio_arrived {
            app.play_audio();
        }

        app.gloss_state.drain();

        terminal.draw(|frame| app.draw(frame))?;

        if app.should_quit {
            return Ok(());
        }

        let start = Instant::now();
        let timeout = tick_rate;

        if event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    app.handle_key(key.code).await?;
                }
            }
        }

        if app.tick_auto_advance(start.elapsed()) {
            app.handle_timeout().await?;
        }
    }
}

#[cfg(test)]
mod gloss_state_tests {
    use super::*;
    use crate::srs::scheduler::{CardState, CardStatus};
    use chrono::Utc;
    use sqlx::postgres::PgPoolOptions;

    fn make_session_with_card() -> Session {
        Session {
            id: 1,
            native_lang: "en".to_owned(),
            foreign_lang: "ru".to_owned(),
            deck_size: 1,
            speed_ms: 3000,
            current_index: 0,
            cards: vec![CardState {
                card_id: 1,
                translation_id: 1,
                from_phrase: "my name is Ivan".to_owned(),
                to_phrase: "Меня зовут Иван".to_owned(),
                frequency: 100,
                stability: 0.0,
                difficulty: 0.0,
                state: CardStatus::New,
                due: Utc::now(),
                reps: 0,
                lapses: 0,
            }],
        }
    }

    fn lazy_pool() -> PgPool {
        PgPoolOptions::new()
            .connect_lazy("postgres://test:test@127.0.0.1:5432/test")
            .expect("lazy pool construction")
    }

    impl App {
        fn test_only_minimal() -> Self {
            App::new(
                lazy_pool(),
                make_session_with_card(),
                None,
                "Russian".to_owned(),
            )
        }
    }

    #[tokio::test]
    async fn gloss_state_starts_idle() {
        let state = GlossState::new();
        assert!(!state.is_loading());
        assert!(state.current().is_none());
    }

    #[tokio::test]
    async fn g_keypress_toggles_modal() {
        let mut app = App::test_only_minimal();
        assert!(!app.gloss_modal_open);
        app.handle_key(KeyCode::Char('g')).await.unwrap();
        assert!(app.gloss_modal_open);
        app.handle_key(KeyCode::Char('g')).await.unwrap();
        assert!(!app.gloss_modal_open);
    }

    #[tokio::test]
    async fn esc_closes_modal_when_open() {
        let mut app = App::test_only_minimal();
        app.gloss_modal_open = true;
        app.handle_key(KeyCode::Esc).await.unwrap();
        assert!(!app.gloss_modal_open);
    }

    #[tokio::test]
    async fn modal_open_blocks_auto_advance() {
        let mut app = App::test_only_minimal();
        app.gloss_modal_open = true;
        let advanced = app.tick_auto_advance(Duration::from_secs(10));
        assert!(!advanced, "auto-advance must not fire while modal is open");
    }

    #[tokio::test]
    async fn auto_advance_fires_when_modal_closed_and_timer_expires() {
        let mut app = App::test_only_minimal();
        let advanced = app.tick_auto_advance(Duration::from_secs(10));
        assert!(
            advanced,
            "auto-advance fires past speed_ms when modal is closed"
        );
    }
}
