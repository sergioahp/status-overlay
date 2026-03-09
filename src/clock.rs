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

    #[allow(dead_code)]
    pub fn on_minute(mut self, cb: impl Fn(DateTime<Local>) + 'static) -> Self {
        self.minute_subs.push(Box::new(cb));
        self
    }

    /// Start the dispatcher. Must be called on the GTK main thread.
    pub fn start(self) {
        let second_subs: Rc<Vec<Cb>> = Rc::new(self.second_subs);
        let minute_subs: Rc<Vec<Cb>> = Rc::new(self.minute_subs);
        // Unix minute of the last minute-boundary fire.
        let last_minute: Rc<Cell<i64>> = Rc::new(Cell::new(0));
        // Generation counter: incremented on every 200ms tick so that each tick
        // supersedes the previous one-shot. Old one-shots still fire (glib
        // timeouts can't be cancelled cheaply) but check the this_generation and
        // no-op if they're stale. This means the LAST 200ms tick before each
        // second boundary is the one that actually fires — giving the most
        // accurate estimate of ms_until at the cost of a few extra no-op firings.
        let this_generation: Rc<Cell<u64>> = Rc::new(Cell::new(0));

        // Fire immediately so the UI is populated before the first boundary tick.
        let now = Local::now();
        for cb in second_subs.iter() {
            cb(now);
        }

        glib::timeout_add_local(std::time::Duration::from_millis(200), move || {
            let now = Local::now();
            let ms_until = 1000 - now.timestamp_subsec_millis() as u64;

            // Each 200ms tick gets a new this_generation, superseding the previous
            // one-shot. The old one-shot will fire but do nothing.
            let this_gen = this_generation.get().wrapping_add(1);
            this_generation.set(this_gen);

            let subs = second_subs.clone();
            let min_subs = minute_subs.clone();
            let last_min = last_minute.clone();
            let this_gen_check = this_generation.clone();

            glib::timeout_add_local(std::time::Duration::from_millis(ms_until), move || {
                // Only the most-recently-scheduled one-shot fires.
                if this_gen_check.get() == this_gen {
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
                }

                glib::ControlFlow::Break
            });

            glib::ControlFlow::Continue
        });
    }
}
