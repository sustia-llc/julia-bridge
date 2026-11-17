//! Run with: cargo nextest run --features tokio
//!
//! This file holds one test function, which closes the shared Julia handle.
#![cfg(feature = "tokio")]

mod common;

use common::{
    SLOW_FN, UNTIL_CLOSED_FN, UNTIL_RELEASED_FN, admitted_then, define_slow, define_waits, ends,
    release, within,
};
use julia_bridge::shared;
use tokio::sync::{mpsc, oneshot};

const SLOW_SECS: f64 = 0.5;
/// The time after which each waiting Julia function gives up, in seconds.
const CAP_SECS: f64 = 10.0;

#[tokio::test(flavor = "current_thread")]
async fn close_and_wait_returns_after_running_calls_and_leaves_the_runtime_free() {
    define_slow().await;
    define_waits().await;

    let (echo_tx, mut echo_rx) = mpsc::unbounded_channel::<oneshot::Sender<()>>();
    tokio::spawn(async move {
        while let Some(reply) = echo_rx.recv().await {
            reply.send(()).unwrap();
        }
    });

    let (admitted_tx, mut admitted_rx) = mpsc::unbounded_channel();
    let slow = tokio::spawn(admitted_then(
        shared::call_base1(SLOW_FN, SLOW_SECS),
        admitted_tx.clone(),
    ));
    let until_released = tokio::spawn(admitted_then(
        shared::call_base1(UNTIL_RELEASED_FN, CAP_SECS),
        admitted_tx.clone(),
    ));
    let until_closed = tokio::spawn({
        let admitted_tx = admitted_tx.clone();
        async move {
            let call = shared::call_base1(UNTIL_CLOSED_FN, CAP_SECS);
            let out = admitted_then(call, admitted_tx).await;
            let (reply_tx, reply_rx) = oneshot::channel();
            echo_tx.send(reply_tx).unwrap();
            reply_rx.await.unwrap();
            release();
            out
        }
    });
    drop(admitted_tx);
    for _ in 0..3 {
        within("the first poll of a call", admitted_rx.recv())
            .await
            .unwrap();
    }

    let waited = within("close_and_wait", shared::close_and_wait()).await;
    let ends_at_return = ends();
    let slow_call = within("the slow call", slow).await.unwrap();
    let closed_call = within("the call that waits for the closed handle", until_closed)
        .await
        .unwrap();
    let released_call = within("the call that waits for the release", until_released)
        .await
        .unwrap();

    assert!(
        waited.is_ok(),
        "close_and_wait: got {waited:?}, expected Ok"
    );
    assert_eq!(
        ends_at_return.len(),
        1,
        "slow calls that had ended when close_and_wait returned"
    );
    assert!(
        matches!(slow_call, Ok(x) if x == SLOW_SECS),
        "slow call: got {slow_call:?}, expected Ok(0.5)"
    );
    assert!(
        matches!(closed_call, Ok(x) if x == 1.0),
        "call that waits for the closed handle: got {closed_call:?}, expected Ok(1.0)"
    );
    assert!(
        matches!(released_call, Ok(x) if x == 1.0),
        "call that waits for the channel round trip of another task: got {released_call:?}, expected Ok(1.0), released while close_and_wait was pending"
    );
}
