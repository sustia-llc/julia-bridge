//! Run with: cargo nextest run --features tokio
//!
//! This file holds one test function, which starts Julia with `julia_bridge::run`
//! before it uses the shared handle.
#![cfg(feature = "tokio")]

use std::time::Duration;

use jlrs::error::{JlrsError, RuntimeError};
use julia_bridge::shared;

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_start_is_an_error() {
    julia_bridge::run(1, |_handle| ()).unwrap();

    let call = shared::call_base1("sqrt", 4.0).await;
    eprintln!("start failed: call resolved to {call:?}");
    assert!(
        matches!(
            &call,
            Err(error) if matches!(**error, JlrsError::RuntimeError(RuntimeError::AlreadyInitialized))
        ),
        "call after Julia was started by run: got {call:?}, expected Err(AlreadyInitialized)"
    );

    let waited = tokio::time::timeout(Duration::from_secs(30), shared::close_and_wait())
        .await
        .expect("close_and_wait did not return within 30 s");
    assert!(
        waited.is_ok(),
        "close_and_wait after the failed start: got {waited:?}, expected Ok"
    );
}
