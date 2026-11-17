//! A process-wide Julia runtime for tokio applications (feature `tokio`).
//!
//! Julia can be initialized only once per process. Instead of scoping the runtime to a
//! closure like [`crate::run`], this module starts it lazily on a dedicated background
//! thread the first time it's needed and keeps it for the life of the process. Any
//! tokio task, on any thread, in any test, sends work to it and awaits the result.
//!
//! Don't mix this with [`crate::run`] in one process: whichever starts Julia first wins
//! and the other fails.

use std::sync::OnceLock;

use jlrs::{error::JlrsError, prelude::*};

pub use jlrs::runtime::handle::async_handle::AsyncHandle;

/// How many Julia tasks may be in flight on the runtime thread at once.
const MAX_CONCURRENT_TASKS: usize = 16;

static JULIA: OnceLock<AsyncHandle> = OnceLock::new();

/// The shared handle. Starts Julia on first use; later calls return the same handle.
///
/// The first call blocks for the few seconds Julia takes to start. In an async context,
/// either call it once before starting your tasks or accept that one-off stall.
pub fn julia() -> &'static AsyncHandle {
    JULIA.get_or_init(|| {
        let (handle, _runtime_thread) = Builder::new()
            // `false`: don't enable tokio's IO driver on the Julia thread.
            .async_runtime(Tokio::<MAX_CONCURRENT_TASKS>::new(false))
            .spawn()
            .expect("failed to start Julia");
        // The runtime thread exits when the last handle is dropped. This one lives in a
        // static, so Julia stays up until the process exits.
        handle
    })
}

/// Evaluate a snippet of Julia code and return its result as an `f64`.
///
/// Runs on the Julia runtime thread as a single unit, so keep the snippet short. For
/// anything long-running use [`call_base1`], which lets other tasks make progress.
pub async fn eval_f64(code: impl Into<String>) -> JlrsResult<f64> {
    let code = code.into();
    let receiver = julia()
        .task(async move |mut frame: AsyncGcFrame| -> JlrsResult<f64> {
            // Safety: evaluating arbitrary code is only as safe as the code itself.
            let value = unsafe { Value::eval_string(&mut frame, &code) }?;
            value.unbox::<f64>()
        })
        .dispatch()
        .await?;

    receiver.await.map_err(JlrsError::other)?
}

/// Call a one-argument function from Julia's `Base` module, e.g. `"sqrt"`.
///
/// The call is scheduled on one of Julia's own threads with `call_async`, so the
/// runtime thread stays free to start other tasks while this one computes.
pub async fn call_base1(func: impl Into<String>, x: f64) -> JlrsResult<f64> {
    let func = func.into();
    let receiver = julia()
        .task(async move |mut frame: AsyncGcFrame| -> JlrsResult<f64> {
            let x = Value::new(&mut frame, x);
            let func: Value = Module::base(&frame).global(&mut frame, &func)?;
            // Safety: only as safe as the Julia function being called.
            let out = unsafe { func.call_async(&mut frame, [x]) }.await?;
            out.unbox::<f64>()
        })
        .dispatch()
        .await?;

    receiver.await.map_err(JlrsError::other)?
}
