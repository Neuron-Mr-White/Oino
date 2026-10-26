//! The `oino` binary: see the `oino` library for the modes.

#![forbid(unsafe_code)]

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = oino::run(args).await;
    // Exit without waiting for a blocked stdin reader thread (tokio's stdin is a blocking
    // read on a worker thread); stdout was flushed by the mode.
    std::process::exit(code);
}
