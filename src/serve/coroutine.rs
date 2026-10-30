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
    CURRENT.with(|c| c.get().is_some())
        && NO_YIELD.with(|n| n.get() == 0)
        && !crate::interpreter::builtins::model::crud::has_active_tx()
        && !crate::interpreter::builtins::model::batch::in_block()
        && !crate::interpreter::value::formatting_in_progress()
}

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
        GLOBAL_ENV.with(|g| *g.borrow_mut() = Some(global_env));
        Self {
            max: concurrency(),
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
        interpreter: &Interpreter,
        worker_vm: &mut Option<crate::vm::Vm>,
        data: super::RequestData,
        dev_mode: bool,
    ) {
        let id = self.next_id;
        self.next_id += 1;
        let mut shell = interpreter.coroutine_shell();
        let mut vm = worker_vm.as_ref().map(|worker| match self.vms.pop() {
            // A VM back from an earlier request already holds the worker's
            // globals; the request names (`req`, `params`, `cookies`) are
            // written by the handler call. Copying the globals again is most
            // of what a request on a coroutine costs.
            Some(mut vm) => {
                vm.failed_handlers.clone_from(&worker.failed_handlers);
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
                        worker.failed_handlers.extend(vm.failed_handlers.drain());
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
