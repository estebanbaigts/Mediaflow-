//! End-to-end: kill the real `mediaflow` process mid-import (SIGKILL on
//! Unix, TerminateProcess on Windows), then resume from a new process.

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_mediaflow")
}

fn content(i: u8, len: usize) -> Vec<u8> {
    (0..len)
        .map(|k| (k as u8).wrapping_mul(31).wrapping_add(i))
        .collect()
}

fn temps(dir: &std::path::Path) -> usize {
    std::fs::read_dir(dir)
        .unwrap()
        .filter(|e| {
            e.as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".mfpart-")
        })
        .count()
}

#[test]
fn killed_process_resumes_cleanly() {
    let tmp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(tmp.path()).unwrap();
    let (src, dst, lib) = (
        root.join("card"),
        root.join("backup"),
        root.join("lib.sqlite"),
    );
    std::fs::create_dir_all(&src).unwrap();
    let n = 6u8;
    let size = 9 * 1024 * 1024; // 3 chunks per file
    for i in 0..n {
        std::fs::write(src.join(format!("C{i:04}.MP4")), content(i, size)).unwrap();
    }

    let mut child = Command::new(bin())
        .args([
            "--library",
            lib.to_str().unwrap(),
            "import",
            src.to_str().unwrap(),
            "--dest",
            dst.to_str().unwrap(),
        ])
        .args(["--profile", "custom", "--yes", "--debug-throttle-ms", "60"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut out = BufReader::new(child.stdout.take().unwrap());
    let mut id = None;
    let mut line = String::new();
    while out.read_line(&mut line).unwrap() > 0 {
        if let Some(rest) = line.trim().strip_prefix("Session ") {
            id = Some(rest.to_string());
            break;
        }
        line.clear();
    }
    let id = id.expect("session id printed");
    // Let it copy a few files, then kill it hard.
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(500) {
        std::thread::sleep(Duration::from_millis(20));
    }
    child.kill().unwrap();
    child.wait().unwrap();

    let finished_before = walk(&dst)
        .iter()
        .filter(|p| {
            !p.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(".mfpart-")
        })
        .count();
    assert!(
        finished_before < n as usize,
        "process should have been killed before finishing ({finished_before})"
    );

    let status = Command::new(bin())
        .args(["--library", lib.to_str().unwrap(), "resume", &id])
        .stdout(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "resume exit status {status:?}");

    let proj = dst.clone();
    for i in 0..n {
        let rel = format!("C{i:04}.MP4");
        let found = walk(&proj)
            .into_iter()
            .find(|p| p.file_name().unwrap().to_string_lossy() == rel)
            .expect(&rel);
        assert_eq!(std::fs::read(found).unwrap(), content(i, size), "{rel}");
    }
    assert_eq!(walk(&dst).len(), n as usize, "no duplicates, no temp files");
    assert_eq!(temps(&dst), 0);

    let report = Command::new(bin())
        .args(["--library", lib.to_str().unwrap(), "report", &id])
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&report.stdout).unwrap();
    assert_eq!(v["state"], "completed");
    assert_eq!(v["totals"]["copied_and_verified"], n as u64);
    let interrupted = v["events"]
        .as_array()
        .unwrap()
        .iter()
        .any(|e| e["code"] == "interrupted");
    assert!(interrupted, "the kill was detected and journaled");
}

fn walk(d: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = vec![];
    if let Ok(rd) = std::fs::read_dir(d) {
        for e in rd.flatten() {
            if e.file_type().unwrap().is_dir() {
                out.extend(walk(&e.path()));
            } else {
                out.push(e.path());
            }
        }
    }
    out
}
