//! Pure Fast-mode playback reducer and a compile-safe play adapter.

/// DOM id of the single stable Fast-mode audio element.
pub const AUDIO_ELEMENT_ID: &str = "wisecrow-fast-audio";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunMode {
    AwaitingChoice,
    StartingSound,
    WithSound,
    WithoutSound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayStatus {
    Idle,
    Loading,
    Ready,
    Playing,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Playback {
    pub mode: RunMode,
    pub status: PlayStatus,
    pub source: Option<String>,
    pub timer_enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    AudioReady(String),
    AudioFailed,
    StartWithSound,
    StartWithoutSound,
    PlaySucceeded,
    PlayRejected,
    Retry,
    Pause,
    Resume,
    SourceChanged(Option<String>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Play(String),
    Pause,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayError {
    Unavailable,
    Rejected,
}

#[must_use]
pub fn classify_play_failure(element_missing: bool) -> PlayError {
    if element_missing {
        PlayError::Unavailable
    } else {
        PlayError::Rejected
    }
}

#[must_use]
pub fn initial() -> Playback {
    Playback {
        mode: RunMode::AwaitingChoice,
        status: PlayStatus::Idle,
        source: None,
        timer_enabled: false,
    }
}

#[must_use]
pub fn reduce(state: &Playback, event: Event) -> (Playback, Option<Command>) {
    match event {
        Event::AudioReady(source) => on_audio_ready(state, source),
        Event::AudioFailed => on_audio_failed(state),
        Event::StartWithSound => on_start_with_sound(state),
        Event::StartWithoutSound => on_start_without_sound(state),
        Event::PlaySucceeded => on_play_succeeded(state),
        Event::PlayRejected => on_play_rejected(state),
        Event::Retry => on_retry(state),
        Event::Pause => on_pause(state),
        Event::Resume => on_resume(state),
        Event::SourceChanged(source) => on_source_changed(state, source),
    }
}

fn on_audio_ready(state: &Playback, source: String) -> (Playback, Option<Command>) {
    let mut next = state.clone(); // clone: reducer returns a new state
    next.source = Some(source.clone()); // clone: command and state both need the URL
    next.status = PlayStatus::Ready;
    if state.mode == RunMode::StartingSound {
        next.status = PlayStatus::Loading;
        return (next, Some(Command::Play(source)));
    }
    if state.mode == RunMode::WithSound && state.timer_enabled {
        next.status = PlayStatus::Loading;
        return (next, Some(Command::Play(source)));
    }
    (next, None)
}

fn on_audio_failed(state: &Playback) -> (Playback, Option<Command>) {
    let mut next = state.clone(); // clone: reducer returns a new state
    next.status = PlayStatus::Failed;
    if state.mode == RunMode::StartingSound {
        next.timer_enabled = false;
    }
    (next, None)
}

fn on_start_with_sound(state: &Playback) -> (Playback, Option<Command>) {
    let mut next = state.clone(); // clone: reducer returns a new state
    next.mode = RunMode::StartingSound;
    next.timer_enabled = false;
    match state.source.clone() {
        // clone: command owns the URL while state retains it
        Some(source) => {
            next.status = PlayStatus::Loading;
            (next, Some(Command::Play(source)))
        }
        None => {
            next.status = PlayStatus::Loading;
            (next, None)
        }
    }
}

fn on_start_without_sound(state: &Playback) -> (Playback, Option<Command>) {
    let mut next = state.clone(); // clone: reducer returns a new state
    next.mode = RunMode::WithoutSound;
    next.timer_enabled = true;
    (next, None)
}

fn on_play_succeeded(state: &Playback) -> (Playback, Option<Command>) {
    let mut next = state.clone(); // clone: reducer returns a new state
    next.mode = RunMode::WithSound;
    next.status = PlayStatus::Playing;
    next.timer_enabled = true;
    (next, None)
}

fn on_play_rejected(state: &Playback) -> (Playback, Option<Command>) {
    let mut next = state.clone(); // clone: reducer returns a new state
    next.status = PlayStatus::Failed;
    next.timer_enabled = false;
    next.mode = RunMode::AwaitingChoice;
    (next, None)
}

fn on_retry(state: &Playback) -> (Playback, Option<Command>) {
    let mut next = state.clone(); // clone: reducer returns a new state
    next.mode = RunMode::StartingSound;
    next.timer_enabled = false;
    match state.source.clone() {
        // clone: retry replays the same URL
        Some(source) => {
            next.status = PlayStatus::Loading;
            (next, Some(Command::Play(source)))
        }
        None => {
            next.status = PlayStatus::Loading;
            (next, None)
        }
    }
}

fn on_pause(state: &Playback) -> (Playback, Option<Command>) {
    let mut next = state.clone(); // clone: reducer returns a new state
    next.timer_enabled = false;
    let command = (state.mode == RunMode::WithSound).then_some(Command::Pause);
    (next, command)
}

fn on_resume(state: &Playback) -> (Playback, Option<Command>) {
    let mut next = state.clone(); // clone: reducer returns a new state
    match state.mode {
        RunMode::WithSound => {
            next.timer_enabled = true;
            let command = state.source.clone().map(Command::Play); // clone: resume replays current URL
            (next, command)
        }
        RunMode::WithoutSound => {
            next.timer_enabled = true;
            (next, None)
        }
        RunMode::AwaitingChoice | RunMode::StartingSound => (next, None),
    }
}

fn on_source_changed(state: &Playback, source: Option<String>) -> (Playback, Option<Command>) {
    if state.source == source {
        return (state.clone(), None); // clone: reducer returns a new state
    }
    let mut next = state.clone(); // clone: reducer returns a new state
    next.source = source.clone(); // clone: optional command needs the same URL
    match (state.mode, source) {
        (RunMode::WithSound | RunMode::StartingSound, Some(source)) => {
            next.status = PlayStatus::Loading;
            (next, Some(Command::Play(source)))
        }
        (RunMode::WithSound | RunMode::StartingSound, None) => {
            next.status = PlayStatus::Loading;
            (next, None)
        }
        (_, Some(_)) => {
            next.status = PlayStatus::Ready;
            (next, None)
        }
        (_, None) => {
            if next.status != PlayStatus::Failed {
                next.status = PlayStatus::Idle;
            }
            (next, None)
        }
    }
}

/// Looks up the stable audio element and starts playback. Native/server builds
/// are a no-op so the adapter compiles without a browser.
pub async fn play_source(src: &str) -> Result<(), PlayError> {
    play_source_impl(src).await
}

pub fn pause_element() {
    pause_element_impl();
}

#[cfg(target_arch = "wasm32")]
async fn play_source_impl(src: &str) -> Result<(), PlayError> {
    use wasm_bindgen::JsCast;

    let window = web_sys::window().ok_or(classify_play_failure(true))?;
    let document = window.document().ok_or(classify_play_failure(true))?;
    let element = document
        .get_element_by_id(AUDIO_ELEMENT_ID)
        .ok_or(classify_play_failure(true))?;
    let audio: web_sys::HtmlAudioElement = element
        .dyn_into()
        .map_err(|_| classify_play_failure(true))?;
    audio.set_src(src);
    let promise = audio.play().map_err(|_| classify_play_failure(false))?;
    wasm_bindgen_futures::JsFuture::from(promise)
        .await
        .map(|_| ())
        .map_err(|_| classify_play_failure(false))
}

#[cfg(not(target_arch = "wasm32"))]
async fn play_source_impl(_src: &str) -> Result<(), PlayError> {
    Err(classify_play_failure(true))
}

#[cfg(target_arch = "wasm32")]
fn pause_element_impl() {
    use wasm_bindgen::JsCast;

    let Some(window) = web_sys::window() else {
        return;
    };
    let Some(document) = window.document() else {
        return;
    };
    let Some(element) = document.get_element_by_id(AUDIO_ELEMENT_ID) else {
        return;
    };
    let Ok(audio) = element.dyn_into::<web_sys::HtmlAudioElement>() else {
        return;
    };
    if let Err(error) = audio.pause() {
        tracing::warn!(?error, "failed to pause fast audio element");
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn pause_element_impl() {}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready() -> Playback {
        let (state, _) = reduce(&initial(), Event::AudioReady("clip-1".to_owned()));
        state
    }

    #[test]
    fn ready_start_succeeds_and_enables_timer() {
        let state = ready();
        let (starting, command) = reduce(&state, Event::StartWithSound);
        assert_eq!(command, Some(Command::Play("clip-1".to_owned())));
        assert!(!starting.timer_enabled);
        let (playing, _) = reduce(&starting, Event::PlaySucceeded);
        assert!(playing.timer_enabled);
        assert_eq!(playing.mode, RunMode::WithSound);
    }

    #[test]
    fn delayed_first_audio_plays_after_start() {
        let (starting, command) = reduce(&initial(), Event::StartWithSound);
        assert_eq!(command, None);
        assert!(!starting.timer_enabled);
        let (loading, command) = reduce(&starting, Event::AudioReady("clip-1".to_owned()));
        assert_eq!(command, Some(Command::Play("clip-1".to_owned())));
        assert!(!loading.timer_enabled);
    }

    #[test]
    fn rejected_play_stays_without_timer() {
        let state = ready();
        let (starting, _) = reduce(&state, Event::StartWithSound);
        let (failed, command) = reduce(&starting, Event::PlayRejected);
        assert_eq!(command, None);
        assert!(!failed.timer_enabled);
        assert_eq!(failed.status, PlayStatus::Failed);
        assert_eq!(failed.mode, RunMode::AwaitingChoice);
    }

    #[test]
    fn retry_after_rejection_can_succeed() {
        let state = ready();
        let (starting, _) = reduce(&state, Event::StartWithSound);
        let (failed, _) = reduce(&starting, Event::PlayRejected);
        let (retrying, command) = reduce(&failed, Event::Retry);
        assert_eq!(command, Some(Command::Play("clip-1".to_owned())));
        let (playing, _) = reduce(&retrying, Event::PlaySucceeded);
        assert!(playing.timer_enabled);
    }

    #[test]
    fn start_without_sound_enables_timer() {
        let (state, command) = reduce(&initial(), Event::StartWithoutSound);
        assert_eq!(command, None);
        assert!(state.timer_enabled);
        assert_eq!(state.mode, RunMode::WithoutSound);
    }

    #[test]
    fn pause_and_resume_toggle_timer_and_play() {
        let state = ready();
        let (starting, _) = reduce(&state, Event::StartWithSound);
        let (playing, _) = reduce(&starting, Event::PlaySucceeded);
        let (paused, command) = reduce(&playing, Event::Pause);
        assert_eq!(command, Some(Command::Pause));
        assert!(!paused.timer_enabled);
        let (resumed, command) = reduce(&paused, Event::Resume);
        assert_eq!(command, Some(Command::Play("clip-1".to_owned())));
        assert!(resumed.timer_enabled);
    }

    #[test]
    fn source_change_replays_through_unlocked_element() {
        let state = ready();
        let (starting, _) = reduce(&state, Event::StartWithSound);
        let (playing, _) = reduce(&starting, Event::PlaySucceeded);
        let (next, command) = reduce(&playing, Event::SourceChanged(Some("clip-2".to_owned())));
        assert_eq!(command, Some(Command::Play("clip-2".to_owned())));
        assert_eq!(next.source.as_deref(), Some("clip-2"));
    }

    #[test]
    fn identical_source_change_does_not_replay() {
        let state = ready();
        let (starting, _) = reduce(&state, Event::StartWithSound);
        let (playing, _) = reduce(&starting, Event::PlaySucceeded);
        let (same, command) = reduce(&playing, Event::SourceChanged(Some("clip-1".to_owned())));
        assert_eq!(command, None);
        assert_eq!(same.status, PlayStatus::Playing);
    }

    #[test]
    fn play_errors_are_distinct() {
        assert_ne!(PlayError::Unavailable, PlayError::Rejected);
    }

    #[test]
    fn timer_does_not_advance_before_start_choice() {
        let initial = initial();
        assert!(!initial.timer_enabled);
        let (ready_state, _) = reduce(&initial, Event::AudioReady("clip-1".to_owned()));
        assert!(!ready_state.timer_enabled);
        let (failed, _) = reduce(&ready_state, Event::AudioFailed);
        assert!(!failed.timer_enabled);
    }
}
