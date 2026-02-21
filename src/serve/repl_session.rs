//! REPL Session Management
//!
//! This module provides thread-local REPL session storage for the development mode.
//! Each tokio worker thread has its own store since Interpreter is !Send.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};
use uuid::Uuid;

use crate::interpreter::Interpreter;

/// REPL session data containing interpreter and last access time
pub(crate) struct ReplSession {
    pub interpreter: RefCell<Interpreter>,
    pub last_accessed: RefCell<Instant>,
    pub models_loaded: RefCell<bool>,
}

/// Thread-local REPL session store
/// Each tokio worker thread has its own store
pub(crate) struct ReplSessionStore {
    sessions: RefCell<HashMap<String, Rc<ReplSession>>>,
    max_age: Duration,
}

impl ReplSessionStore {
    pub(crate) fn new() -> Self {
        Self {
            sessions: RefCell::new(HashMap::new()),
            max_age: Duration::from_secs(30 * 60), // 30 minutes
        }
    }

    /// Get existing session or create new one
    pub(crate) fn get_or_create(&self, session_id: &str) -> (String, Rc<ReplSession>) {
        // Try to get existing valid session
        {
            let sessions = self.sessions.borrow();
            if let Some(session) = sessions.get(session_id) {
                if !self.is_expired(session) {
                    *session.last_accessed.borrow_mut() = Instant::now();
                    return (session_id.to_string(), Rc::clone(session));
                }
            }
        }

        // Cleanup expired sessions periodically (1 in 50 chance)
        if rand::random::<u64>().is_multiple_of(50) {
            self.cleanup();
        }

        // Create new session
        let new_id = if session_id.is_empty() {
            Uuid::new_v4().to_string()
        } else {
            session_id.to_string()
        };

        let session = Rc::new(ReplSession {
            interpreter: RefCell::new(Interpreter::new()),
            last_accessed: RefCell::new(Instant::now()),
            models_loaded: RefCell::new(false),
        });

        {
            let mut sessions = self.sessions.borrow_mut();
            sessions.insert(new_id.clone(), Rc::clone(&session));
        }

        (new_id, session)
    }

    /// Check if a session is expired
    fn is_expired(&self, session: &ReplSession) -> bool {
        session.last_accessed.borrow().elapsed() > self.max_age
    }

    /// Cleanup expired sessions
    fn cleanup(&self) {
        let mut sessions = self.sessions.borrow_mut();
        let expired_keys: Vec<String> = sessions
            .iter()
            .filter(|(_, session)| self.is_expired(session.as_ref()))
            .map(|(k, _)| k.clone())
            .collect();
        for key in expired_keys {
            sessions.remove(&key);
        }
    }
}

// Thread-local storage for REPL sessions
// Each thread gets its own store, which is fine since sessions are identified by UUID
// and requests from the same browser tab will hit the same thread most of the time
thread_local! {
    pub(crate) static REPL_STORE: ReplSessionStore = ReplSessionStore::new();
}
