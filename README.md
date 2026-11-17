# julia-bridge

A scaffold for embedding Julia in a Rust library with [jlrs](https://github.com/Taaitaaiger/jlrs)
and its multithreaded runtime (`multi-rt`). Rename the crate and replace the helper
functions in `src/lib.rs` with your own.

## Requirements

- Rust 1.97 or newer
- Julia 1.10 to 1.13 from the official tarball, extracted somewhere like `~/julia-1.13.4`
- Network access on first run: jlrs installs the `JlrsCore` Julia package automatically

Using juliaup instead of the tarball? jlrs' default detection doesn't work with it; see
the `jlrs-launcher` notes in the jlrs README.

## Layout

```
Cargo.toml              jlrs with the multi-rt feature
.cargo/config.toml      linker flag -rdynamic
build.rs                rpath of Julia's lib directory, from JLRS_JULIA_DIR
tests/rpath.rs          cargo test --test rpath
src/lib.rs              run(), eval_f64(), call_base2(), par_map_f64()
src/shared.rs           process-wide runtime for tokio tasks (feature "tokio")
src/exec.rs             rayon thread pool awaited from async code (feature "exec")
examples/demo.rs        cargo run --example demo
examples/tokio_demo.rs  cargo run --features tokio --example tokio_demo
tests/runtime.rs        cargo test
examples/pipeline.rs    cargo run --features tokio,exec --example pipeline
tests/shared.rs         cargo test --features tokio
tests/exec.rs           cargo test --features exec
src/lifecycle.rs        cancellation token, task tracker and shutdown (feature "lifecycle")
tests/lifecycle_*.rs    cargo test --features lifecycle
benches/per_call.rs     cargo bench --features lifecycle --bench per_call
benches/shutdown.rs     cargo bench --features lifecycle --bench shutdown
contrib/julia.fish      fish environment
contrib/julia-env.sh    bash-syntax environment file
```

## What jlrs needs from the environment

| When | What | How it's found |
|---|---|---|
| Build | `julia.h` and `libjulia.so` | `which julia`, or `JLRS_JULIA_DIR` if set |
| Run | `libjulia.so` | `LD_LIBRARY_PATH`, or an rpath baked into the binary |

In bash this is the familiar pair:

```bash
export PATH="$HOME/julia-1.13.4/bin:$PATH"
export LD_LIBRARY_PATH="$HOME/julia-1.13.4/lib"
```

The sections below point at `~/julia`. Make that path a symlink, so that a Julia upgrade
is one `ln -sfn` and a rebuild:

```bash
ln -sfn ~/julia-1.13.4 ~/julia
```

### Environment file

`contrib/julia-env.sh` sets `PATH`, `JLRS_JULIA_DIR` and `LD_LIBRARY_PATH` for `~/julia`:

```bash
cp contrib/julia-env.sh ~/.config/julia-env.sh
```

In `~/.bashrc`:

```bash
source ~/.config/julia-env.sh
```

### Rpath: no `LD_LIBRARY_PATH`

`build.rs` reads `JLRS_JULIA_DIR` at build time. If the variable is set, the script adds
an rpath of `$JLRS_JULIA_DIR/lib` to the tests, examples and benches of this package.
Those binaries then run with no `LD_LIBRARY_PATH`:

```bash
JLRS_JULIA_DIR=~/julia cargo test
```

Notes:

- The value goes into the binary as given, so use an absolute path.
- If the variable is not set, the script adds no rpath. If `julia` is also not on
  `PATH`, the build stops with `JLRS_JULIA_DIR is not set and no installed version of
  Julia can be found`.
- Cargo applies the link argument to the benches, binaries, examples and tests of this
  package.
- To see the result, run `readelf -d target/debug/examples/demo | grep RUNPATH`.

### Which one?

| | Global environment change | Binary runs outside your shell |
|---|---|---|
| Environment file | yes | no |
| Rpath | no | yes |

A global `LD_LIBRARY_PATH` applies to each program that you start. `~/julia/lib` holds
only the libraries of Julia. The rpath setup does not need that variable.

## About `RUSTFLAGS` and `-rdynamic`

jlrs recommends linking embedding applications with `-rdynamic` for fast code. This
scaffold sets it in `.cargo/config.toml`.

If the `RUSTFLAGS` environment variable is set, cargo ignores `build.rustflags` from
the config file completely. So either leave `RUSTFLAGS` unset, or put all flags in it:

```bash
export RUSTFLAGS="-Clink-arg=-rdynamic"
```

## Check the setup

```bash
type -a julia                    # ~/julia/bin/julia
echo "$LD_LIBRARY_PATH"
echo "$JLRS_JULIA_DIR"
```

## Build and run

```bash
cargo test
cargo run --example demo
```

Expected demo output:

```
eval      sqrt(2)     = 1.4142135623730951
call      hypot(3, 4) = 5
par_map   sqrt.(1:8)  = [1.0, 1.4142135623730951, ...]
```

The first run prints Julia package installation output while `JlrsCore` is installed
and precompiled. Later runs start quietly.

## Using the library

```rust
use julia_bridge::{JlrsResult, eval_f64, par_map_f64, run};

fn main() {
    run(4, |mut julia| -> JlrsResult<()> {
        let x = eval_f64(&mut julia, "sqrt(2.0)")?;
        let ys = par_map_f64(&julia, "sqrt", &[1.0, 4.0, 9.0], 3)?;
        println!("{x} {ys:?}");
        Ok(())
    })
    .expect("failed to start Julia")
    .expect("Julia call failed");
}
```

How the multithreaded runtime is structured:

- `run` starts Julia and passes your closure an `MtHandle`. Julia shuts down when the
  closure returns.
- `MtHandle::spawn` starts a scoped thread that may call into Julia.
- `MtHandle::with` marks the section of a thread that talks to Julia. Inside it,
  `local_scope::<_, N>` roots up to `N` Julia values so the garbage collector leaves
  them alone.

## Tokio: one shared runtime

The scoped API above has two awkward rules: Julia starts once per process, and you
mustn't block inside `with`. The `tokio` feature adds `julia_bridge::shared`, which
handles both by switching to jlrs' async runtime:

- Julia starts lazily, once, on its own background thread, and the handle lives in a
  static. There is no closure to stay inside and no second start to get wrong.
- Any tokio task on any thread sends work to that runtime and awaits the result.
- `call_async` runs the Julia function on one of Julia's threads, so a slow call
  doesn't hold up other tasks.
- Tests can be ordinary separate `#[tokio::test]` functions (see `tests/shared.rs`).

```rust
use julia_bridge::shared::{call_base1, close_and_wait, eval_f64, julia};

#[tokio::main]
async fn main() {
    // Optional: start Julia up front instead of on the first request.
    tokio::task::spawn_blocking(julia).await.unwrap();

    let x = eval_f64("sqrt(2.0)").await.unwrap();
    let tasks: Vec<_> = (1..=8)
        .map(|i| tokio::spawn(call_base1("sqrt", f64::from(i))))
        .collect();
    // ... await the tasks

    // Close the handle and wait for the Julia runtime thread before `main` returns.
    close_and_wait().await.unwrap();
}
```

```bash
cargo test --features tokio
cargo run --features tokio --example tokio_demo
```

What to know:

- **Pick one style per process.** `run` and `shared` each try to start Julia, and only
  the first one succeeds.
- **The first `julia()` call blocks** for the few seconds Julia takes to start. Warm it
  up with `spawn_blocking` as above, or accept one slow first request.
- **Julia data stays on the runtime thread.** Tasks send plain Rust values in
  (`String`, `f64`, `Vec<f64>`) and get plain Rust values back.
- **`MAX_CONCURRENT_TASKS`** in `src/shared.rs` caps how many Julia tasks are in flight
  at once. Further tasks queue.
- Before `main` returns, await `shared::close_and_wait()`. That function closes the
  shared handle and waits until the Julia runtime thread ends, which is after each
  accepted call returned. It does not block the tokio thread. If the process exits while Julia
  compiles code, it can print `signal 11 (1): Segmentation fault` or die from the
  signal.

## Rayon: a pool for the Rust-side work

The `exec` feature adds `julia_bridge::exec::Executor`, a caller-owned rayon thread
pool whose jobs are awaited from async code (via `tokio-rayon`). Use it for CPU-heavy
pure-Rust work so it blocks neither tokio's workers nor the Julia runtime thread.

```rust
use julia_bridge::{
    exec::{Executor, rayon::prelude::*},
    shared::call_base1,
};

let exec = Executor::new()?;                       // or Executor::with_num_threads(8)
let sum: u64 = exec
    .run(|| (1..=1_000_000u64).into_par_iter().map(|x| x * x).sum())
    .await;
let root = call_base1("sqrt", sum as f64).await?;  // then hand the result to Julia
```

```bash
cargo test --features exec
cargo run --features tokio,exec --example pipeline
cargo test --all-features                          # everything
```

What to know:

- **Pool threads are not Julia threads.** A job must not call into Julia directly.
  Compute in the pool, then pass plain Rust values to the `shared` functions.
- **Parallel iterators inside `run` use this pool**, not rayon's global one.
- **A panic in a job resumes on the awaiting task.**
- **Watch total thread count.** Rayon and Julia each default to one thread per core.
  If both are busy at once, size them explicitly (`with_num_threads`, and
  `n_threads` on the jlrs `Builder` in `src/shared.rs`).
- The `exec` and `tokio` features are independent; enable either or both.

## Lifecycle: cancel, drain, shut down

The `lifecycle` feature adds `julia_bridge::lifecycle::Lifecycle`. It enables the
`tokio` feature. A `Lifecycle` owns a `CancellationToken` and a `TaskTracker` from
`tokio-util`, and it sends its calls through the `shared` module. Each call runs in a
tokio task that the tracker counts until the call returns. If you drop the future of a
call after its first poll, the call still runs in Julia and stays counted. Calls need a
tokio runtime, and `shutdown` needs a tokio runtime with the time driver.

```rust
use std::time::Duration;
use julia_bridge::lifecycle::{Lifecycle, LifecycleError, Strategy};

let lifecycle = Lifecycle::new(Strategy::Drain);
let root = lifecycle.call_base1("sqrt", 2.0).await?;

let report = lifecycle.shutdown(Duration::from_secs(30)).await?;
// report.drained, report.remaining, report.elapsed
// report.close holds the result of the close step under Strategy::Close.

let late = lifecycle.call_base1("sqrt", 2.0).await;
assert!(matches!(late, Err(LifecycleError::Cancelled)));
```

`shutdown` does these steps in order. Steps 1 and 2 run in the first poll of `shutdown`.
Steps 3 to 5 run in a tokio task, so they also run if you drop the `shutdown` future
after that poll.

1. It cancels a token. Under `Strategy::Drain`, that is the token of this `Lifecycle`.
   Under `Strategy::Close`, that is a process-wide root token, the parent of the token
   of each `Lifecycle`.
2. It closes the tracker.
3. It waits for the tracked calls, for `drain_timeout` at most.
4. It reads the number of tracked calls that remain.
5. Under `Strategy::Close`, it awaits `shared::close_and_wait()` in a tokio task.
   That function sets a process-wide closed flag and, if Julia started, calls
   `close(false)` on the shared Julia handle. Then it waits until the Julia runtime
   thread ends, which is after each accepted call returned. After it,
   `shared::eval_f64` and `shared::call_base1` return an error and do not start Julia.

Under `Strategy::Close`, `shutdown` can take longer than `drain_timeout`, because step 5
waits for the calls that still run. If step 5 fails, `shutdown` still returns
`Ok(report)`, and `report.close` holds the error. Under `Strategy::Drain`, do the
`shared::close_and_wait()` step yourself before `main` returns.

The two strategies differ in steps 1 and 5:

| Strategy | Other `Lifecycle` values | Shared Julia handle | Later `shared` calls |
|---|---|---|---|
| `Drain` | work | open | work |
| `Close` | return `Cancelled` | closed | return an error |

`cancellation_token()` returns the token of the `Lifecycle`. If you cancel it from your
own signal handler, the next call through that `Lifecycle` returns
`LifecycleError::Cancelled`.

Put each test that shuts a `Close` lifecycle down in its own file under `tests/`.

### Benchmarks

```bash
cargo bench --features lifecycle --bench per_call
cargo bench --features lifecycle --bench shutdown
```

`per_call` measures one `call_base1("sqrt", 2.0)` round trip through `shared` directly
(`baseline`) and through a `Lifecycle` of each strategy. `shutdown` starts 64 calls of a
Julia function that sleeps 0.02 s, waits for the first result, and then calls
`shutdown`. The bench starts one child process for each of its 20 `Close` runs. The
`Drain` row counts each run that criterion made, warm-up included.

Results of one run on the development machine:

| Case | Per call, criterion estimate |
|---|---|
| `baseline` | 6.98 ms |
| `drain` | 7.01 ms |
| `close` | 6.82 ms |

| Strategy | Runs | `shutdown` time, min / median / max | Calls not returned |
|---|---|---|---|
| `Drain` | 45 | 74.2 / 77.3 / 80.4 ms | 0 |
| `Close` | 20 | 76.1 / 79.8 / 83.0 ms | 0 |

The `Close` time includes the wait for the Julia runtime thread.

## Things that will bite

- **Julia starts once per process.** A second `run` call fails. Call it once, near
  `main`, and do all Julia work inside, or use the `shared` module above.
- **Tests share a process.** Every test in one file runs in the same process, so all
  runtime tests live in the single test function in `tests/runtime.rs`. Add cases
  there, or add a new file under `tests/` (each file is its own process).
- **Don't block inside `with`.** Julia's garbage collector waits for every thread that
  is inside `with`. Do file I/O, network calls, and long pure-Rust work outside it.
- **Slot counts are exact.** `local_scope::<_, 3>` panics if you root a fourth value.
  Count the `&mut frame` uses.
- **After a Julia upgrade, rebuild.** Run `cargo clean`, since the bindings are
  generated for the Julia version found at build time.
- **`libjulia.so.1.13: cannot open shared object file`** means the runtime half of the
  setup is missing: fix `LD_LIBRARY_PATH` (environment file) or build with `JLRS_JULIA_DIR` set (rpath).
