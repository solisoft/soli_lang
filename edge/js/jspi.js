// The one import Soli suspends on (see solilang::platform::jspi). Called from
// wasm while a future is pending: it parks the wasm stack and yields a
// macrotask, so the fetch that future waits on can settle; the stack resumes
// when this promise does, and polls again.
export const soli_yield = new WebAssembly.Suspending(
  () => new Promise((resolve) => setTimeout(resolve, 0)),
);
