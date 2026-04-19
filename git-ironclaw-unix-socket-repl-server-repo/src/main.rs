use std::io::{self, Write};                                                                                                                                                      15:01:24 [252/9142]
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::time::timeout;

const SOCKET_PATH: &str = "/tmp/ironclaw.sock";
const MODEL_TIMEOUT: Duration = Duration::from_secs(599); // Configurable timeout

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("IronClaw Unix Socket Client v2 (Optimized)");
    println!("Connecting to: {}", SOCKET_PATH);
     
    let stream = match UnixStream::connect(SOCKET_PATH).await {
            Ok(s) => {
            println!("Connected! Type commands (type 'quit' to exit):");
            s
        }
        Err(e) => {
            eprintln!("❌ Failed to connect: {}", e);
            eprintln!("Make sure IronClaw daemon is running");
            return Ok(());
        }
    };
     
    let (mut read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);
    let mut stdin = BufReader::new(tokio::io::stdin());
     
    loop {
        print!("> ");
        io::stdout().flush()?;
         
        let mut input = String::new();
        if stdin.read_line(&mut input).await? == 0 {
            break;
        }
         
        let trimmed = input.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed == "quit" || trimmed == "exit" {
            break;
        }
         
        // Send request
        write_half.write_all(input.as_bytes()).await?;
         
        // Show progress indicator
        print!("⏳  Thinking...");
        io::stdout().flush()?;
        // Wait for response with timeout
        let mut response = String::new();
        match timeout(MODEL_TIMEOUT, reader.read_line(&mut response)).await {
            Ok(Ok(0)) => {
                print!("\r                \r");
                eprintln!("⚠️  Connection closed by server");
                break;
            }
            Ok(Ok(_)) => {
                // Clear progress indicator and show response
                print!("\r                \r");
                println!("{}", response.trim());
            }
            Ok(Err(e)) => {
                print!("\r                \r");
                eprintln!("⚠️  Read error: {}", e);
            }
            Err(_) => {
                print!("\r                \r");
                eprintln!("⏱️  Timeout: Model took longer than {}s to respond", MODEL_TIMEOUT.as_secs());
                eprintln!("    (You can adjust MODEL_TIMEOUT in the code)");
            }
        }
     
    println!("Goodbye! 👋");
    Ok(())
}
}
