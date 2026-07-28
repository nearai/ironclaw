use std::{
    sync::{Arc, mpsc},
    thread,
    time::{Duration, Instant},
};

use crate::{
    Args, Sample, Scenario,
    progress::{ProgressCounters, spawn_progress_reporter, stop_progress_reporter},
    trace::{spawn_trace_reporter, stop_trace_reporter},
};

pub(crate) fn run(args: &Args) -> Result<Vec<Sample>, String> {
    let operation_target = args.operation_target();
    let progress = Arc::new(ProgressCounters::new(args.trace_jsonl.is_some()));
    let progress_reporter = spawn_progress_reporter(
        crate::log_prefix(args),
        args.backend.as_str(),
        args.scenario.as_str(),
        args.progress_interval_seconds,
        operation_target.progress_total(),
        Arc::clone(&progress),
    );
    let trace_reporter = spawn_trace_reporter(args, "process://local", Arc::clone(&progress));
    let samples = run_threads_inner(args, &progress);
    stop_trace_reporter(trace_reporter);
    stop_progress_reporter(progress_reporter);
    samples
}

fn run_threads_inner(args: &Args, progress: &Arc<ProgressCounters>) -> Result<Vec<Sample>, String> {
    let (sender, receiver) = mpsc::channel();
    let mut handles = Vec::with_capacity(args.concurrency);

    for worker_index in 0..args.concurrency {
        let sender = sender.clone();
        let progress = Arc::clone(progress);
        let args = args.clone();
        let handle = match thread::Builder::new()
            .name(format!("ironclaw-stress-pressure-{worker_index}"))
            .spawn(move || -> Result<(), String> {
                let mut samples = Vec::with_capacity(args.initial_worker_sample_capacity());
                let started = Instant::now();
                let mut operation_index = 0;
                while should_run_operation(args.operation_target(), started, operation_index) {
                    let sample = run_one_operation(&args, worker_index, operation_index);
                    progress.record(sample.error.is_some(), sample.latency);
                    samples.push(sample);
                    operation_index += 1;
                }
                sender
                    .send(samples)
                    .map_err(|_| "sample receiver dropped".to_string())
            }) {
            Ok(handle) => handle,
            Err(error) => {
                join_workers(handles)?;
                return Err(format!("spawn pressure worker {worker_index}: {error}"));
            }
        };
        handles.push((worker_index, handle));
    }
    drop(sender);

    let mut samples = Vec::with_capacity(args.concurrency * args.operations);
    for worker_samples in receiver {
        samples.extend(worker_samples);
    }
    join_workers(handles)?;
    if let Some(expected) = args.operation_target().progress_total()
        && samples.len() != expected
    {
        return Err(format!(
            "collected {} samples but expected {expected}",
            samples.len()
        ));
    }
    Ok(samples)
}

fn should_run_operation(
    operation_target: crate::OperationTarget,
    started: Instant,
    operation_index: usize,
) -> bool {
    match operation_target {
        crate::OperationTarget::Fixed {
            operations_per_worker,
            ..
        } => operation_index < operations_per_worker,
        crate::OperationTarget::Duration { duration } => started.elapsed() < duration,
    }
}

fn run_one_operation(args: &Args, worker_index: usize, operation_index: usize) -> Sample {
    let started = Instant::now();
    match args.scenario {
        Scenario::CpuBurn => cpu_burn(args.cpu_work_units, worker_index, operation_index),
        Scenario::MemoryChurn => memory_churn(args.memory_bytes, args.memory_hold_ms),
        Scenario::ReserveRelease
        | Scenario::ReserveReconcile
        | Scenario::ChatTurn
        | Scenario::TurnLifecycleChurn
        | Scenario::ThreadList
        | Scenario::MixedUserSession
        | Scenario::ContextGrowth
        | Scenario::ToolSession
        | Scenario::ApiUserCapacity
        | Scenario::SecretConsume => {
            unreachable!("process pressure only handles process-local scenarios")
        }
    }
    Sample {
        latency: started.elapsed(),
        error: None,
        failure: None,
        stages: None,
    }
}

fn cpu_burn(work_units: u64, worker_index: usize, operation_index: usize) {
    let mut state =
        0x9E37_79B9_7F4A_7C15u64 ^ (worker_index as u64).wrapping_shl(32) ^ operation_index as u64;
    for _ in 0..work_units {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state = state.wrapping_mul(0xD6E8_FD9D_5A42_9A1D);
    }
    std::hint::black_box(state);
}

fn memory_churn(memory_bytes: usize, hold_ms: u64) {
    let mut buffer = vec![0u8; memory_bytes];
    for (index, byte) in buffer.iter_mut().enumerate().step_by(4096) {
        *byte = index.wrapping_mul(31) as u8;
    }
    let checksum = buffer
        .iter()
        .step_by(4096)
        .fold(0u8, |checksum, byte| checksum.wrapping_add(*byte));
    std::hint::black_box(checksum);
    if hold_ms > 0 {
        thread::sleep(Duration::from_millis(hold_ms));
    }
    std::hint::black_box(buffer);
}

fn join_workers(
    handles: Vec<(usize, thread::JoinHandle<Result<(), String>>)>,
) -> Result<(), String> {
    for (worker_index, handle) in handles {
        match handle.join() {
            Ok(Ok(())) => {}
            Ok(Err(error)) => return Err(error),
            Err(error) => {
                return Err(format!(
                    "pressure worker {worker_index} panicked: {}",
                    panic_payload_to_string(&error)
                ));
            }
        }
    }
    Ok(())
}

fn panic_payload_to_string(payload: &Box<dyn std::any::Any + Send + 'static>) -> String {
    if let Some(message) = payload.downcast_ref::<&'static str>() {
        (*message).to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "unknown panic payload".to_string()
    }
}
