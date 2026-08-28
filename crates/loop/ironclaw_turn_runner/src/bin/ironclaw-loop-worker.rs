fn main() {
    let result = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("loop worker runtime creation failed: {error}"))
        .and_then(|runtime| {
            runtime
                .block_on(ironclaw_turn_runner::sandboxed_planned_driver::run_loop_worker_stdio())
        });
    if let Err(error) = result {
        eprintln!("ironclaw-loop-worker: {error}");
        std::process::exit(1);
    }
}
