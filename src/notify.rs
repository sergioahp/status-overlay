/// Send a desktop notification via notify-send and log delivery failures.
pub fn send(summary: &str, body: &str) {
    crate::storage::append_notification_event(summary, body);
    let result = std::process::Command::new("notify-send")
        .args([
            "--app-name=status-overlay",
            "--icon=dialog-warning",
            summary,
            body,
        ])
        .status();
    match result {
        Ok(status) if status.success() => {}
        Ok(status) => eprintln!("[notify] notify-send exited with {status}: {summary}"),
        Err(error) => eprintln!("[notify] failed to run notify-send: {error}: {summary}"),
    }
}

/// Usage thresholds for warnings and reset detection.
pub const WARN_PCT: u32 = 90;
pub const RESTORE_PCT: u32 = 2;

/// Returns what notification (if any) should fire given old → new percentage.
pub enum Transition {
    Low,
    Depleted,
    Restored,
}

pub fn transition(prev: u32, next: u32) -> Option<Transition> {
    if prev < 100 && next >= 100 {
        Some(Transition::Depleted)
    } else if prev < WARN_PCT && (WARN_PCT..100).contains(&next) {
        Some(Transition::Low)
    } else if prev >= RESTORE_PCT && next < RESTORE_PCT {
        Some(Transition::Restored)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn is_depleted(t: Option<Transition>) -> bool {
        matches!(t, Some(Transition::Depleted))
    }
    fn is_low(t: Option<Transition>) -> bool {
        matches!(t, Some(Transition::Low))
    }
    fn is_restored(t: Option<Transition>) -> bool {
        matches!(t, Some(Transition::Restored))
    }

    #[test]
    fn depleted_when_crossing_100() {
        assert!(is_depleted(transition(99, 100)));
        assert!(is_depleted(transition(50, 100)));
        assert!(is_depleted(transition(0, 100)));
    }

    #[test]
    fn not_depleted_if_already_at_100() {
        assert!(!is_depleted(transition(100, 100)));
    }

    #[test]
    fn low_when_crossing_warn_threshold() {
        assert!(is_low(transition(WARN_PCT - 1, WARN_PCT)));
        assert!(is_low(transition(0, WARN_PCT)));
        assert!(is_low(transition(80, 95)));
    }

    #[test]
    fn not_low_when_crossing_into_100() {
        // Crossing to 100 is Depleted, not Low.
        assert!(!is_low(transition(80, 100)));
    }

    #[test]
    fn not_low_when_already_above_warn() {
        assert!(!is_low(transition(WARN_PCT, 95)));
    }

    #[test]
    fn restored_when_dropping_below_restore_threshold() {
        assert!(is_restored(transition(WARN_PCT, RESTORE_PCT - 1)));
        assert!(is_restored(transition(100, 0)));
        assert!(is_restored(transition(26, 0)));
    }

    #[test]
    fn not_restored_if_still_above_restore() {
        assert!(!is_restored(transition(95, RESTORE_PCT)));
    }

    #[test]
    fn not_restored_for_sub_threshold_noise() {
        assert!(!is_restored(transition(RESTORE_PCT - 1, 0)));
    }

    #[test]
    fn no_transition_for_small_increases_in_normal_range() {
        assert!(transition(50, 60).is_none());
        assert!(transition(0, 50).is_none());
    }

    #[test]
    fn no_transition_for_decreasing_within_normal_range() {
        assert!(transition(50, 40).is_none());
    }
}
