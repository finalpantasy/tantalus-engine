use std::collections::VecDeque;
use tantalus_contracts::TraceEvent;

pub const DEFAULT_TRACE_CAPACITY: usize = 1024;

#[derive(Debug, Clone)]
pub struct TraceLog {
    entries: VecDeque<TraceEvent>,
    capacity: usize,
}

impl Default for TraceLog {
    fn default() -> Self {
        Self::new(DEFAULT_TRACE_CAPACITY)
    }
}

impl TraceLog {
    pub fn new(capacity: usize) -> Self {
        Self {
            entries: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    pub fn push(&mut self, event: TraceEvent) {
        if self.capacity == 0 {
            return;
        }
        if self.entries.len() == self.capacity {
            self.entries.pop_front();
        }
        self.entries.push_back(event);
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &TraceEvent> {
        self.entries.iter()
    }
}
