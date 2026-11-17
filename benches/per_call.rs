//! Run with: cargo bench --features lifecycle --bench per_call
//!
//! `per_call_ns`: the time of one round trip of `call_base1("sqrt", 2.0)` on a
//! multi-thread tokio runtime, through `shared` and through a `Lifecycle` of each
//! strategy that is never cancelled. After the last case it closes the shared handle
//! and waits for the Julia runtime thread.

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use julia_bridge::{
    lifecycle::{Lifecycle, Strategy},
    shared,
};

fn per_call(c: &mut Criterion) {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_time()
        .build()
        .expect("failed to build the tokio runtime");
    let started = runtime
        .block_on(shared::call_base1("sqrt", 2.0))
        .expect("failed to start Julia");
    assert_eq!(started, 2.0_f64.sqrt());

    let drain = Lifecycle::new(Strategy::Drain);
    let close = Lifecycle::new(Strategy::Close);

    let mut group = c.benchmark_group("per_call_ns");
    group.bench_function("baseline", |b| {
        b.to_async(&runtime).iter(|| async {
            black_box(shared::call_base1("sqrt", black_box(2.0)).await).expect("baseline call")
        });
    });
    group.bench_function("drain", |b| {
        b.to_async(&runtime).iter(|| async {
            black_box(drain.call_base1("sqrt", black_box(2.0)).await).expect("drain call")
        });
    });
    group.bench_function("close", |b| {
        b.to_async(&runtime).iter(|| async {
            black_box(close.call_base1("sqrt", black_box(2.0)).await).expect("close call")
        });
    });
    group.finish();

    runtime
        .block_on(shared::close_and_wait())
        .expect("the Julia runtime thread panicked");
}

criterion_group!(benches, per_call);
criterion_main!(benches);
