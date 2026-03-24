pub use wisecrow_dto::SpeedController;

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case(100, 500)]
    #[case(20_000, 10_000)]
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
    #[case(500, 500)]
    fn speed_up_clamps(#[case] start: u32, #[case] expected: u32) {
        let mut sc = SpeedController::new(start);
        sc.speed_up();
        assert_eq!(sc.interval_ms(), expected);
    }

    #[rstest]
    #[case(3000, 3500)]
    #[case(10_000, 10_000)]
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
