//! Emits the rpath of Julia's `lib` directory for the tests, examples and benches of this
//! package, from the environment variable `JLRS_JULIA_DIR`.

use std::env::{self, VarError};

fn main() {
    println!("cargo::rerun-if-env-changed=JLRS_JULIA_DIR");
    match env::var("JLRS_JULIA_DIR") {
        Ok(julia_dir) => println!("cargo::rustc-link-arg=-Wl,-rpath,{julia_dir}/lib"),
        Err(VarError::NotPresent) => {}
        Err(VarError::NotUnicode(_)) => {
            println!("cargo::warning=JLRS_JULIA_DIR is not valid Unicode, so no rpath is emitted");
        }
    }
}
