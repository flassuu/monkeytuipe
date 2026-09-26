//! A replayable record of everything a test did.
//!
//! The website keeps this as `EventLog` and derives the chart, the key-timing
//! statistics and the whole result payload from it. So does this: a test's
//! running counters are enough to score it, but not enough to say *when*
//! anything happened, and timing is most of what a typing result is.
//!
//! The log is append-only and never consulted while typing, so a bug in reading
//! it cannot cost the typist their test. Timestamps are milliseconds since the
//! first keystroke, which is the origin the website uses.

/// One thing the typist did to a word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// A character was entered.
    Insert {
        word: usize,
        /// Where it landed, in UTF-16 code units from the start of the word.
        index: usize,
        /// Whether it was the right character at that position.
        correct: bool,
        /// What was entered.
        ch: char,
    },
    /// A character was removed from a word.
    Delete { word: usize, index: usize },
    /// The word was jumped over with the skip key.
    Skip { word: usize },
}

impl Event {
    /// The word the event happened in.
    pub fn word(&self) -> usize {
        match *self {
            Self::Insert { word, .. } | Self::Delete { word, .. } | Self::Skip { word } => word,
        }
    }
}

/// An event and when it happened.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimedEvent {
    /// Milliseconds since the first keystroke.
    pub ms: f64,
    pub event: Event,
}

/// Every event of one test, in order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EventLog {
    events: Vec<TimedEvent>,
    /// When the test ended, in the same origin as the events.
    ///
    /// `None` while the test is still running; [`EventLog::end_ms`] falls back
    /// to the last event, which is what a live chart wants.
    end_ms: Option<f64>,
}

impl EventLog {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records an event at `ms` from the start of the test.
    pub fn push(&mut self, ms: f64, event: Event) {
        self.events.push(TimedEvent { ms, event });
    }

    /// Marks the test as over.
    pub fn finish(&mut self, ms: f64) {
        self.end_ms = Some(ms);
    }

    /// Whether the test has ended.
    pub fn is_finished(&self) -> bool {
        self.end_ms.is_some()
    }

    /// The recorded end time, if there is one.
    pub fn recorded_end_ms(&self) -> Option<f64> {
        self.end_ms
    }

    /// How far the log reaches: the end of the test, or the last event.
    pub fn end_ms(&self) -> f64 {
        self.end_ms
            .or_else(|| self.events.last().map(|e| e.ms))
            .unwrap_or(0.0)
    }

    /// Every event, in order.
    pub fn events(&self) -> &[TimedEvent] {
        &self.events
    }

    /// Just the insertions, which is what the chart's two fastest series count.
    pub fn inserts(&self) -> impl Iterator<Item = &TimedEvent> {
        self.events
            .iter()
            .filter(|e| matches!(e.event, Event::Insert { .. }))
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Discards everything, for a restart.
    pub fn clear(&mut self) {
        self.events.clear();
        self.end_ms = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn insert(word: usize, ch: char) -> Event {
        Event::Insert {
            word,
            index: 0,
            correct: true,
            ch,
        }
    }

    #[test]
    fn an_empty_log_reaches_nowhere() {
        let log = EventLog::new();
        assert!(log.is_empty());
        assert!(!log.is_finished());
        assert_eq!(log.end_ms(), 0.0);
    }

    #[test]
    fn events_are_kept_in_order() {
        let mut log = EventLog::new();
        log.push(0.0, insert(0, 'a'));
        log.push(120.0, insert(0, 'b'));
        log.push(400.0, Event::Delete { word: 0, index: 1 });
        let times: Vec<f64> = log.events().iter().map(|e| e.ms).collect();
        assert_eq!(times, [0.0, 120.0, 400.0]);
        assert_eq!(log.len(), 3);
    }

    #[test]
    fn a_running_log_ends_at_its_last_event() {
        let mut log = EventLog::new();
        log.push(0.0, insert(0, 'a'));
        log.push(900.0, insert(0, 'b'));
        assert_eq!(log.end_ms(), 900.0, "a live chart reads up to now");
    }

    #[test]
    fn a_finished_log_ends_where_it_was_marked() {
        let mut log = EventLog::new();
        log.push(0.0, insert(0, 'a'));
        log.finish(1500.0);
        assert!(log.is_finished());
        assert_eq!(
            log.end_ms(),
            1500.0,
            "a pause before the end is part of the test"
        );
    }

    #[test]
    fn only_insertions_are_counted_as_typing() {
        let mut log = EventLog::new();
        log.push(0.0, insert(0, 'a'));
        log.push(10.0, Event::Delete { word: 0, index: 0 });
        log.push(20.0, Event::Skip { word: 1 });
        assert_eq!(log.inserts().count(), 1);
    }

    #[test]
    fn every_event_knows_its_word() {
        assert_eq!(insert(3, 'a').word(), 3);
        assert_eq!(Event::Delete { word: 4, index: 0 }.word(), 4);
        assert_eq!(Event::Skip { word: 5 }.word(), 5);
    }

    #[test]
    fn clearing_empties_the_log_and_unfinishes_the_test() {
        let mut log = EventLog::new();
        log.push(0.0, insert(0, 'a'));
        log.finish(100.0);
        log.clear();
        assert!(log.is_empty());
        assert!(!log.is_finished());
        assert_eq!(log.end_ms(), 0.0);
    }
}
