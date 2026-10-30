//! Several requests on one worker: each runs on a coroutine of its own, and
//! hands the worker over while it waits on the network.
//!
//! A worker used to run a request from start to finish, so an action calling
//! a slow API held one of the (by default two) production workers for the
//! whole wait. With `SOLI_COROUTINES=1` the worker runs each request on a
//! stackful coroutine ([`corosensei`]); when the request's HTTP call would
//! block, [`drive`] polls the call's future from the coroutine, and while it
//! is pending suspends the coroutine — the worker serves other requests, and
//! the future's waker puts this one back on the worker's ready list. Soli code
//! does not change: no `await`, the call returns as before.
//!
//! What makes a request's state its own across a switch:
//! - its own VM and interpreter shell ([`Scheduler::start`]), sharing the
//!   worker's classes, functions and compiled code;
//! - the request-scoped thread-locals ([`crate::request_local`]), taken out
//!   when it suspends and put back when it resumes;
//! - `req`/`params`/`cookies` in the global environment and the view
//!   helpers' environment, saved and rewritten the same way.
//!
//! A request does not suspend — the call blocks as before — inside a
//! transaction, a `grouped` block, a render, a value's formatting, or a
//! [`NoYield`] section (a `stream` block). Off by default, and under `--dev`.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::future::Future;
use std::pin::pin;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};

use corosensei::stack::DefaultStack;
use corosensei::{Coroutine, CoroutineResult, Yielder};
use crossbeam_channel as channel;

use crate::interpreter::environment::Environment;
use crate::interpreter::{Interpreter, Value};
use crate::request_local::RequestState;

/// Each coroutine's stack: the worker thread's own size (`SOLI_WORKER_STACK_MB`),
/// since the tree-walker recurses on it. Reserved, not committed: pages are
/// only touched as deep as a request goes.
fn stack_size() -> usize {
    super::server_constants::worker_stack_bytes()
}

/// Are coroutines on? `SOLI_COROUTINES=1` (or `true`/`on`/`yes`), and not in
/// `--dev`, where the hot reload and the dev bar assume one request at a time.
pub(crate) fn enabled(dev_mode: bool) -> bool {
    !dev_mode
        && crate::platform::env::var("SOLI_COROUTINES")
            .map(|v| matches!(v.trim(), "1" | "true" | "on" | "yes"))
            .unwrap_or(false)
}

/// How many requests one worker runs at once, suspended ones included
/// (`SOLI_WORKER_CONCURRENCY`, default 32).
fn concurrency() -> usize {
    crate::platform::env::var("SOLI_WORKER_CONCURRENCY")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(32)
}

type CoYielder = Yielder<(), ()>;

/// The coroutine running on this thread, if any.
#[derive(Clone, Copy)]
struct Current {
    yielder: *const CoYielder,
    id: u64,
}

thread_local! {
    /// Set while a coroutine runs, cleared while it is suspended.
    static CURRENT: Cell<Option<Current>> = const { Cell::new(None) };
    /// Where a waker sends the id of the coroutine to resume.
    static READY_TX: RefCell<Option<channel::Sender<u64>>> = const { RefCell::new(None) };
    /// The worker's global environment, for the request globals.
    static GLOBAL_ENV: RefCell<Option<Rc<RefCell<Environment>>>> = const { RefCell::new(None) };
    /// Open [`NoYield`] sections.
    static NO_YIELD: Cell<usize> = const { Cell::new(0) };
    /// Set while the scheduler runs a request directly, on the worker.
    static DIRECT: Cell<bool> = const { Cell::new(false) };
    /// That direct request reached a wait it could have suspended at.
    static WOULD_WAIT: Cell<bool> = const { Cell::new(false) };
}

/// While alive, the current request does not suspend: its I/O blocks.
pub(crate) struct NoYield(());

impl NoYield {
    pub(crate) fn enter() -> Self {
        NO_YIELD.with(|n| n.set(n.get() + 1));
        NoYield(())
    }
}

impl Drop for NoYield {
    fn drop(&mut self) {
        let _ = NO_YIELD.try_with(|n| n.set(n.get().saturating_sub(1)));
    }
}

/// May the current code suspend its request here?
pub(crate) fn can_yield() -> bool {
    CURRENT.with(|c| c.get().is_some()) && suspendable_here()
}

/// Nothing in progress that a suspension would break.
fn suspendable_here() -> bool {
    NO_YIELD.with(|n| n.get() == 0)
        && !crate::interpreter::builtins::model::crud::has_active_tx()
        && !crate::interpreter::builtins::model::batch::in_block()
        && !crate::interpreter::value::formatting_in_progress()
}

/// Called where a wait blocks because it could not suspend. A request the
/// scheduler ran directly (its route never waited before) that reaches a
/// wait it could have suspended at marks its route: the next ones run on a
/// coroutine. One read of a thread-local when nothing runs directly.
pub(crate) fn note_blocking_wait() {
    if DIRECT.with(|d| d.get()) && suspendable_here() {
        WOULD_WAIT.with(|w| w.set(true));
    }
}

/// The route a request is grouped under, for [`Scheduler`]'s choice:
/// the method and the path, with segments that look like ids (a digit, or
/// more than 24 bytes) counted as one. A wrong grouping only costs speed —
/// a request on a coroutine it did not need, or one wait that blocks.
fn route_key(method: &str, path: &str) -> u64 {
    use std::hash::{BuildHasher, Hasher};
    static SEED: std::sync::OnceLock<ahash::RandomState> = std::sync::OnceLock::new();
    let mut hasher = SEED.get_or_init(ahash::RandomState::new).build_hasher();
    hasher.write(method.as_bytes());
    let path = path.split('?').next().unwrap_or(path);
    for segment in path.split('/') {
        hasher.write_u8(b'/');
        if segment.len() > 24 || segment.bytes().any(|b| b.is_ascii_digit()) {
            hasher.write_u8(0);
        } else {
            hasher.write(segment.as_bytes());
        }
    }
    hasher.finish()
}

/// How many routes a worker remembers as waiting; past it, it forgets them
/// all and learns again.
const MAX_WAITING_ROUTES: usize = 4096;

/// Wakes a suspended coroutine: sends its id to its worker, once per wait.
struct WakeFlag {
    id: u64,
    woken: AtomicBool,
    tx: channel::Sender<u64>,
}

impl Wake for WakeFlag {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }
    fn wake_by_ref(self: &Arc<Self>) {
        if !self.woken.swap(true, Ordering::AcqRel) {
            let _ = self.tx.send(self.id);
        }
    }
}

/// The request globals as they are now, to write back on resume.
struct Globals(Vec<(Rc<RefCell<Environment>>, &'static str, Value)>);

const GLOBAL_NAMES: [&str; 3] = ["req", "params", "cookies"];
const HELPER_NAMES: [&str; 5] = ["req", "params", "session", "cookies", "headers"];

impl Globals {
    fn save() -> Self {
        let mut saved = Vec::new();
        let mut keep = |env: Rc<RefCell<Environment>>, names: &[&'static str]| {
            for name in names {
                if let Some(value) = env.borrow().get(name) {
                    saved.push((env.clone(), *name, value));
                }
            }
        };
        if let Some(env) = GLOBAL_ENV.with(|g| g.borrow().clone()) {
            keep(env, &GLOBAL_NAMES);
        }
        if let Some(env) = crate::interpreter::builtins::template::helper_env() {
            keep(env, &HELPER_NAMES);
        }
        Globals(saved)
    }

    fn restore(self) {
        for (env, name, value) in self.0 {
            env.borrow_mut().define_or_update(name, value);
        }
    }
}

/// Runs `future` to completion from the current coroutine, suspending it
/// while the future is pending. Its I/O is driven by `handle`'s runtime; the
/// future itself is polled here, on the worker thread, with this request's
/// state in place — so it may hold `Value`s and write the dev logs.
///
/// Call only when [`can_yield`] said yes.
pub(crate) fn drive<F: Future>(handle: &tokio::runtime::Handle, future: F) -> F::Output {
    let current = CURRENT
        .with(|c| c.get())
        .expect("coroutine::drive outside a coroutine");
    let tx = READY_TX
        .with(|t| t.borrow().clone())
        .expect("coroutine::drive on a thread without a scheduler");
    let flag = Arc::new(WakeFlag {
        id: current.id,
        woken: AtomicBool::new(false),
        tx,
    });
    let waker = Waker::from(flag.clone());
    let mut cx = Context::from_waker(&waker);
    let mut future = pin!(future);
    loop {
        let polled = {
            let _runtime = handle.enter();
            future.as_mut().poll(&mut cx)
        };
        if let Poll::Ready(value) = polled {
            return value;
        }
        // Woken while it was being polled: poll again rather than sleep.
        if flag.woken.swap(false, Ordering::AcqRel) {
            continue;
        }
        let state = RequestState::capture();
        let globals = Globals::save();
        CURRENT.with(|c| c.set(None));
        // SAFETY: the yielder belongs to the coroutine running this code; it
        // lives as long as that coroutine, which is suspended right here.
        unsafe { (*current.yielder).suspend(()) };
        CURRENT.with(|c| c.set(Some(current)));
        globals.restore();
        state.restore();
        flag.woken.store(false, Ordering::Release);
    }
}

/// What a finished request hands back to the worker.
struct Done {
    vm: Option<crate::vm::Vm>,
}

type Task = Coroutine<(), (), Done, DefaultStack>;

/// The coroutines of one worker.
pub(crate) struct Scheduler {
    max: usize,
    /// Routes ([`route_key`]) seen waiting on the network: their requests
    /// run on a coroutine. Every other request runs directly on the worker,
    /// as without coroutines, and costs nothing more.
    waiting_routes: ahash::AHashSet<u64>,
    ready_rx: channel::Receiver<u64>,
    tasks: HashMap<u64, Task>,
    next_id: u64,
    stacks: Vec<DefaultStack>,
    vms: Vec<crate::vm::Vm>,
}

impl Scheduler {
    /// A scheduler for this worker thread; `global_env` is its interpreter's.
    pub(crate) fn new(global_env: Rc<RefCell<Environment>>) -> Self {
        let (ready_tx, ready_rx) = channel::unbounded();
        READY_TX.with(|t| *t.borrow_mut() = Some(ready_tx));
        // Queries of a suspended request must progress while the worker
        // serves others: this worker's DB I/O goes to the shared runtime.
        crate::interpreter::builtins::http_class::use_shared_db_reactor_on_this_thread();
        GLOBAL_ENV.with(|g| *g.borrow_mut() = Some(global_env));
        Self {
            max: concurrency(),
            waiting_routes: ahash::AHashSet::new(),
            ready_rx,
            tasks: HashMap::new(),
            next_id: 1,
            stacks: Vec::new(),
            vms: Vec::new(),
        }
    }

    /// Room for another request?
    pub(crate) fn has_room(&self) -> bool {
        self.tasks.len() < self.max
    }

    /// Requests started and not finished.
    pub(crate) fn running(&self) -> usize {
        self.tasks.len()
    }

    /// Where the wakers send ids: a channel the worker selects on.
    pub(crate) fn ready(&self) -> &channel::Receiver<u64> {
        &self.ready_rx
    }

    /// Starts `data` on a coroutine of its own and runs it until it finishes
    /// or first suspends.
    pub(crate) fn start(
        &mut self,
        interpreter: &mut Interpreter,
        worker_vm: &mut Option<crate::vm::Vm>,
        data: super::RequestData,
        dev_mode: bool,
    ) {
        let key = route_key(&data.method, &data.path);
        if !self.waiting_routes.contains(&key) {
            // A route that has not waited yet runs on the worker itself: a
            // coroutine costs ~100 ns a request, which a route that never
            // waits would pay for nothing. If it does wait, that wait blocks
            // once and the route moves to coroutines.
            DIRECT.with(|d| d.set(true));
            WOULD_WAIT.with(|w| w.set(false));
            super::dispatch_http_request(interpreter, worker_vm, data, dev_mode);
            DIRECT.with(|d| d.set(false));
            if WOULD_WAIT.with(|w| w.replace(false)) {
                if self.waiting_routes.len() >= MAX_WAITING_ROUTES {
                    self.waiting_routes.clear();
                }
                self.waiting_routes.insert(key);
            }
            return;
        }
        let id = self.next_id;
        self.next_id += 1;
        let mut shell = interpreter.coroutine_shell();
        let mut vm = worker_vm.as_ref().map(|worker| match self.vms.pop() {
            // A VM back from an earlier request already holds the worker's
            // globals; the request names (`req`, `params`, `cookies`) are
            // written by the handler call. Copying the globals again is most
            // of what a request on a coroutine costs.
            Some(mut vm) => {
                // The set only grows: copy it when the worker's has.
                if vm.failed_handlers.len() != worker.failed_handlers.len() {
                    vm.failed_handlers.clone_from(&worker.failed_handlers);
                }
                vm
            }
            None => {
                let mut vm = crate::vm::Vm::new();
                vm.globals.clone_from(&worker.globals);
                vm.failed_handlers.clone_from(&worker.failed_handlers);
                vm.check_return_types = worker.check_return_types;
                vm
            }
        });
        let stack = match self.stacks.pop() {
            Some(stack) => stack,
            None => match DefaultStack::new(stack_size()) {
                Ok(stack) => stack,
                Err(e) => {
                    // No stack to be had: serve it the old way.
                    eprintln!("[soli] coroutine stack: {e}; serving the request directly");
                    let mut direct = worker_vm.take();
                    super::dispatch_http_request(&mut shell, &mut direct, data, dev_mode);
                    *worker_vm = direct;
                    return;
                }
            },
        };
        let task = Coroutine::with_stack(stack, move |yielder: &CoYielder, ()| {
            CURRENT.with(|c| {
                c.set(Some(Current {
                    yielder: yielder as *const CoYielder,
                    id,
                }))
            });
            // The request state is what a suspended request's capture left
            // (the initial values) or what the last finished one left, as
            // between two requests without coroutines: the request path
            // resets what it reads.
            super::dispatch_http_request(&mut shell, &mut vm, data, dev_mode);
            CURRENT.with(|c| c.set(None));
            Done { vm: vm.take() }
        });
        self.tasks.insert(id, task);
        self.resume(id, worker_vm);
    }

    /// Resumes coroutine `id`, if it is still waiting. A stale or repeated
    /// wake finds it gone or polls a future still pending: harmless.
    pub(crate) fn resume(&mut self, id: u64, worker_vm: &mut Option<crate::vm::Vm>) {
        let Some(task) = self.tasks.get_mut(&id) else {
            return;
        };
        match task.resume(()) {
            CoroutineResult::Yield(()) => {}
            CoroutineResult::Return(done) => {
                let task = self.tasks.remove(&id).expect("task present");
                self.stacks.push(task.into_stack());
                if let Some(mut vm) = done.vm {
                    // A handler the VM failed on stays on the tree-walker for
                    // the requests after this one, as it does without
                    // coroutines.
                    if let Some(worker) = worker_vm.as_mut() {
                        if vm.failed_handlers.len() != worker.failed_handlers.len() {
                            worker
                                .failed_handlers
                                .extend(vm.failed_handlers.iter().cloned());
                        }
                    }
                    vm.reset();
                    self.vms.push(vm);
                }
            }
        }
    }

    /// Resumes every coroutine whose wait is over.
    pub(crate) fn resume_ready(&mut self, worker_vm: &mut Option<crate::vm::Vm>) {
        while let Ok(id) = self.ready_rx.try_recv() {
            self.resume(id, worker_vm);
        }
    }
}

impl Drop for Scheduler {
    fn drop(&mut self) {
        // A suspended coroutine is dropped by unwinding its stack with a
        // panic that the request's own `catch_unwind` would swallow, which
        // aborts the process. The worker is going away: let them go unrun.
        for (_, task) in self.tasks.drain() {
            std::mem::forget(task);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    crate::request_local! {
        static WHO: RefCell<Option<&'static str>> = const { RefCell::new(None) };
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .unwrap()
    }

    /// A coroutine that sets its request state, waits `wait`, and reports
    /// what it sees afterwards.
    fn request(
        id: u64,
        name: &'static str,
        wait: Duration,
        handle: tokio::runtime::Handle,
    ) -> Coroutine<(), (), Option<&'static str>, DefaultStack> {
        Coroutine::with_stack(
            DefaultStack::new(256 * 1024).unwrap(),
            move |yielder: &CoYielder, ()| {
                CURRENT.with(|c| {
                    c.set(Some(Current {
                        yielder: yielder as *const CoYielder,
                        id,
                    }))
                });
                WHO.set(Some(name));
                assert!(can_yield());
                drive(&handle, async move { tokio::time::sleep(wait).await });
                let seen = WHO.with(|w| *w.borrow());
                CURRENT.with(|c| c.set(None));
                seen
            },
        )
    }

    #[test]
    fn a_suspended_request_gets_its_own_state_back() {
        std::thread::spawn(|| {
            let rt = runtime();
            let (tx, rx) = channel::unbounded();
            READY_TX.with(|t| *t.borrow_mut() = Some(tx));

            let mut a = request(1, "a", Duration::from_millis(80), rt.handle().clone());
            let mut b = request(2, "b", Duration::from_millis(10), rt.handle().clone());
            assert!(
                matches!(a.resume(()), CoroutineResult::Yield(())),
                "A waits"
            );
            assert_eq!(WHO.with(|w| *w.borrow()), None, "A's state is put away");
            assert!(
                matches!(b.resume(()), CoroutineResult::Yield(())),
                "B waits"
            );

            // B's wait ends first: it finishes with its own state.
            assert_eq!(rx.recv_timeout(Duration::from_secs(2)).unwrap(), 2);
            assert!(matches!(b.resume(()), CoroutineResult::Return(Some("b"))));
            // Then A, which must see "a", not what B left behind.
            assert_eq!(rx.recv_timeout(Duration::from_secs(2)).unwrap(), 1);
            assert!(matches!(a.resume(()), CoroutineResult::Return(Some("a"))));
        })
        .join()
        .unwrap();
    }

    #[test]
    fn routes_group_their_ids_and_keep_their_names_apart() {
        assert_eq!(
            route_key("GET", "/orders/123/pay"),
            route_key("GET", "/orders/987/pay")
        );
        assert_eq!(
            route_key("GET", "/u/01a189d6-c9bf-70a5-af36-4ef5a15607aa"),
            route_key("GET", "/u/01a18c08-1009-7b27-b365-8afd1afc150e")
        );
        assert_eq!(route_key("GET", "/io?id=1"), route_key("GET", "/io?id=2"));
        assert_ne!(
            route_key("GET", "/orders/1/pay"),
            route_key("GET", "/orders/1/refund")
        );
        assert_ne!(route_key("GET", "/io"), route_key("POST", "/io"));
    }

    #[test]
    fn a_direct_request_that_blocks_on_a_wait_is_noted_once_allowed() {
        std::thread::spawn(|| {
            note_blocking_wait();
            assert!(!WOULD_WAIT.with(|w| w.get()), "nothing runs directly");
            DIRECT.with(|d| d.set(true));
            {
                let _guard = NoYield::enter();
                note_blocking_wait();
                assert!(!WOULD_WAIT.with(|w| w.get()), "a render could not suspend");
            }
            note_blocking_wait();
            assert!(WOULD_WAIT.with(|w| w.get()));
        })
        .join()
        .unwrap();
    }

    #[test]
    fn nothing_yields_outside_a_coroutine_or_inside_a_no_yield_section() {
        std::thread::spawn(|| {
            assert!(!can_yield());
            let rt = runtime();
            let (tx, _rx) = channel::unbounded();
            READY_TX.with(|t| *t.borrow_mut() = Some(tx));
            let mut co: Coroutine<(), (), bool, DefaultStack> = Coroutine::with_stack(
                DefaultStack::new(256 * 1024).unwrap(),
                |yielder: &CoYielder, ()| {
                    CURRENT.with(|c| {
                        c.set(Some(Current {
                            yielder: yielder as *const CoYielder,
                            id: 9,
                        }))
                    });
                    let _guard = NoYield::enter();
                    let blocked = !can_yield();
                    CURRENT.with(|c| c.set(None));
                    blocked
                },
            );
            assert!(matches!(co.resume(()), CoroutineResult::Return(true)));
            drop(rt);
        })
        .join()
        .unwrap();
    }
}
