//! A process-wide Julia runtime for tokio applications (feature `tokio`).
//!
//! Julia can be initialized only once per process. Instead of scoping the runtime to a
//! closure like [`crate::run`], this module starts it lazily on a dedicated background
//! thread the first time it's needed and keeps it for the life of the process. Any
//! tokio task, on any thread, in any test, sends work to it and awaits the result.
//!
//! Don't mix this with [`crate::run`] in one process: whichever starts Julia first wins
//! and the other fails.

use std::{
    any::Any,
    sync::{Mutex, MutexGuard, OnceLock, PoisonError, mpsc},
    thread::JoinHandle,
};

use jlrs::{
    error::{JlrsError, RuntimeError},
    prelude::*,
};
use tokio::sync::watch;

pub use jlrs::runtime::handle::async_handle::AsyncHandle;

/// How many Julia tasks may be in flight on the runtime thread at once.
pub const MAX_CONCURRENT_TASKS: usize = 16;

/// How the Julia runtime thread ended: `Ok` on a return, `Err` with its panic message
/// on a panic.
type Ended = Result<(), String>;

/// What the one lock of this module guards.
struct Runtime {
    /// The process-wide closed flag, set by [`close`].
    closed: bool,
    /// The ended signal of the Julia runtime thread, present once Julia has been
    /// started: `None` while the thread runs, then how it ended.
    ended: Option<watch::Receiver<Option<Ended>>>,
}

static JULIA: OnceLock<AsyncHandle> = OnceLock::new();

static RUNTIME: Mutex<Runtime> = Mutex::new(Runtime {
    closed: false,
    ended: None,
});

/// Locks the state of this module.
fn lock() -> MutexGuard<'static, Runtime> {
    RUNTIME.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Returns the message of a panic payload if it is a `String` or a `&str`, and a fixed
/// text otherwise.
fn panic_message(payload: &(dyn Any + Send)) -> String {
    payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| {
            payload
                .downcast_ref::<&str>()
                .map(|text| (*text).to_owned())
        })
        .unwrap_or_else(|| "the panic payload is not a string".to_owned())
}

/// Returns the error that reports a panic of the Julia runtime thread with `message`.
fn panicked_error(message: &str) -> Box<JlrsError> {
    Box::new(JlrsError::exception(format!(
        "the Julia runtime thread panicked: {message}"
    )))
}

/// Returns the shared handle and, on first use, starts a watcher thread, then starts
/// Julia and sends the watcher the join handle of the Julia runtime thread, and stores
/// the receiver of the ended signal in `runtime`; the watcher joins the runtime thread
/// and then sends how it ended on that signal, or ends without a signal if Julia did
/// not start.
fn start(runtime: &mut Runtime) -> JlrsResult<&'static AsyncHandle> {
    if let Some(handle) = JULIA.get() {
        return Ok(handle);
    }
    let (thread_tx, thread_rx) = mpsc::channel::<JoinHandle<()>>();
    let (ended_tx, ended_rx) = watch::channel(None);
    std::thread::Builder::new()
        .name("julia-runtime-watcher".into())
        .spawn(move || {
            let Ok(thread) = thread_rx.recv() else {
                return;
            };
            let ended = thread
                .join()
                .map_err(|payload| panic_message(payload.as_ref()));
            ended_tx.send_replace(Some(ended));
        })
        .map_err(|error| Box::new(JlrsError::other(error)))?;
    let (handle, thread) = Builder::new()
        // `false`: don't enable tokio's IO driver on the Julia thread.
        .async_runtime(Tokio::<MAX_CONCURRENT_TASKS>::new(false))
        .spawn()?;
    thread_tx
        .send(thread)
        .expect("invariant: the watcher thread holds the receiver until it has received");
    runtime.ended = Some(ended_rx);
    // The runtime thread exits when the handle is closed and its calls have returned.
    // The handle lives in a static, so it is never dropped.
    Ok(JULIA.get_or_init(|| handle))
}

/// Sets the closed flag in `runtime` and calls `close(false)` on the shared handle if
/// Julia has been started.
fn close_locked(runtime: &mut Runtime) {
    runtime.closed = true;
    if let Some(handle) = JULIA.get() {
        handle.close(false);
    }
}

/// Returns the shared handle and starts Julia on first use, also after [`close`] or
/// [`close_and_wait`], in which case the handle it starts is open; later calls return
/// the same handle; it panics if Julia does not start.
pub fn julia() -> &'static AsyncHandle {
    if let Some(handle) = JULIA.get() {
        return handle;
    }
    let started = start(&mut lock());
    started.expect("failed to start Julia")
}

/// Returns the shared handle if Julia has been started, and `None` otherwise; it does
/// not start Julia.
pub fn julia_if_started() -> Option<&'static AsyncHandle> {
    JULIA.get()
}

/// Sets the process-wide closed flag, after which [`eval_f64`] and [`call_base1`]
/// return an error without starting Julia, and calls `close(false)` on the shared
/// handle if Julia has been started; a first start of Julia that is in progress ends
/// before this function proceeds.
pub fn close() {
    close_locked(&mut lock());
}

/// Does what [`close`] does, then waits, without blocking the thread that polls it,
/// until the Julia runtime thread has ended, which is after every call that the handle
/// accepted has returned; it returns at once if Julia was never started or the thread
/// has already ended, and it returns an error with the panic message if the runtime
/// thread panicked.
pub async fn close_and_wait() -> JlrsResult<()> {
    let ended = {
        let mut runtime = lock();
        close_locked(&mut runtime);
        runtime.ended.clone()
    };
    let Some(mut ended) = ended else {
        return Ok(());
    };
    let ended = ended
        .wait_for(Option::is_some)
        .await
        .map_err(|error| Box::new(JlrsError::other(error)))?
        .clone();
    match ended {
        Some(Err(message)) => Err(panicked_error(&message)),
        _ => Ok(()),
    }
}

/// Returns the shared handle, starting Julia on first use, or the panic error if the
/// Julia runtime thread panicked, or else a channel-closed error if the closed flag is
/// set, or the error of a start of Julia that failed.
fn open_handle() -> JlrsResult<&'static AsyncHandle> {
    let mut runtime = lock();
    if let Some(ended) = &runtime.ended
        && let Some(Err(message)) = &*ended.borrow()
    {
        return Err(panicked_error(message));
    }
    if runtime.closed {
        Err(RuntimeError::ChannelClosed)?;
    }
    start(&mut runtime)
}

/// Evaluate a snippet of Julia code and return its result as an `f64`.
///
/// Runs on the Julia runtime thread as a single unit, so keep the snippet short. For
/// anything long-running use [`call_base1`], which lets other tasks make progress.
pub async fn eval_f64(code: impl Into<String>) -> JlrsResult<f64> {
    let code = code.into();
    let receiver = open_handle()?
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
    let receiver = open_handle()?
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panic_message_reads_string_and_str_payloads() {
        let string =
            std::panic::catch_unwind(|| std::panic::panic_any(String::from("formatted 7")))
                .unwrap_err();
        let literal = std::panic::catch_unwind(|| std::panic::panic_any("literal")).unwrap_err();
        let other = std::panic::catch_unwind(|| std::panic::panic_any(7_u8)).unwrap_err();

        assert_eq!(panic_message(string.as_ref()), "formatted 7");
        assert_eq!(panic_message(literal.as_ref()), "literal");
        assert_eq!(
            panic_message(other.as_ref()),
            "the panic payload is not a string"
        );
        assert_eq!(
            panicked_error("literal").to_string(),
            "Exception: the Julia runtime thread panicked: literal"
        );
    }
}
