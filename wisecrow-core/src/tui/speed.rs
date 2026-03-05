const MIN_SPEED_MS: u32 = 500;
const MAX_SPEED_MS: u32 = 10_000;
const SPEED_STEP_MS: u32 = 500;

#[derive(Debug)]
pub struct SpeedController {
    interval_ms: u32,
    remaining_ms: u32,
    paused: bool,
}

impl SpeedController {
    #[must_use]
    pub fn new(interval_ms: u32) -> Self {
        let clamped = interval_ms.clamp(MIN_SPEED_MS, MAX_SPEED_MS);
        Self {
            interval_ms: clamped,
            remaining_ms: clamped,
            paused: false,
        }
    }

    /// Advances the timer by `elapsed_ms`. Returns `true` if the timer
    /// expired (user ran out of time).
    pub fn tick(&mut self, elapsed_ms: u32) -> bool {
        if self.paused {
            return false;
        }
        self.remaining_ms = self.remaining_ms.saturating_sub(elapsed_ms);
        self.remaining_ms == 0
    }

    pub fn reset(&mut self) {
        self.remaining_ms = self.interval_ms;
    }

    pub fn speed_up(&mut self) {
        self.interval_ms = self
            .interval_ms
            .saturating_sub(SPEED_STEP_MS)
            .max(MIN_SPEED_MS);
    }

    pub fn slow_down(&mut self) {
        self.interval_ms = self
            .interval_ms
            .saturating_add(SPEED_STEP_MS)
            .min(MAX_SPEED_MS);
    }

    pub fn pause(&mut self) {
        self.paused = true;
    }

    pub fn unpause(&mut self) {
        self.paused = false;
    }

    #[must_use]
    pub const fn is_paused(&self) -> bool {
        self.paused
    }

    /// Returns 0.0 (expired) to 1.0 (full time remaining).
    #[must_use]
    pub fn remaining_fraction(&self) -> f64 {
        if self.interval_ms == 0 {
            return 0.0;
        }
        f64::from(self.remaining_ms) / f64::from(self.interval_ms)
    }

    #[must_use]
    pub const fn interval_ms(&self) -> u32 {
        self.interval_ms
    }

    #[must_use]
    pub const fn remaining_ms(&self) -> u32 {
        self.remaining_ms
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case(100, MIN_SPEED_MS)]
    #[case(20_000, MAX_SPEED_MS)]
    #[case(3000, 3000)]
    fn new_clamps_to_bounds(#[case] input: u32, #[case] expected: u32) {
        let sc = SpeedController::new(input);
        assert_eq!(sc.interval_ms(), expected);
    }

    #[rstest]
    #[case(3000, 1000, false, 2000)]
    #[case(1000, 2000, true, 0)]
    fn tick_behavior(
        #[case] interval: u32,
        #[case] elapsed: u32,
        #[case] expired: bool,
        #[case] remaining: u32,
    ) {
        let mut sc = SpeedController::new(interval);
        assert_eq!(sc.tick(elapsed), expired);
        assert_eq!(sc.remaining_ms(), remaining);
    }

    #[test]
    fn tick_expires_exactly_at_zero() {
        let mut sc = SpeedController::new(1000);
        assert!(!sc.tick(999));
        assert!(sc.tick(1));
    }

    #[test]
    fn reset_restores_full_time() {
        let mut sc = SpeedController::new(3000);
        sc.tick(2000);
        sc.reset();
        assert_eq!(sc.remaining_ms(), 3000);
    }

    #[rstest]
    #[case(3000, 2500)]
    #[case(MIN_SPEED_MS, MIN_SPEED_MS)]
    fn speed_up_clamps(#[case] start: u32, #[case] expected: u32) {
        let mut sc = SpeedController::new(start);
        sc.speed_up();
        assert_eq!(sc.interval_ms(), expected);
    }

    #[rstest]
    #[case(3000, 3500)]
    #[case(MAX_SPEED_MS, MAX_SPEED_MS)]
    fn slow_down_clamps(#[case] start: u32, #[case] expected: u32) {
        let mut sc = SpeedController::new(start);
        sc.slow_down();
        assert_eq!(sc.interval_ms(), expected);
    }

    #[test]
    fn pause_prevents_tick() {
        let mut sc = SpeedController::new(3000);
        sc.pause();
        assert!(!sc.tick(5000));
        assert_eq!(sc.remaining_ms(), 3000);
    }

    #[test]
    fn unpause_resumes_tick() {
        let mut sc = SpeedController::new(3000);
        sc.pause();
        sc.tick(1000);
        sc.unpause();
        assert!(!sc.tick(1000));
        assert_eq!(sc.remaining_ms(), 2000);
    }

    #[rstest]
    #[case(3000, 0, 1.0)]
    #[case(2000, 1000, 0.5)]
    #[case(1000, 1000, 0.0)]
    fn remaining_fraction_values(
        #[case] interval: u32,
        #[case] tick_ms: u32,
        #[case] expected: f64,
    ) {
        let mut sc = SpeedController::new(interval);
        if tick_ms > 0 {
            sc.tick(tick_ms);
        }
        assert!((sc.remaining_fraction() - expected).abs() < f64::EPSILON);
    }
}
