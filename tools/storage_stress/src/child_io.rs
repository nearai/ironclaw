use std::{
    any::Any,
    io::{BufRead, BufReader},
    process::ChildStderr,
    thread::{self, JoinHandle},
};

pub(crate) fn spawn_child_stderr_reader(
    child_index: usize,
    stderr: ChildStderr,
) -> Option<JoinHandle<()>> {
    thread::Builder::new()
        .name(format!("storage-stress-child-{child_index}-stderr"))
        .spawn(move || {
            let reader = BufReader::new(stderr);
            for line in reader.lines() {
                match line {
                    Ok(line) => eprintln!("{line}"),
                    Err(error) => {
                        eprintln!("failed to read child {child_index} stderr: {error}");
                        break;
                    }
                }
            }
        })
        .ok()
}

pub(crate) fn join_child_stderr_reader(child_index: usize, stderr_reader: Option<JoinHandle<()>>) {
    if let Some(stderr_reader) = stderr_reader
        && let Err(payload) = stderr_reader.join()
    {
        eprintln!(
            "child {child_index} stderr reader panicked: {}",
            panic_payload_to_string(&payload)
        );
    }
}

fn panic_payload_to_string(payload: &Box<dyn Any + Send + 'static>) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        return (*message).to_string();
    }
    if let Some(message) = payload.downcast_ref::<String>() {
        return message.clone();
    }
    "non-string panic payload".to_string()
}
