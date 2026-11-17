//! Run with: cargo nextest run --features lifecycle
//!
//! This file holds one test function, which shuts a `Strategy::Close` lifecycle down.
#![cfg(feature = "lifecycle")]

use std::{
    future::Future,
    pin::pin,
    sync::{Arc, OnceLock},
    task::{Context, Wake, Waker},
    time::Duration,
};

use julia_bridge::lifecycle::{Lifecycle, Strategy};

/// Records, at its first wake, whether the tracker of `lifecycle` is closed.
struct RecordClosed {
    lifecycle: Arc<Lifecycle>,
    closed_at_wake: OnceLock<bool>,
}

impl Wake for RecordClosed {
    fn wake(self: Arc<Self>) {
        self.closed_at_wake
            .get_or_init(|| self.lifecycle.is_closed());
    }
}

#[tokio::test]
async fn close_cancels_the_root_token_before_it_closes_the_tracker() {
    let lifecycle = Arc::new(Lifecycle::new(Strategy::Close));
    let sibling = Lifecycle::new(Strategy::Drain);
    let record = Arc::new(RecordClosed {
        lifecycle: Arc::clone(&lifecycle),
        closed_at_wake: OnceLock::new(),
    });
    let waker = Waker::from(Arc::clone(&record));
    let mut root_cancelled = pin!(sibling.cancellation_token().cancelled());
    let first = root_cancelled
        .as_mut()
        .poll(&mut Context::from_waker(&waker));
    assert!(first.is_pending(), "root token cancelled before shutdown");
    assert!(!lifecycle.is_closed(), "tracker closed before shutdown");

    tokio::time::timeout(
        Duration::from_secs(30),
        lifecycle.shutdown(Duration::from_secs(1)),
    )
    .await
    .expect("shutdown did not return within 30 s")
    .unwrap();

    assert_eq!(
        record.closed_at_wake.get(),
        Some(&false),
        "tracker of the Close lifecycle closed when the root cancel woke the waiter on the token of another lifecycle: expected Some(false)"
    );
    assert!(
        lifecycle.is_closed(),
        "tracker after shutdown: got open, expected closed"
    );
}
