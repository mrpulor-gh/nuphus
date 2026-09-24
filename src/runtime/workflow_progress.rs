//! Non-blocking commentary reminder. It never schedules another model request.
use std::time::{Duration, Instant};

pub(super) struct ProgressCadence {
    last_message: Instant,
    last_reminder: Option<Instant>,
    calls_since_message: usize,
}

impl ProgressCadence {
    pub fn new(now: Instant) -> Self {
        Self {
            last_message: now,
            last_reminder: None,
            calls_since_message: 0,
        }
    }

    pub fn reported(&mut self, now: Instant) {
        self.last_message = now;
        self.calls_since_message = 0;
    }

    pub fn business_call(&mut self) {
        self.calls_since_message += 1;
    }

    pub fn take_reminder(&mut self, now: Instant) -> bool {
        let quiet = now.duration_since(self.last_message) >= Duration::from_secs(30)
            || self.calls_since_message >= 6;
        let ready = self
            .last_reminder
            .is_none_or(|last| now.duration_since(last) >= Duration::from_secs(30));
        if quiet && ready {
            self.last_reminder = Some(now);
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn silence_reminder_is_bounded_and_resets_on_public_text() {
        let now = Instant::now();
        let mut cadence = ProgressCadence::new(now);
        assert!(!cadence.take_reminder(now));
        for _ in 0..6 {
            cadence.business_call();
        }
        assert!(cadence.take_reminder(now));
        assert!(!cadence.take_reminder(now + Duration::from_secs(29)));
        assert!(cadence.take_reminder(now + Duration::from_secs(30)));
        cadence.reported(now + Duration::from_secs(31));
        assert!(!cadence.take_reminder(now + Duration::from_secs(60)));
        assert!(cadence.take_reminder(now + Duration::from_secs(61)));
    }
    #[test]
    fn long_tool_needs_no_tool_count_to_trigger_next_request_reminder() {
        let now = Instant::now();
        assert!(ProgressCadence::new(now).take_reminder(now + Duration::from_secs(30)));
    }
}
