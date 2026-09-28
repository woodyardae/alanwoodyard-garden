use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};

use serde_json::Value;

const HELLO: &str = "\
---\n\
blog: \"acme\"\n\
title: \"Hello\"\n\
---\n\
\n\
See [[Harbor Log|the log]].\n\
";

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "bosun-garden-cli-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn write(&self, rel: &str, body: &str) {
        let path = self.0.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, body).unwrap();
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn bosun() -> Command {
    // Integration tests of a binary-only package do not receive CARGO_BIN_EXE_*.
    // The test harness lives in target/debug/deps, next to the built binary.
    let exe = std::env::current_exe().expect("test executable");
    let debug_dir = exe
        .parent()
        .and_then(|dir| dir.parent())
        .expect("target debug directory");
    let bin = debug_dir.join(format!("bosun-garden{}", std::env::consts::EXE_SUFFIX));
    assert!(bin.is_file(), "missing CLI binary at {}", bin.display());
    Command::new(bin)
}

fn json_lines(output: &Output) -> Vec<Value> {
    let stdout = String::from_utf8(output.stdout.clone()).expect("stdout is utf-8");
    stdout
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_str(line).unwrap_or_else(|err| panic!("{err}: {line}")))
        .collect()
}

#[test]
fn build_streams_jsonl_for_one_tenant() {
    let scratch = Scratch::new("build");
    scratch.write("hello.md", HELLO);
    scratch.write(
        "other.md",
        "---\nblog: \"other\"\ntitle: Other\n---\n\nOther tenant.\n",
    );
    let dist = scratch.0.join("dist");
    let output = bosun()
        .args([
            "build",
            "--vault",
            scratch.0.to_str().unwrap(),
            "--output",
            dist.to_str().unwrap(),
            "--tenant",
            "acme",
            "--origin",
            "https://garden.example",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "status {:?}\nstdout {}\nstderr {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());

    let events = json_lines(&output);
    let names: Vec<&str> = events
        .iter()
        .map(|event| event["event"].as_str().unwrap())
        .collect();
    assert_eq!(names.first().copied(), Some("build_started"));
    assert_eq!(names.last().copied(), Some("build_finished"));
    assert!(names.contains(&"tenant_started"));
    assert!(names.contains(&"tenant_compiled"));
    assert!(names.contains(&"page_emitted"));
    assert!(names.contains(&"tenant_finished"));
    assert!(!names.contains(&"build_error"));

    let finished = events.last().unwrap();
    assert_eq!(finished["ok"], true);
    assert_eq!(finished["tenant"], "acme");
    assert!(finished["pages"].as_u64().unwrap() >= 1);

    let hello = fs::read_to_string(dist.join("acme").join("hello").join("index.html")).unwrap();
    assert!(hello.contains("https://garden.example/acme/harbor-log"));
    assert!(!hello.contains("[["));
    assert!(!dist.join("other").exists());
}

#[test]
fn missing_tenant_is_a_json_error() {
    let scratch = Scratch::new("missing");
    scratch.write("hello.md", HELLO);
    let dist = scratch.0.join("dist");
    let output = bosun()
        .args([
            "build",
            "--vault",
            scratch.0.to_str().unwrap(),
            "--output",
            dist.to_str().unwrap(),
            "--tenant",
            "nope",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let events = json_lines(&output);
    let error = events.last().unwrap();
    assert_eq!(error["event"], "build_error");
    assert_eq!(error["ok"], false);
    assert_eq!(error["code"], "unknown_tenant");
    assert_eq!(error["path"], "nope");
    assert!(!dist.exists());
}

#[test]
fn usage_errors_are_json_on_stdout() {
    let output = bosun().arg("build").output().unwrap();
    assert_eq!(output.status.code(), Some(2));
    let events = json_lines(&output);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["event"], "build_error");
    assert_eq!(events[0]["code"], "usage");
    assert_eq!(events[0]["ok"], false);
}

#[test]
fn help_exits_cleanly() {
    let output = bosun().args(["build", "--help"]).output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("--vault"));
    assert!(stdout.contains("--output"));
    assert!(stdout.contains("--tenant"));
}
