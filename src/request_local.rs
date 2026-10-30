//! Request-scoped thread-locals: state that belongs to the request a worker
//! thread is serving, not to the thread.
//!
//! A worker serves one request at a time, so per-request state — the session
//! id, the cookies to set, the open transaction, the dev logs, the deadline —
//! has always lived in thread-locals that the worker resets between requests
//! (`serve::request_scope`). For several requests to share one worker (one
//! coroutine each, switching while one waits on the network), each one needs
//! its own copy of that state, put in place when it runs and put away when it
//! yields.
//!
//! [`request_local!`](crate::request_local) declares such a variable. It is
//! an ordinary `thread_local!` — reads and writes cost the same and keep the
//! same API (`with`, `try_with`, `get`, `set`) — whose initialiser also enters
//! it in this thread's registry. [`RequestState::capture`] then takes every
//! registered value out (leaving each at its initial value) and
//! [`RequestState::restore`] puts a captured set back. A variable first used
//! after a capture was not part of it: restoring that capture resets it, so a
//! request never sees a value another one wrote.
//!
//! Every other `thread_local!` is shared by whatever runs on the thread
//! (registries, caches, pools, runtimes); `tests/request_local_audit.rs`
//! fails on one that is in neither list, so a new piece of request state
//! cannot be added as a plain thread-local by accident.

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::thread::{AccessError, LocalKey};

/// A request-scoped thread-local, declared with
/// [`request_local!`](crate::request_local).
pub struct RequestLocal<T: 'static> {
    key: &'static LocalKey<T>,
    init: fn() -> T,
    name: &'static str,
}

impl<T: 'static> RequestLocal<T> {
    #[doc(hidden)]
    pub const fn new(key: &'static LocalKey<T>, init: fn() -> T, name: &'static str) -> Self {
        Self { key, init, name }
    }

    /// As [`LocalKey::with`].
    #[inline]
    pub fn with<F, R>(&'static self, f: F) -> R
    where
        F: FnOnce(&T) -> R,
    {
        self.key.with(f)
    }

    /// As [`LocalKey::try_with`].
    #[inline]
    pub fn try_with<F, R>(&'static self, f: F) -> Result<R, AccessError>
    where
        F: FnOnce(&T) -> R,
    {
        self.key.try_with(f)
    }

    /// Where it is declared: `module::path::NAME`.
    pub fn name(&self) -> &'static str {
        self.name
    }
}

impl<T: 'static> RequestLocal<Cell<T>> {
    /// As [`LocalKey::set`], except that the variable is initialised (and
    /// so registered) first.
    #[inline]
    pub fn set(&'static self, value: T) {
        self.key.with(|cell| cell.set(value));
    }
}

impl<T: Copy + 'static> RequestLocal<Cell<T>> {
    /// As [`LocalKey::get`].
    #[inline]
    pub fn get(&'static self) -> T {
        self.key.with(Cell::get)
    }
}

impl<T: 'static> RequestLocal<RefCell<T>> {
    /// As [`LocalKey::set`] on a `RefCell`: replaces the value.
    #[inline]
    pub fn set(&'static self, value: T) {
        self.key.with(|cell| *cell.borrow_mut() = value);
    }
}

/// What the registry can do with a variable, whatever its type.
pub trait Swap: Sync {
    fn name(&self) -> &'static str;
    /// The current value, the variable left at its initial value.
    fn take(&self) -> Box<dyn Any>;
    /// Puts back a value [`Swap::take`] gave.
    fn put(&self, value: Box<dyn Any>);
    /// Back to the initial value.
    fn reset(&self);
}

impl<T: 'static> Swap for RequestLocal<RefCell<T>> {
    fn name(&self) -> &'static str {
        self.name
    }
    fn take(&self) -> Box<dyn Any> {
        let fresh = (self.init)().into_inner();
        Box::new(self.key.with(|cell| cell.replace(fresh)))
    }
    fn put(&self, value: Box<dyn Any>) {
        let value = *value
            .downcast::<T>()
            .unwrap_or_else(|_| panic!("request_local {}: a value of another type", self.name));
        self.key.with(|cell| *cell.borrow_mut() = value);
    }
    fn reset(&self) {
        let fresh = (self.init)().into_inner();
        self.key.with(|cell| *cell.borrow_mut() = fresh);
    }
}

impl<T: 'static> Swap for RequestLocal<Cell<T>> {
    fn name(&self) -> &'static str {
        self.name
    }
    fn take(&self) -> Box<dyn Any> {
        let fresh = (self.init)().into_inner();
        Box::new(self.key.with(|cell| cell.replace(fresh)))
    }
    fn put(&self, value: Box<dyn Any>) {
        let value = *value
            .downcast::<T>()
            .unwrap_or_else(|_| panic!("request_local {}: a value of another type", self.name));
        self.key.with(|cell| cell.set(value));
    }
    fn reset(&self) {
        let fresh = (self.init)().into_inner();
        self.key.with(|cell| cell.set(fresh));
    }
}

thread_local! {
    /// The request-scoped variables this thread has used, in the order it
    /// first used them.
    static REGISTRY: RefCell<Vec<&'static dyn Swap>> = const { RefCell::new(Vec::new()) };
}

/// Called by each variable's initialiser, once per thread.
#[doc(hidden)]
pub fn register(local: &'static dyn Swap) {
    let _ = REGISTRY.try_with(|registry| registry.borrow_mut().push(local));
}

fn registered() -> Vec<&'static dyn Swap> {
    REGISTRY.with(|registry| registry.borrow().clone())
}

/// The request-scoped state of this thread, taken out by [`capture`].
///
/// [`capture`]: RequestState::capture
pub struct RequestState {
    values: Vec<Box<dyn Any>>,
}

impl RequestState {
    /// Takes every request-scoped value of this thread out, leaving each
    /// variable at its initial value, as for a request that has just begun.
    pub fn capture() -> Self {
        let values = registered().into_iter().map(|local| local.take()).collect();
        Self { values }
    }

    /// Puts a captured state back. Variables first used since that capture
    /// were not in it, and go back to their initial value.
    pub fn restore(self) {
        let locals = registered();
        let captured = self.values.len();
        for (local, value) in locals.iter().zip(self.values) {
            local.put(value);
        }
        for local in &locals[captured.min(locals.len())..] {
            local.reset();
        }
    }

    /// Every request-scoped variable of this thread back to its initial
    /// value.
    pub fn reset() {
        for local in registered() {
            local.reset();
        }
    }

    /// The names of the variables registered on this thread so far.
    pub fn names() -> Vec<&'static str> {
        registered().iter().map(|local| local.name()).collect()
    }
}

/// Declares request-scoped thread-locals; the syntax of `thread_local!`.
///
/// ```ignore
/// crate::request_local! {
///     /// The session of the request being served.
///     static CURRENT_SESSION_ID: RefCell<Option<String>> = const { RefCell::new(None) };
/// }
/// ```
#[macro_export]
macro_rules! request_local {
    () => {};
    ($(#[$attr:meta])* $vis:vis static $name:ident: $t:ty = const $init:block; $($rest:tt)*) => {
        $crate::request_local!(@one [$(#[$attr])*] $vis $name, $t, $init);
        $crate::request_local!($($rest)*);
    };
    ($(#[$attr:meta])* $vis:vis static $name:ident: $t:ty = $init:expr; $($rest:tt)*) => {
        $crate::request_local!(@one [$(#[$attr])*] $vis $name, $t, $init);
        $crate::request_local!($($rest)*);
    };
    (@one [$($attr:tt)*] $vis:vis $name:ident, $t:ty, $init:expr) => {
        $($attr)*
        $vis static $name: $crate::request_local::RequestLocal<$t> = {
            fn init() -> $t {
                $init
            }
            ::std::thread_local! {
                static KEY: $t = {
                    $crate::request_local::register(&$name);
                    init()
                };
            }
            $crate::request_local::RequestLocal::new(
                &KEY,
                init,
                concat!(module_path!(), "::", stringify!($name)),
            )
        };
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    crate::request_local! {
        static NAME: RefCell<Option<String>> = const { RefCell::new(None) };
        static COUNT: Cell<u32> = Cell::new(7);
        static LATE: RefCell<Vec<u8>> = RefCell::new(Vec::new());
    }

    #[test]
    fn a_captured_state_comes_back_and_the_next_request_starts_fresh() {
        std::thread::spawn(|| {
            NAME.set(Some("a".into()));
            COUNT.set(1);
            let a = RequestState::capture();
            assert_eq!(NAME.with(|n| n.borrow().clone()), None, "B starts fresh");
            assert_eq!(COUNT.get(), 7, "at its initial value, not its Default");

            NAME.set(Some("b".into()));
            LATE.with(|l| l.borrow_mut().push(1));
            let b = RequestState::capture();

            a.restore();
            assert_eq!(NAME.with(|n| n.borrow().clone()).as_deref(), Some("a"));
            assert_eq!(COUNT.get(), 1);
            assert!(
                LATE.with(|l| l.borrow().is_empty()),
                "a variable first used by B is reset for A"
            );

            b.restore();
            assert_eq!(NAME.with(|n| n.borrow().clone()).as_deref(), Some("b"));
            assert_eq!(LATE.with(|l| l.borrow().clone()), vec![1]);

            RequestState::reset();
            assert_eq!(COUNT.get(), 7);
            assert!(RequestState::names()
                .iter()
                .any(|n| n.ends_with("request_local::tests::LATE")));
        })
        .join()
        .unwrap();
    }
}
