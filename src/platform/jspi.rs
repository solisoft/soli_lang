//! Drive a future to completion from synchronous code on wasm32, which
//! has no thread to block. Each time the future is pending, `soli_yield` — an
//! import the host wraps in `WebAssembly.Suspending` — suspends this wasm stack
//! and returns to the event loop, so the `fetch` the future waits on can make
//! progress; the stack resumes when the host's promise settles, and polls again.
//! The export that reached here must have been entered through
//! `WebAssembly.promising`, or suspending throws.

use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll, Waker};

#[link(wasm_import_module = "./jspi.js")]
extern "C" {
    fn soli_yield();
}

pub fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    let mut cx = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(output) = future.as_mut().poll(&mut cx) {
            return output;
        }
        unsafe { soli_yield() }
    }
}
