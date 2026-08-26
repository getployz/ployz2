#![cfg(unix)]

use std::io::{BufRead, BufReader};
use std::os::unix::process::ExitStatusExt;
use std::process::{Command, ExitStatus, Stdio};

const SIGPIPE: i32 = 13;

fn ployz() -> Command {
    Command::new(env!("CARGO_BIN_EXE_ployz"))
}

fn died_on_closed_pipe(status: &ExitStatus) -> bool {
    status.signal() == Some(SIGPIPE) || status.code() == Some(128 + SIGPIPE)
}

#[test]
fn completion_exits_quietly_when_the_writer_closes_after_the_first_line() {
    let mut child = ployz()
        .args(["completion", "bash"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    assert!(
        !line.is_empty(),
        "completion produced no output before the pipe closed"
    );
    let output = child.wait_with_output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("panicked"),
        "closed pipe panicked: {stderr}"
    );
    assert!(
        died_on_closed_pipe(&output.status),
        "status={:?} stderr={stderr}",
        output.status
    );
}
