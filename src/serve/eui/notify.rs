//! `eui_notify(...)` — one line said to the person at the window (EUI 02 §5.2).
//!
//! A notification is not part of the document, so it is not part of the view:
//! there is no node to put it on and no state that "is" a notification. It is
//! something an application *does*, once, where something happened — a message
//! arrived, a build finished, a backup failed — and that is a call in a
//! handler, not a value in a tree.
//!
//! So the call queues an op here, and the render that follows it carries the
//! op out with its batch. Which render that is, is the point: a notification
//! goes to the session whose handler asked for it and to no other. Reaching
//! *other* people's windows is what [`super::wake_component`] is for — wake
//! them, and their own `wake` handler decides whether their machine should
//! say anything.
//!
//! The queue is per thread and is emptied at both ends of a render pass
//! ([`super::stats::Current`]), so a handler that raised before its render
//! cannot leave a line behind for whoever this worker thread serves next.

use std::cell::RefCell;

use eui_proto::limits::{MAX_NOTIFY_BODY, MAX_NOTIFY_TAG, MAX_NOTIFY_TITLE};
use eui_proto::Op;

thread_local! {
    /// What this pass has asked to say, in the order it asked.
    static QUEUED: RefCell<Vec<Op>> = const { RefCell::new(Vec::new()) };
}

/// Queue one line for the session this thread is rendering.
///
/// The three strings are cut to the protocol's limits rather than refused.
/// The limits are a client's defence against a server that sends a novel;
/// an application that interpolates a mail subject into a body is not that,
/// and failing its handler over a long subject would be the framework
/// breaking a working application on the day somebody sent it a long one.
pub fn queue(title: &str, body: &str, tag: &str) {
    let op = Op::Notify {
        title: cut(title, MAX_NOTIFY_TITLE),
        body: cut(body, MAX_NOTIFY_BODY),
        tag: cut(tag, MAX_NOTIFY_TAG),
    };
    QUEUED.with(|q| q.borrow_mut().push(op));
}

/// What this pass asked to say, taken. Called once, by the encoder, as the
/// batch is built.
pub fn take() -> Vec<Op> {
    QUEUED.with(|q| std::mem::take(&mut *q.borrow_mut()))
}

/// Forget what was queued: a new pass on this thread starts with nothing,
/// and a pass that ended without a render leaves nothing.
pub fn clear() {
    QUEUED.with(|q| q.borrow_mut().clear());
}

/// `s` at most `max` bytes, cut on a character boundary.
///
/// An ellipsis where something was cut, so a line that was shortened reads
/// as one — and not when it was not.
fn cut(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_owned();
    }
    const MARK: &str = "…";
    let mut end = max.saturating_sub(MARK.len());
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{MARK}", &s[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_line_is_cut_on_a_character_boundary() {
        let long = "é".repeat(MAX_NOTIFY_TITLE);
        let shortened = cut(&long, MAX_NOTIFY_TITLE);
        assert!(
            shortened.len() <= MAX_NOTIFY_TITLE,
            "{} bytes",
            shortened.len()
        );
        assert!(shortened.ends_with('…'));
        assert!(shortened.chars().all(|c| c == 'é' || c == '…'));
        // A line that fits is untouched, ellipsis included.
        assert_eq!(cut("bonjour", MAX_NOTIFY_TITLE), "bonjour");
    }

    #[test]
    fn the_queue_belongs_to_the_pass_that_filled_it() {
        clear();
        queue("Nouveau message", "Ana : on déjeune ?", "thread-7");
        assert_eq!(take().len(), 1);
        assert!(take().is_empty(), "taken once");
        queue("orphelin", "", "");
        clear();
        assert!(
            take().is_empty(),
            "a pass that never rendered leaves nothing"
        );
    }
}
