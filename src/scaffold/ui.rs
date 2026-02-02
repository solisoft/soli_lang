//! UI components for scaffold display

use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

/// TUI helper for displaying progress
pub struct ProgressDisplay {
    current_step: usize,
    total_steps: usize,
}

impl ProgressDisplay {
    pub fn new(total_steps: usize) -> Self {
        Self {
            current_step: 0,
            total_steps,
        }
    }

    pub fn header(name: &str) {
        println!();
        println!(
            "  \x1b[1m\x1b[38;5;141m◆\x1b[0m  \x1b[1mCreating new Soli application:\x1b[0m \x1b[36m{}\x1b[0m",
            name
        );
        println!();
    }

    pub fn step(&mut self, description: &str) {
        self.current_step += 1;
        print!(
            "  \x1b[2m[{}/{}]\x1b[0m {} ",
            self.current_step, self.total_steps, description
        );
        io::stdout().flush().unwrap();
    }

    pub fn done() {
        println!("\x1b[32m✓\x1b[0m");
    }

    pub fn skip(reason: &str) {
        println!("\x1b[33m⊘\x1b[0m \x1b[2m{}\x1b[0m", reason);
    }

    #[allow(dead_code)]
    pub fn fail(reason: &str) {
        println!("\x1b[31m✗\x1b[0m \x1b[2m{}\x1b[0m", reason);
    }

    pub fn info(message: &str) {
        println!("  \x1b[2m│\x1b[0m  {}", message);
    }
}

/// Spinner for long-running operations
pub struct Spinner {
    running: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl Spinner {
    pub fn start(message: &str) -> Self {
        let running = Arc::new(AtomicBool::new(true));
        let running_clone = running.clone();
        let message = message.to_string();

        let handle = thread::spawn(move || {
            let frames = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
            let mut i = 0;
            while running_clone.load(Ordering::Relaxed) {
                print!(
                    "\r  \x1b[36m{}\x1b[0m {} ",
                    frames[i % frames.len()],
                    message
                );
                io::stdout().flush().unwrap();
                thread::sleep(Duration::from_millis(80));
                i += 1;
            }
        });

        Self {
            running,
            handle: Some(handle),
        }
    }

    pub fn stop_with_success(self, message: &str) {
        self.running.store(false, Ordering::Relaxed);
        if let Some(handle) = self.handle {
            let _ = handle.join();
        }
        println!("\r  \x1b[32m✓\x1b[0m {}                    ", message);
    }

    pub fn stop_with_warning(self, message: &str) {
        self.running.store(false, Ordering::Relaxed);
        if let Some(handle) = self.handle {
            let _ = handle.join();
        }
        println!("\r  \x1b[33m⊘\x1b[0m {}                    ", message);
    }
}
