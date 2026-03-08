/// Fire-and-forget desktop notification via notify-send.
pub fn send(summary: &str, body: &str) {
    let _ = std::process::Command::new("notify-send")
        .args(["--app-name=status-overlay", "--icon=dialog-warning", summary, body])
        .spawn();
}

/// Thresholds
pub const WARN_PCT: u32 = 90;
pub const RESTORE_PCT: u32 = 30;

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
    } else if prev >= WARN_PCT && next < RESTORE_PCT {
        Some(Transition::Restored)
    } else {
        None
    }
}
