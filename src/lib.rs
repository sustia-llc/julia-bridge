//! Scaffold for embedding Julia in Rust with jlrs' multithreaded runtime (`multi-rt`).
//!
//! The shape of the multithreaded runtime:
//!
//! 1. [`run`] initializes Julia and hands your closure an [`MtHandle`]. Julia stays alive
//!    until the closure returns.
//! 2. `MtHandle::spawn` starts scoped threads that are allowed to call into Julia.
//! 3. Inside a thread, `MtHandle::with` marks the section that actually talks to Julia.
//!    Keep unrelated blocking work *outside* `with`, or you stall Julia's garbage collector.
//!
//! Julia can be initialized only once per process, so call [`run`] once (typically from
//! `main`) and do all Julia work inside it.

#[cfg(feature = "exec")]
pub mod exec;
#[cfg(feature = "lifecycle")]
pub mod lifecycle;
#[cfg(feature = "tokio")]
pub mod shared;

use jlrs::prelude::*;

pub use jlrs::{error::JlrsResult, runtime::handle::mt_handle::MtHandle};

/// Start Julia with `n_threads` Julia threads (0 = one per CPU core) and run `f`.
///
/// Julia shuts down when `f` returns. Calling this a second time in the same process
/// fails, because Julia can't be re-initialized.
pub fn run<'env, T, F>(n_threads: usize, f: F) -> JlrsResult<T>
where
    T: 'static + Send,
    F: 'env + for<'scope> FnOnce(MtHandle<'scope, 'env>) -> T + Send,
{
    Builder::new().n_threads(n_threads).start_mt(f)
}

/// Evaluate a snippet of Julia code and return its result as an `f64`.
///
/// Errors if the code throws or the result isn't a `Float64`.
pub fn eval_f64(handle: &mut MtHandle<'_, '_>, code: &str) -> JlrsResult<f64> {
    handle.with(|active| {
        // One rooted value (the result), so the local scope needs exactly 1 slot.
        active.local_scope::<_, 1>(|mut frame| {
            // Safety: evaluating arbitrary code is only as safe as the code itself.
            let value = unsafe { Value::eval_string(&mut frame, code) }?;
            value.unbox::<f64>()
        })
    })
}

/// Call a two-argument function from Julia's `Base` module, e.g. `"hypot"` or `"+"`.
pub fn call_base2(handle: &mut MtHandle<'_, '_>, func: &str, a: f64, b: f64) -> JlrsResult<f64> {
    handle.with(|active| {
        // Slots: a, b, the function, the result.
        active.local_scope::<_, 4>(|mut frame| {
            let a = Value::new(&mut frame, a);
            let b = Value::new(&mut frame, b);
            let func = Module::base(&frame).global(&mut frame, func)?;
            // Safety: only as safe as the Julia function being called.
            let out = unsafe { func.call(&mut frame, [a, b]) }?;
            out.unbox::<f64>()
        })
    })
}

/// Apply a one-argument `Base` function to every input, splitting the work over
/// `workers` OS threads that each call into Julia. Output order matches input order.
pub fn par_map_f64(
    handle: &MtHandle<'_, '_>,
    func: &str,
    inputs: &[f64],
    workers: usize,
) -> JlrsResult<Vec<f64>> {
    let chunk_len = inputs.len().div_ceil(workers.max(1)).max(1);

    let threads: Vec<_> = inputs
        .chunks(chunk_len)
        .map(|chunk| {
            // Scoped threads need owned data.
            let chunk = chunk.to_vec();
            let func = func.to_owned();
            handle.spawn(move |mut handle| -> JlrsResult<Vec<f64>> {
                handle.with(|active| {
                    chunk
                        .iter()
                        .map(|&x| {
                            // Slots: x, the function, the result. A fresh scope per item
                            // lets the GC reclaim values as we go.
                            active.local_scope::<_, 3>(|mut frame| {
                                let x = Value::new(&mut frame, x);
                                let func = Module::base(&frame).global(&mut frame, &func)?;
                                let out = unsafe { func.call(&mut frame, [x]) }?;
                                out.unbox::<f64>()
                            })
                        })
                        .collect()
                })
            })
        })
        .collect();

    let mut out = Vec::with_capacity(inputs.len());
    for thread in threads {
        out.extend(thread.join().expect("Julia worker thread panicked")?);
    }
    Ok(out)
}
