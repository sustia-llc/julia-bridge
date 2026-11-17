//! Run with: cargo nextest run --features tokio
//!
//! This file holds one test function, which closes the shared Julia handle.
#![cfg(feature = "tokio")]

use std::time::Duration;

use julia_bridge::shared;

#[tokio::test(flavor = "multi_thread")]
async fn close_and_wait_twice_returns_ok_both_times() {
    assert_eq!(shared::call_base1("sqrt", 4.0).await.unwrap(), 2.0);

    let first = tokio::time::timeout(Duration::from_secs(30), shared::close_and_wait())
        .await
        .expect("first close_and_wait did not return within 30 s");
    let closed = shared::julia_if_started().map(|handle| handle.is_closed());
    let second = tokio::time::timeout(Duration::from_secs(30), shared::close_and_wait())
        .await
        .expect("second close_and_wait did not return within 30 s");

    assert!(first.is_ok(), "first close_and_wait: got {first:?}");
    assert_eq!(closed, Some(true), "handle closed after close_and_wait");
    assert!(second.is_ok(), "second close_and_wait: got {second:?}");
}
