use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::{
    layout::{Constraint, Layout},
    DefaultTerminal, Frame,
};
use sqlx::PgPool;

use crate::errors::WisecrowError;
use crate::media::MediaContext;
use crate::srs::scheduler::ReviewRating;
use crate::srs::session::{Session, SessionManager};
use crate::tui::speed::SpeedController;
use crate::tui::widgets::card::CardWidget;
use crate::tui::widgets::stats::StatsWidget;

use super::TICK_RATE_MS;

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

        pub fn drain(&mut self) {
            while let Ok(result) = self.rx.try_recv() {
                match result {
                    #[cfg(feature = "audio")]
                    MediaResult::Audio(path) => self.audio_path = Some(path),
                    #[cfg(feature = "images")]
                    MediaResult::Image(protocol) => self.image_protocol = Some(protocol),
                }
            }
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
                    let api_key = api_key.expose().to_owned(); // expose: SecureString → String at point of HTTP use
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
    #[cfg(any(feature = "audio", feature = "images"))]
    media: Option<media_support::MediaState>,
}

impl App {
    fn new(pool: PgPool, session: Session, media_ctx: Option<MediaContext>) -> Self {
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
            #[cfg(any(feature = "audio", feature = "images"))]
            media,
        }
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

    fn drain_media(&mut self) {
        #[cfg(any(feature = "audio", feature = "images"))]
        if let Some(ref mut media) = self.media {
            media.drain();
        }
    }

    fn clear_media(&mut self) {
        #[cfg(any(feature = "audio", feature = "images"))]
        if let Some(ref mut media) = self.media {
            media.clear();
        }
    }

    fn play_audio_on_flip(&self) {
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
            self.play_audio_on_flip();
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
            KeyCode::Char(' ') if !self.flipped => {
                self.flipped = true;
                self.speed.reset();
                self.play_audio_on_flip();
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
    }
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
) -> Result<(), WisecrowError> {
    let mut terminal = ratatui::init();
    let result = run_app(&mut terminal, pool, session, media_ctx).await;
    ratatui::restore();
    result
}

async fn run_app(
    terminal: &mut DefaultTerminal,
    pool: PgPool,
    session: Session,
    media_ctx: Option<MediaContext>,
) -> Result<(), WisecrowError> {
    let mut app = App::new(pool, session, media_ctx);
    let tick_rate = Duration::from_millis(TICK_RATE_MS);

    app.fetch_media_for_current_card();

    loop {
        app.drain_media();
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

        let elapsed_ms = u32::try_from(start.elapsed().as_millis()).unwrap_or(u32::MAX);
        if app.speed.tick(elapsed_ms) {
            app.handle_timeout().await?;
        }
    }
}
