/// Centralized time dispatcher.
///
/// Runs a 200ms outer guard that schedules a one-shot timer at each second
/// boundary, accurate to within ~50ms. The 200ms poll also detects NTP jumps
/// and reschedules accordingly.
///
/// Usage:
/// ```
/// clock::Clock::new()
///     .on_second(|now| { ... })
///     .on_minute(|now| { ... })
///     .start();
/// ```
use std::cell::Cell;
use std::rc::Rc;

use chrono::{DateTime, Local};

type Cb = Box<dyn Fn(DateTime<Local>) + 'static>;

pub struct Clock {
    second_subs: Vec<Cb>,
    minute_subs: Vec<Cb>,
}

impl Clock {
    pub fn new() -> Self {
        Self { second_subs: Vec::new(), minute_subs: Vec::new() }
    }

    pub fn on_second(mut self, cb: impl Fn(DateTime<Local>) + 'static) -> Self {
        self.second_subs.push(Box::new(cb));
        self
    }

    pub fn on_minute(mut self, cb: impl Fn(DateTime<Local>) + 'static) -> Self {
        self.minute_subs.push(Box::new(cb));
        self
    }

    /// Start the dispatcher. Must be called on the GTK main thread.
    pub fn start(self) {
        let second_subs: Rc<Vec<Cb>> = Rc::new(self.second_subs);
        let minute_subs: Rc<Vec<Cb>> = Rc::new(self.minute_subs);
        // Unix timestamp of the next second boundary we have already scheduled.
        let last_scheduled: Rc<Cell<i64>> = Rc::new(Cell::new(0));
        // Unix minute of the last minute-boundary fire.
        let last_minute: Rc<Cell<i64>> = Rc::new(Cell::new(0));

        // Fire immediately so the UI is populated before the first boundary tick.
        let now = Local::now();
        for cb in second_subs.iter() {
            cb(now);
        }

        glib::timeout_add_local(std::time::Duration::from_millis(200), move || {
            let now = Local::now();
            // The next whole second we want to fire at.
            let next_sec_ts = now.timestamp() + 1;
            let ms_until = 1000 - now.timestamp_subsec_millis() as u64;

            // Only schedule if we haven't already queued this second boundary.
            if last_scheduled.get() != next_sec_ts {
                last_scheduled.set(next_sec_ts);

                let subs = second_subs.clone();
                let min_subs = minute_subs.clone();
                let last_min = last_minute.clone();

                // One-shot: fires once at the boundary then removes itself.
                glib::timeout_add_local(std::time::Duration::from_millis(ms_until), move || {
                    let fire_time = Local::now();
                    for cb in subs.iter() {
                        cb(fire_time);
                    }

                    let this_min = fire_time.timestamp() / 60;
                    if last_min.get() != this_min {
                        last_min.set(this_min);
                        for cb in min_subs.iter() {
                            cb(fire_time);
                        }
                    }

                    glib::ControlFlow::Break
                });
            }

            glib::ControlFlow::Continue
        });
    }
}
