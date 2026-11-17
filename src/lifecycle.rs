//! Admission, tracking and shutdown of calls to the shared Julia runtime (feature `lifecycle`).

use std::{
    future::poll_fn,
    pin::pin,
    sync::LazyLock,
    task::Poll,
    time::{Duration, Instant},
};

use jlrs::error::{JlrsError, JlrsResult};
use tokio::task::JoinError;
use tokio_util::{sync::CancellationToken, task::TaskTracker};

use crate::shared;

/// The process-wide root token, parent of the token of every [`Lifecycle`].
static ROOT: LazyLock<CancellationToken> = LazyLock::new(CancellationToken::new);

/// Which token [`Lifecycle::shutdown`] cancels at entry, and what it does to the shared
/// Julia handle after its wait for tracked calls ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Strategy {
    /// Cancels the token of this lifecycle and leaves the shared handle open.
    Drain,
    /// Cancels the process-wide root token, which cancels the token of every
    /// lifecycle in the process, and after the wait runs [`shared::close_and_wait`] in
    /// a tokio task and awaits it, whether or not every tracked call returned before
    /// the drain timeout.
    Close,
}

/// The error of a call or a shutdown made through a [`Lifecycle`].
#[derive(Debug, thiserror::Error)]
pub enum LifecycleError {
    /// The cancellation token was cancelled when the call was made.
    #[error("the lifecycle is cancelled")]
    Cancelled,
    /// jlrs returned an error for the call.
    #[error(transparent)]
    Julia(#[from] Box<JlrsError>),
    /// The tokio task that ran the call, the shutdown steps or the close step panicked
    /// or was aborted.
    #[error("the lifecycle task failed")]
    Join(#[from] JoinError),
}

/// The result of one [`Lifecycle::shutdown`] call.
#[derive(Debug, Clone)]
pub struct ShutdownReport {
    /// Whether every tracked call returned before the drain timeout.
    pub drained: bool,
    /// The number of tracked calls, read after `drained` is decided; it can include a
    /// call that is being rejected with [`LifecycleError::Cancelled`].
    pub remaining: usize,
    /// The result of the close step: `Ok` under [`Strategy::Drain`], and under
    /// [`Strategy::Close`] what [`shared::close_and_wait`] returned.
    pub close: JlrsResult<()>,
    /// The time from entry of `shutdown` to its return, which under
    /// [`Strategy::Close`] includes the await of [`shared::close_and_wait`].
    pub elapsed: Duration,
}

/// A child of the process-wide root cancellation token, a tracker of in-flight calls
/// and a shutdown [`Strategy`] for calls to the shared Julia runtime.
#[derive(Debug)]
pub struct Lifecycle {
    token: CancellationToken,
    tracker: TaskTracker,
    tasks: TaskTracker,
    strategy: Strategy,
}

impl Lifecycle {
    /// Creates a lifecycle with a new child of the process-wide root token, an empty
    /// open tracker and `strategy`.
    pub fn new(strategy: Strategy) -> Self {
        Self {
            token: ROOT.child_token(),
            tracker: TaskTracker::new(),
            tasks: TaskTracker::new(),
            strategy,
        }
    }

    /// Returns the token of this lifecycle, whose cancellation makes later calls
    /// return [`LifecycleError::Cancelled`].
    pub fn cancellation_token(&self) -> &CancellationToken {
        &self.token
    }

    /// Returns the strategy given to [`Lifecycle::new`].
    pub fn strategy(&self) -> Strategy {
        self.strategy
    }

    /// Returns whether [`Lifecycle::shutdown`] has closed the tracker of this
    /// lifecycle.
    pub fn is_closed(&self) -> bool {
        self.tracker.is_closed()
    }

    /// Runs [`shared::call_base1`] with `func` and `x` in a tokio task tracked until
    /// the call returns, or returns [`LifecycleError::Cancelled`] without a dispatch
    /// if the token is cancelled at the first poll; it must be polled in a tokio
    /// runtime context.
    pub async fn call_base1(&self, func: impl Into<String>, x: f64) -> Result<f64, LifecycleError> {
        self.admit(shared::call_base1(func.into(), x)).await
    }

    /// Runs [`shared::eval_f64`] with `code` in a tokio task tracked until the call
    /// returns, or returns [`LifecycleError::Cancelled`] without a dispatch if the
    /// token is cancelled at the first poll; it must be polled in a tokio runtime
    /// context.
    pub async fn eval_f64(&self, code: impl Into<String>) -> Result<f64, LifecycleError> {
        self.admit(shared::eval_f64(code.into())).await
    }

    /// On a tokio runtime with the time driver, in its first poll: builds a timer of
    /// `drain_timeout`, cancels the token of this lifecycle, or the process-wide root
    /// token under [`Strategy::Close`], and closes the tracker; then, in a tokio task
    /// that it awaits: waits for the tracked calls to return or for the timer, reads
    /// the number of tracked calls and, under [`Strategy::Close`], runs
    /// [`shared::close_and_wait`] in a further tokio task, awaits it and stores what
    /// it returned in [`ShutdownReport::close`]; it returns `Err` only if one of
    /// these two tasks panicked or was aborted.
    pub async fn shutdown(
        &self,
        drain_timeout: Duration,
    ) -> Result<ShutdownReport, LifecycleError> {
        let entered = Instant::now();
        let timer = tokio::time::sleep(drain_timeout);
        match self.strategy {
            Strategy::Drain => self.token.cancel(),
            Strategy::Close => ROOT.cancel(),
        }
        self.tracker.close();
        let tracker = self.tracker.clone();
        let tasks = self.tasks.clone();
        let strategy = self.strategy;
        let rest = self.tasks.spawn(async move {
            let mut timer = pin!(timer);
            let mut wait = pin!(tracker.wait());
            let drained = poll_fn(|cx| {
                if wait.as_mut().poll(cx).is_ready() {
                    Poll::Ready(true)
                } else if timer.as_mut().poll(cx).is_ready() {
                    Poll::Ready(false)
                } else {
                    Poll::Pending
                }
            })
            .await;
            let remaining = tracker.len();
            let close = match strategy {
                Strategy::Drain => Ok(()),
                Strategy::Close => tasks.spawn(shared::close_and_wait()).await?,
            };
            Ok::<_, JoinError>((drained, remaining, close))
        });
        let (drained, remaining, close) = rest.await??;
        Ok(ShutdownReport {
            drained,
            remaining,
            close,
            elapsed: entered.elapsed(),
        })
    }

    /// In its first poll, tracks `call`, then returns [`LifecycleError::Cancelled`]
    /// without polling `call` if the token is cancelled, and otherwise spawns the
    /// tracked `call` as a tokio task; then awaits that task.
    async fn admit(
        &self,
        call: impl Future<Output = JlrsResult<f64>> + Send + 'static,
    ) -> Result<f64, LifecycleError> {
        let tracked = self.tracker.track_future(call);
        if self.token.is_cancelled() {
            return Err(LifecycleError::Cancelled);
        }
        Ok(tokio::spawn(tracked).await??)
    }
}

#[cfg(test)]
mod tests {
    use std::{
        future::Future,
        pin::pin,
        sync::{
            Arc, OnceLock,
            atomic::{AtomicBool, Ordering},
        },
        task::{Context, Wake, Waker},
    };

    use super::*;

    /// Records, at its first wake, whether `tracker` is closed.
    struct RecordClosed {
        tracker: TaskTracker,
        closed_at_wake: OnceLock<bool>,
    }

    impl Wake for RecordClosed {
        fn wake(self: Arc<Self>) {
            self.closed_at_wake.get_or_init(|| self.tracker.is_closed());
        }
    }

    #[tokio::test]
    async fn admit_on_a_cancelled_token_does_not_poll_the_call() {
        let lifecycle = Lifecycle::new(Strategy::Drain);
        lifecycle.cancellation_token().cancel();
        let polled = Arc::new(AtomicBool::new(false));
        let call = {
            let polled = Arc::clone(&polled);
            async move {
                polled.store(true, Ordering::SeqCst);
                Ok(1.0)
            }
        };

        let out = lifecycle.admit(call).await;
        let tracked = lifecycle.tracker.len();

        assert!(
            matches!(out, Err(LifecycleError::Cancelled)),
            "admit on a cancelled token: got {out:?}, expected Err(Cancelled)"
        );
        assert_eq!(tracked, 0, "tracked calls after the rejected call");
        assert!(
            !polled.load(Ordering::SeqCst),
            "call polled: got true, expected false"
        );
    }

    #[tokio::test]
    async fn one_poll_admits_the_call() {
        let lifecycle = Lifecycle::new(Strategy::Drain);
        let (release, released) = tokio::sync::oneshot::channel::<()>();
        let call = async move {
            released.await.unwrap();
            Ok(1.0)
        };
        let waker = Waker::noop();

        let mut admitted = pin!(lifecycle.admit(call));
        let first = admitted.as_mut().poll(&mut Context::from_waker(waker));
        assert!(first.is_pending(), "first poll of admit: got {first:?}");
        assert_eq!(lifecycle.tracker.len(), 1, "tracked calls after one poll");

        lifecycle.cancellation_token().cancel();
        release.send(()).unwrap();
        let out = admitted.await;

        assert!(
            matches!(out, Ok(x) if x == 1.0),
            "call polled once before the cancel: got {out:?}, expected Ok(1.0)"
        );
    }

    #[tokio::test]
    async fn first_poll_of_shutdown_cancels_the_token_and_closes_the_tracker() {
        let lifecycle = Lifecycle::new(Strategy::Drain);
        let waker = Waker::noop();

        let mut shutdown = pin!(lifecycle.shutdown(Duration::from_secs(1)));
        let first = shutdown.as_mut().poll(&mut Context::from_waker(waker));
        let cancelled = lifecycle.token.is_cancelled();
        let closed = lifecycle.tracker.is_closed();

        assert!(first.is_pending(), "first poll of shutdown: got {first:?}");
        assert!(
            cancelled,
            "token after one poll: got live, expected cancelled"
        );
        assert!(closed, "tracker after one poll: got open, expected closed");
        shutdown.await.unwrap();
    }

    #[test]
    fn shutdown_without_a_time_driver_panics_before_it_cancels() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let lifecycle = Lifecycle::new(Strategy::Drain);

        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            runtime.block_on(lifecycle.shutdown(Duration::from_secs(1)))
        }));

        let payload = outcome.expect_err("shutdown without a time driver returned");
        let message = payload
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| payload.downcast_ref::<&str>().copied())
            .unwrap_or("");
        assert!(
            message.contains("timers are disabled"),
            "panic message: got {message:?}, expected it to name the disabled timers"
        );
        assert!(
            !lifecycle.token.is_cancelled(),
            "token after the panic: got cancelled, expected live"
        );
        assert!(
            !lifecycle.tracker.is_closed(),
            "tracker after the panic: got closed, expected open"
        );
    }

    #[tokio::test]
    async fn shutdown_cancels_the_token_before_it_closes_the_tracker() {
        let lifecycle = Lifecycle::new(Strategy::Drain);
        let record = Arc::new(RecordClosed {
            tracker: lifecycle.tracker.clone(),
            closed_at_wake: OnceLock::new(),
        });
        let waker = Waker::from(Arc::clone(&record));
        let mut cancelled = pin!(lifecycle.token.cancelled());
        let first = cancelled.as_mut().poll(&mut Context::from_waker(&waker));
        assert!(first.is_pending(), "token cancelled before shutdown");

        lifecycle.shutdown(Duration::from_secs(1)).await.unwrap();

        assert_eq!(
            record.closed_at_wake.get(),
            Some(&false),
            "tracker closed when the token woke its waiter: expected Some(false)"
        );
    }
}
