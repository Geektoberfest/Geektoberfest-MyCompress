use std::io::{self, IsTerminal, Write};
use std::time::{Duration, Instant};

pub struct Progress {
    operation: &'static str,
    completed_entries: u64,
    completed_bytes: u64,
    terminal: bool,
    last_update: Instant,
}

impl Progress {
    pub fn new(
        operation: &'static str,
        _total_entries: Option<u64>,
        _total_bytes: Option<u64>,
    ) -> Self {
        Self {
            operation,
            completed_entries: 0,
            completed_bytes: 0,
            terminal: io::stderr().is_terminal(),
            last_update: Instant::now(),
        }
    }

    pub fn update(&mut self, entries: u64, bytes: u64) {
        self.completed_entries = entries;
        self.completed_bytes = bytes;

        if self.last_update.elapsed() < Duration::from_millis(100) {
            return;
        }

        self.last_update = Instant::now();

        let message = format!(
            "{}: {} entries, {} bytes processed",
            self.operation, self.completed_entries, self.completed_bytes
        );

        let mut stderr = io::stderr().lock();

        if self.terminal {
            let _ = write!(stderr, "\r{message}");
        } else {
            let _ = writeln!(stderr, "{message}");
        }

        let _ = stderr.flush();
    }

    pub fn finish(&self) {
        if self.completed_entries == 0 {
            return;
        }

        let mut stderr = io::stderr().lock();
        let message = format!(
            "{} complete: {} entries, {} bytes processed",
            self.operation, self.completed_entries, self.completed_bytes
        );

        if self.terminal {
            let _ = writeln!(stderr, "\r{message}");
        } else {
            let _ = writeln!(stderr, "{message}");
        }

        let _ = stderr.flush();
    }
}
