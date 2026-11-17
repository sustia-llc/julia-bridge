//! Run with: cargo run --features tokio --example tokio_demo

use julia_bridge::shared::{call_base1, eval_f64, julia};

#[tokio::main]
async fn main() {
    // Start Julia up front, off the async worker threads, so the first request doesn't
    // pay for it.
    tokio::task::spawn_blocking(julia).await.unwrap();

    println!("eval   sqrt(2) = {}", eval_f64("sqrt(2.0)").await.unwrap());

    // Eight tokio tasks, all talking to the one Julia runtime concurrently.
    let tasks: Vec<_> = (1..=8)
        .map(|i| tokio::spawn(call_base1("sqrt", f64::from(i))))
        .collect();

    for (i, task) in (1..=8).zip(tasks) {
        println!("task   sqrt({i}) = {}", task.await.unwrap().unwrap());
    }
}
