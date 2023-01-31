use assert_cmd::prelude::*;
use std::io::{stderr, stdout, Write};
use std::{process::Command, time::Duration};

#[test]
fn run_sync_example() -> Result<(), Box<dyn std::error::Error>> {
    build_example("server");
    build_example("client");

    // start the server and give it a moment to start.
    let mut cmd = Command::cargo_bin("examples/server")?;
    let mut server = cmd.spawn()?;
    std::thread::sleep(Duration::from_secs(2));

    let client = Command::cargo_bin("examples/client").unwrap().spawn();
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
    server.kill()?;
    assert!(client_succeeded);
    Ok(())
}

fn build_example(example: &str) {
    let mut cmd = Command::new("cargo");
    let output = cmd
        .arg("build")
        .arg("--example")
        .arg(example)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .current_dir("example")
        .output()
        .expect("failed to execute process");
    let code = output.status.code().expect("should have status code");
    if code != 0 {
        stdout().write_all(&output.stdout).unwrap();
        stderr().write_all(&output.stderr).unwrap();
        panic!("failed to command cargo build for {}", example);
    }
}
