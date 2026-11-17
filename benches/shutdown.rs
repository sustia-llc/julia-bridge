//! Run with: cargo bench --features lifecycle --bench shutdown
//!
//! One sample spawns `K` calls of `julia_bridge_bench_slow(0.02)`, waits for the first result
//! and calls `Lifecycle::shutdown(30 s)`. `Strategy::Drain` samples run under criterion
//! in this process. Each `Strategy::Close` sample runs in a child process: this
//! executable started again with `CHILD_ENV` set. The parent prints min, median and max
//! of each quantity for both strategies. The `runs` column counts, for `Drain`, every
//! iteration that criterion ran, warm-up included, and for `Close` the child processes.
//! In criterion's test mode the parent starts no child process and prints no table.
//! After its `Drain` samples the parent closes the shared handle and waits for the Julia
//! runtime thread.

use std::{
    io::Read,
    process::{Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

use criterion::{Criterion, SamplingMode};
use julia_bridge::{
    lifecycle::{Lifecycle, Strategy},
    shared,
};
use tokio::{runtime::Runtime, sync::mpsc};

const K: usize = 64;
const SLOW_FN: &str = "julia_bridge_bench_slow";
const SLOW_SECS: f64 = 0.02;
const DRAIN_TIMEOUT: Duration = Duration::from_secs(30);
const CLOSE_CHILDREN: usize = 20;
const CHILD_TIMEOUT: Duration = Duration::from_secs(120);
const CHILD_ENV: &str = "JULIA_BRIDGE_SHUTDOWN_CHILD";
const SAMPLE_TAG: &str = "SHUTDOWN_SAMPLE";

/// The quantities of one sample.
#[derive(Debug, Clone, Copy)]
struct Sample {
    /// `ShutdownReport::elapsed` in nanoseconds.
    shutdown_ns: u128,
    /// Calls that returned `Ok`, minus those that had returned `Ok` at entry of `shutdown`.
    completed_after_cancel: u128,
    /// Calls that returned `Err`.
    errored: u128,
    /// `ShutdownReport::remaining`.
    remaining: u128,
}

fn runtime() -> Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_time()
        .build()
        .expect("failed to build the tokio runtime")
}

/// Defines `Base.julia_bridge_bench_slow(x)`, which sleeps `x` seconds and returns `x`.
async fn define_slow() {
    let defined = shared::eval_f64(
        "Core.eval(Base, :(julia_bridge_bench_slow(x::Float64) = (sleep(x); x))); 1.0",
    )
    .await
    .expect("failed to define julia_bridge_bench_slow");
    assert_eq!(defined, 1.0);
}

/// Starts a thread that reads `pipe` to its end and returns the text.
fn read_pipe(pipe: Option<impl Read + Send + 'static>) -> JoinHandle<String> {
    let mut pipe = pipe.expect("invariant: the child was started with this pipe");
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        pipe.read_to_end(&mut bytes)
            .expect("failed to read a pipe of the child sample");
        String::from_utf8_lossy(&bytes).into_owned()
    })
}

async fn sample(strategy: Strategy) -> Sample {
    let lifecycle = Arc::new(Lifecycle::new(strategy));
    let ok_returned = Arc::new(AtomicUsize::new(0));
    let (result_tx, mut result_rx) = mpsc::unbounded_channel();
    for _ in 0..K {
        let lifecycle = Arc::clone(&lifecycle);
        let ok_returned = Arc::clone(&ok_returned);
        let result_tx = result_tx.clone();
        tokio::spawn(async move {
            let result = lifecycle.call_base1(SLOW_FN, SLOW_SECS).await;
            if result.is_ok() {
                ok_returned.fetch_add(1, Ordering::SeqCst);
            }
            result_tx
                .send(result.is_ok())
                .expect("the sample dropped its result receiver");
        });
    }
    drop(result_tx);

    let first = result_rx.recv().await.expect("no call returned");
    let ok_at_entry = ok_returned.load(Ordering::SeqCst);
    let report = lifecycle
        .shutdown(DRAIN_TIMEOUT)
        .await
        .expect("the shutdown task failed");
    assert!(
        report.close.is_ok(),
        "the close step failed: {:?}",
        report.close
    );

    let mut ok = usize::from(first);
    let mut errored = usize::from(!first);
    while let Some(returned_ok) = result_rx.recv().await {
        if returned_ok {
            ok += 1;
        } else {
            errored += 1;
        }
    }
    assert_eq!(ok + errored, K, "calls that returned");

    Sample {
        shutdown_ns: report.elapsed.as_nanos(),
        completed_after_cancel: (ok - ok_at_entry) as u128,
        errored: errored as u128,
        remaining: report.remaining as u128,
    }
}

/// Runs one `Strategy::Close` sample and prints it on one tagged line.
fn child() {
    let runtime = runtime();
    let sample = runtime.block_on(async {
        define_slow().await;
        sample(Strategy::Close).await
    });
    println!(
        "{SAMPLE_TAG} {} {} {} {}",
        sample.shutdown_ns, sample.completed_after_cancel, sample.errored, sample.remaining
    );
}

/// Starts this executable as a child, reads its stdout and stderr while it waits at
/// most `CHILD_TIMEOUT` for it to exit with success, and parses the tagged line it
/// prints.
fn close_sample() -> Sample {
    let exe = std::env::current_exe().expect("failed to read the path of this executable");
    let mut child = Command::new(exe)
        .env(CHILD_ENV, "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to start the child sample");
    let stdout = read_pipe(child.stdout.take());
    let stderr = read_pipe(child.stderr.take());
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().expect("failed to poll the child sample") {
            break Some(status);
        }
        if started.elapsed() > CHILD_TIMEOUT {
            child.kill().expect("failed to kill the child sample");
            child.wait().expect("failed to reap the child sample");
            break None;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let stdout = stdout.join().expect("the stdout reader panicked");
    let stderr = stderr.join().expect("the stderr reader panicked");
    let Some(status) = status else {
        panic!(
            "child sample did not exit within {CHILD_TIMEOUT:?}; stdout {stdout:?}; stderr {stderr:?}"
        );
    };
    assert!(
        status.success(),
        "child sample failed: {status}; stdout {stdout:?}; stderr {stderr:?}"
    );
    let line = stdout
        .lines()
        .find_map(|line| line.strip_prefix(SAMPLE_TAG))
        .unwrap_or_else(|| panic!("child printed no sample; stdout {stdout:?}; stderr {stderr:?}"));
    let numbers: Vec<u128> = line
        .split_whitespace()
        .map(|field| {
            field.parse().unwrap_or_else(|error| {
                panic!(
                    "sample field {field:?} is not a number ({error}); line {line:?}; stderr {stderr:?}"
                )
            })
        })
        .collect();
    assert_eq!(
        numbers.len(),
        4,
        "fields in the sample line {line:?}; stderr {stderr:?}"
    );
    Sample {
        shutdown_ns: numbers[0],
        completed_after_cancel: numbers[1],
        errored: numbers[2],
        remaining: numbers[3],
    }
}

/// Returns min, median and max of `values`; the median of an even count is the integer
/// mean of the two middle values.
fn min_median_max(mut values: Vec<u128>) -> (u128, u128, u128) {
    values.sort_unstable();
    let n = values.len();
    let median = if n % 2 == 1 {
        values[n / 2]
    } else {
        (values[n / 2 - 1] + values[n / 2]) / 2
    };
    (values[0], median, values[n - 1])
}

type Field = fn(&Sample) -> u128;

fn print_rows(strategy: &str, samples: &[Sample]) {
    let rows: [(&str, Field); 4] = [
        ("shutdown_ns", |s| s.shutdown_ns),
        ("completed_after_cancel", |s| s.completed_after_cancel),
        ("errored", |s| s.errored),
        ("remaining", |s| s.remaining),
    ];
    for (name, field) in rows {
        let (min, median, max) = min_median_max(samples.iter().map(field).collect());
        println!(
            "{strategy:<8} {name:<24} {:>4} {min:>14} {median:>14} {max:>14}",
            samples.len()
        );
    }
}

/// Returns whether the arguments put criterion in test mode: no `--bench`, or `--test`.
fn test_mode() -> bool {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let bench = args.iter().any(|arg| arg == "--bench");
    let test = args.iter().any(|arg| arg == "--test");
    !bench || test
}

fn main() {
    if std::env::var_os(CHILD_ENV).is_some() {
        child();
        return;
    }

    let runtime = runtime();
    runtime.block_on(define_slow());

    let drain_samples = Arc::new(Mutex::new(Vec::new()));
    let mut criterion = Criterion::default().configure_from_args();
    let mut group = criterion.benchmark_group("shutdown_ns");
    group
        .sampling_mode(SamplingMode::Flat)
        .sample_size(10)
        .warm_up_time(Duration::from_secs(1))
        .measurement_time(Duration::from_secs(3));
    group.bench_function("drain", |b| {
        b.to_async(&runtime).iter_custom(|iters| {
            let drain_samples = Arc::clone(&drain_samples);
            async move {
                let mut total = Duration::ZERO;
                for _ in 0..iters {
                    let sample = sample(Strategy::Drain).await;
                    total += Duration::from_nanos(
                        u64::try_from(sample.shutdown_ns).expect("shutdown_ns exceeds u64"),
                    );
                    drain_samples
                        .lock()
                        .expect("invariant: no holder of this lock panics")
                        .push(sample);
                }
                total
            }
        });
    });
    group.finish();
    criterion.final_summary();
    runtime
        .block_on(shared::close_and_wait())
        .expect("the Julia runtime thread panicked");

    if test_mode() {
        println!("shutdown: test mode, no child process started and no table printed");
        return;
    }
    let drain_samples = drain_samples
        .lock()
        .expect("invariant: no holder of this lock panics")
        .clone();
    if drain_samples.is_empty() {
        println!("shutdown: no drain sample ran, no child process started and no table printed");
        return;
    }
    let close_samples: Vec<Sample> = (0..CLOSE_CHILDREN).map(|_| close_sample()).collect();

    println!(
        "{:<8} {:<24} {:>4} {:>14} {:>14} {:>14}",
        "strategy", "quantity", "runs", "min", "median", "max"
    );
    print_rows("drain", &drain_samples);
    print_rows("close", &close_samples);
}
