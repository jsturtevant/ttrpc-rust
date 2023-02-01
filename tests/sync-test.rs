//use std::io::{stderr, stdout, Write};
use std::{process::Command, time::Duration};

#[test]
fn run_sync_example() -> Result<(), Box<dyn std::error::Error>> {
    // start the server and give it a moment to start.
    let server = run_example("server").spawn();
    std::thread::sleep(Duration::from_secs(2));

    let client = run_example("client").spawn();
    let mut client_succeeded = false;
    match client.unwrap().wait() {
        Ok(status) => {
            client_succeeded = status.success();
        }
        Err(e) => {
            println!("Error: {e}");
        }
    }

    // be sure to clean up the server, the client should have run to completion
    server.unwrap().kill()?;
    assert!(client_succeeded);
    Ok(())
}

fn run_example(example: &str) -> Command {
    let mut cmd = Command::new("cargo");
    cmd.arg("run")
        .arg("--example")
        .arg(example)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .current_dir("example");
    cmd
}
