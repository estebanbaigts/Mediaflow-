//! Reproducible benchmark: generates a synthetic card in a temp directory,
//! then measures scan, plan and import (copy + SHA-256 verification).
//!
//!     cargo run --release -p mediaflow-core --example bench -- [small_files] [large_files] [large_mb]
//!
//! Defaults: 20000 small files (4 KB) + 4 large files (256 MB).
//! Everything happens under a fresh temporary directory, removed at the end.

use std::io::Write;
use std::time::Instant;

use mediaflow_core::db::GrantKind;
use mediaflow_core::profile::builtin;
use mediaflow_core::{Control, Engine, PlanRequest, RunOptions};

fn main() {
    let args: Vec<u64> = std::env::args()
        .skip(1)
        .filter_map(|a| a.parse().ok())
        .collect();
    let small = *args.first().unwrap_or(&20_000);
    let large = *args.get(1).unwrap_or(&4);
    let large_mb = *args.get(2).unwrap_or(&256);

    let tmp = tempfile::tempdir().expect("tempdir");
    let src = tmp.path().join("card");
    let dst = tmp.path().join("backup");
    std::fs::create_dir_all(&dst).unwrap();
    let mut x: u64 = 0x9E3779B97F4A7C15;
    let mut fill = |buf: &mut [u8]| {
        for b in buf.iter_mut() {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            *b = x as u8;
        }
    };
    let t = Instant::now();
    let mut buf = vec![0u8; 4096];
    for i in 0..small {
        let dir = src.join(format!("DCIM/{:03}", i / 1000));
        std::fs::create_dir_all(&dir).unwrap();
        fill(&mut buf);
        std::fs::write(dir.join(format!("IMG_{i:05}.JPG")), &buf).unwrap();
    }
    let mut chunk = vec![0u8; 8 << 20];
    for i in 0..large {
        let dir = src.join("PRIVATE/M4ROOT/CLIP");
        std::fs::create_dir_all(&dir).unwrap();
        let mut f = std::fs::File::create(dir.join(format!("C{i:04}.MP4"))).unwrap();
        for _ in 0..(large_mb / 8) {
            fill(&mut chunk);
            f.write_all(&chunk).unwrap();
        }
    }
    let total_bytes = small * 4096 + large * large_mb * 1024 * 1024;
    println!(
        "generated {small} small + {large}×{large_mb} MB files ({:.1} MB) in {:.1?}",
        total_bytes as f64 / 1e6,
        t.elapsed()
    );

    let engine = Engine::open(&tmp.path().join("lib.sqlite")).unwrap();
    engine.authorize(&src, GrantKind::Source).unwrap();
    engine.authorize(&dst, GrantKind::Destination).unwrap();

    let t = Instant::now();
    let scan = engine.scan(&src, &Control::new(), |_, _| {}).unwrap();
    println!("scan:   {} files in {:.2?}", scan.files.len(), t.elapsed());

    let req = PlanRequest {
        source: src.clone(),
        source_label: Some("BENCH".into()),
        destinations: vec![dst.clone()],
        project: "Bench".into(),
        profile: builtin("custom").unwrap(),
        hash_candidates: true,
    };
    let t = Instant::now();
    let plan = engine.plan(&req, &Control::new(), |_, _, _| {}).unwrap();
    println!("plan:   {} items in {:.2?}", plan.items.len(), t.elapsed());

    let id = engine.create_session(&plan).unwrap();
    let t = Instant::now();
    let state = engine
        .run_session(&id, &Control::new(), RunOptions::default(), &mut |_| {})
        .unwrap();
    let secs = t.elapsed().as_secs_f64();
    println!(
        "import: {:?} in {:.2} s — {:.0} MB/s source throughput (each byte read once, written once, re-read once)",
        state,
        secs,
        total_bytes as f64 / 1e6 / secs
    );

    // Second plan on the same destination: everything must be recognized.
    let t = Instant::now();
    let plan2 = engine.plan(&req, &Control::new(), |_, _, _| {}).unwrap();
    println!(
        "re-plan (all duplicates, hashes cached for source+dest): {} skips in {:.2?}",
        plan2.summary.skip_duplicates,
        t.elapsed()
    );
}
