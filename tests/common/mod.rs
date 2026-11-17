//! The slow Julia workload and the handshakes shared by the lifecycle tests.
#![allow(dead_code)]

use std::{
    future::{Future, poll_fn},
    pin::pin,
    sync::{
        LazyLock, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    task::Poll,
    time::{Duration, Instant},
};

use julia_bridge::shared;
use tokio::sync::{OnceCell, mpsc, watch};

/// The time after which [`within`] panics.
pub const HANG_TIMEOUT: Duration = Duration::from_secs(30);

/// Awaits `step` and returns its output, or panics with `what` if `step` has not
/// returned within [`HANG_TIMEOUT`].
pub async fn within<T>(what: &str, step: impl Future<Output = T>) -> T {
    match tokio::time::timeout(HANG_TIMEOUT, step).await {
        Ok(out) => out,
        Err(_) => panic!("{what} did not return within {HANG_TIMEOUT:?}"),
    }
}

/// The number of calls in a full-drain workload.
pub const K: usize = 64;
/// The name in `Base` of the slow function.
pub const SLOW_FN: &str = "julia_bridge_slow";
/// The sleep of one call in a full-drain workload, in seconds.
pub const SLOW_SECS: f64 = 0.02;

/// The number of `julia_bridge_slow` calls that have started in this process.
static STARTED: LazyLock<watch::Sender<usize>> = LazyLock::new(|| watch::channel(0).0);

/// For each `julia_bridge_slow` call that reached its end, whether the shared handle
/// was closed and how many calls had started at that moment.
static ENDS: Mutex<Vec<(bool, usize)>> = Mutex::new(Vec::new());

/// For each `julia_bridge_slow` call that reached its end, an instant read before the
/// call returned.
static END_INSTANTS: Mutex<Vec<Instant>> = Mutex::new(Vec::new());

static SLOW_DEFINED: OnceCell<()> = OnceCell::const_new();

/// The name in `Base` of the function that returns when the shared handle is closed.
pub const UNTIL_CLOSED_FN: &str = "julia_bridge_until_closed";
/// The name in `Base` of the function that returns when [`release`] has been called.
pub const UNTIL_RELEASED_FN: &str = "julia_bridge_until_released";

/// Whether [`release`] has been called in this process.
static RELEASED: AtomicBool = AtomicBool::new(false);

static WAITS_DEFINED: OnceCell<()> = OnceCell::const_new();

/// Called by `julia_bridge_until_closed`: 1 if the shared handle is closed, else 0.
extern "C" fn handle_closed() -> i32 {
    i32::from(shared::julia_if_started().is_some_and(|handle| handle.is_closed()))
}

/// Called by `julia_bridge_until_released`: 1 if [`release`] has been called, else 0.
extern "C" fn released() -> i32 {
    i32::from(RELEASED.load(Ordering::SeqCst))
}

/// Returns whether the shared handle is closed, or `None` if Julia has not been
/// started; it does not start Julia.
pub fn shared_handle_closed() -> Option<bool> {
    shared::julia_if_started().map(|handle| handle.is_closed())
}

/// Makes `julia_bridge_until_released` return.
pub fn release() {
    RELEASED.store(true, Ordering::SeqCst);
}

/// Defines `Base.julia_bridge_until_closed(x)` and `Base.julia_bridge_until_released(x)`
/// once per process: each returns 1.0 when its condition holds, and 0.0 if the
/// condition does not hold within `x` seconds.
pub async fn define_waits() {
    WAITS_DEFINED
        .get_or_init(|| async {
            let conditions = [
                (
                    UNTIL_CLOSED_FN,
                    handle_closed as extern "C" fn() -> i32 as usize,
                ),
                (
                    UNTIL_RELEASED_FN,
                    released as extern "C" fn() -> i32 as usize,
                ),
            ];
            for (name, condition) in conditions {
                let definition = format!(
                    "Core.eval(Base, :(function {name}(x::Float64)
                         t = time()
                         while ccall(Ptr{{Cvoid}}(UInt({condition})), Cint, ()) == 0
                             time() - t < x || return 0.0
                             sleep(0.001)
                         end
                         1.0
                     end)); 1.0"
                );
                let defined = within(name, shared::eval_f64(definition)).await.unwrap();
                assert_eq!(defined, 1.0);
            }
        })
        .await;
}

/// Called by `julia_bridge_slow` before it sleeps.
extern "C" fn slow_started() {
    STARTED.send_modify(|count| *count += 1);
}

/// Called by `julia_bridge_slow` after it sleeps.
extern "C" fn slow_ending() {
    let closed = shared::julia_if_started().is_some_and(|handle| handle.is_closed());
    let started = *STARTED.borrow();
    if let Ok(mut ends) = ENDS.lock() {
        ends.push((closed, started));
    }
    if let Ok(mut instants) = END_INSTANTS.lock() {
        instants.push(Instant::now());
    }
}

/// Defines `Base.julia_bridge_slow(x)` once per process: it calls `slow_started`,
/// sleeps `x` seconds, calls `slow_ending` and returns `x`.
pub async fn define_slow() {
    SLOW_DEFINED
        .get_or_init(|| async {
            let started = slow_started as extern "C" fn() as usize;
            let ending = slow_ending as extern "C" fn() as usize;
            let definition = format!(
                "Core.eval(Base, :(julia_bridge_slow(x::Float64) = (\
                 ccall(Ptr{{Cvoid}}(UInt({started})), Cvoid, ()); \
                 sleep(x); \
                 ccall(Ptr{{Cvoid}}(UInt({ending})), Cvoid, ()); \
                 x))); 1.0"
            );
            let defined = within(SLOW_FN, shared::eval_f64(definition)).await.unwrap();
            assert_eq!(defined, 1.0);
        })
        .await;
}

/// Returns the number of `julia_bridge_slow` calls that have started in this process.
pub fn started_count() -> usize {
    *STARTED.borrow()
}

/// Returns when at least `n` `julia_bridge_slow` calls have started in this process.
pub async fn wait_started(n: usize) {
    STARTED
        .subscribe()
        .wait_for(|count| *count >= n)
        .await
        .unwrap();
}

/// Returns, for each `julia_bridge_slow` call that reached its end, in order, whether
/// the shared handle was closed and how many calls had started at that moment.
pub fn ends() -> Vec<(bool, usize)> {
    ENDS.lock().unwrap().clone()
}

/// Returns, for each `julia_bridge_slow` call that reached its end, in order, an instant
/// read before the call returned.
pub fn end_instants() -> Vec<Instant> {
    END_INSTANTS.lock().unwrap().clone()
}

/// Returns, for each `julia_bridge_slow` call that reached its end, whether the shared
/// handle was closed at that moment.
pub fn closed_at_end() -> Vec<bool> {
    ends().into_iter().map(|(closed, _)| closed).collect()
}

/// Polls `call` once, sends on `admitted`, then awaits `call` to its end.
pub async fn admitted_then<T>(
    call: impl Future<Output = T>,
    admitted: mpsc::UnboundedSender<()>,
) -> T {
    let mut call = pin!(call);
    let first = poll_fn(|cx| Poll::Ready(call.as_mut().poll(cx))).await;
    admitted.send(()).unwrap();
    match first {
        Poll::Ready(out) => out,
        Poll::Pending => call.await,
    }
}
