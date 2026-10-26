#![allow(dead_code)]
//! Shared fixtures. Everything happens inside fresh temporary directories
//! with generated files; tests never touch real media or home folders.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use mediaflow_core::db::GrantKind;
use mediaflow_core::executor::{Fault, IoHooks, RunOptions};
use mediaflow_core::profile::{builtin, Profile};
use mediaflow_core::progress::Progress;
use mediaflow_core::state::SessionState;
use mediaflow_core::{Control, Engine, PlanRequest};

pub struct Fx {
    pub tmp: tempfile::TempDir,
    pub src: PathBuf,
    pub dst: PathBuf,
    pub dst2: PathBuf,
    pub db: PathBuf,
}

impl Fx {
    pub fn new() -> Fx {
        let tmp = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(tmp.path()).unwrap();
        let (src, dst, dst2) = (root.join("card"), root.join("backup"), root.join("backup2"));
        for d in [&src, &dst, &dst2] {
            std::fs::create_dir_all(d).unwrap();
        }
        let db = root.join("lib").join("mediaflow.sqlite");
        Fx {
            tmp,
            src,
            dst,
            dst2,
            db,
        }
    }

    pub fn engine(&self) -> Engine {
        let e = Engine::open(&self.db).unwrap();
        e.authorize(&self.src, GrantKind::Source).unwrap();
        e.authorize(&self.dst, GrantKind::Destination).unwrap();
        e.authorize(&self.dst2, GrantKind::Destination).unwrap();
        e
    }

    pub fn write_src(&self, rel: &str, data: &[u8]) -> PathBuf {
        write(&self.src, rel, data)
    }
}

pub fn write(root: &Path, rel: &str, data: &[u8]) -> PathBuf {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, data).unwrap();
    p
}

/// Deterministic pseudo-random content.
pub fn data(seed: u64, len: usize) -> Vec<u8> {
    let mut x = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407)
        | 1;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 24) as u8
        })
        .collect()
}

/// Profile that mirrors the source layout (easy assertions).
pub fn mirror() -> Profile {
    let mut p = builtin("custom").unwrap();
    p.template.folders = "{original_dir}".into();
    p
}

pub fn request(fx: &Fx, profile: Profile, dests: Vec<PathBuf>) -> PlanRequest {
    PlanRequest {
        source: fx.src.clone(),
        source_label: Some("CARD".into()),
        destinations: dests,
        project: "Test".into(),
        profile,
        hash_candidates: true,
    }
}

pub fn plan_and_create(e: &Engine, req: &PlanRequest) -> String {
    let plan = e.plan(req, &Control::new(), |_, _, _| {}).unwrap();
    e.create_session(&plan).unwrap()
}

pub fn run(
    e: &Engine,
    id: &str,
    control: &Control,
    hooks: Option<Arc<dyn IoHooks>>,
) -> mediaflow_core::Result<SessionState> {
    let mut opts = RunOptions::default();
    if let Some(h) = hooks {
        opts.hooks = h;
    }
    let mut last = Progress::default();
    e.run_session(id, control, opts, &mut |p| last = p.clone())
}

pub fn all_files(root: &Path) -> Vec<String> {
    let mut v: Vec<String> = walkdir(root)
        .into_iter()
        .map(|p| {
            p.strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect();
    v.sort();
    v
}

fn walkdir(root: &Path) -> Vec<PathBuf> {
    let mut out = vec![];
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap() {
            let e = e.unwrap();
            let ft = e.file_type().unwrap();
            if ft.is_dir() {
                stack.push(e.path());
            } else {
                out.push(e.path());
            }
        }
    }
    out
}

pub fn temps(root: &Path) -> Vec<String> {
    all_files(root)
        .into_iter()
        .filter(|f| f.rsplit('/').next().unwrap().starts_with(".mfpart-"))
        .collect()
}

type WriteFn = dyn Fn(&str, usize, u64) -> Result<(), Fault> + Send + Sync;
type VerifyFn = dyn Fn(&str, usize, &Path) -> Result<(), Fault> + Send + Sync;
type FinFn = dyn Fn(&str, usize) -> Result<(), Fault> + Send + Sync;

/// Closure-based fault injection.
#[derive(Default)]
pub struct Hooks {
    pub after_write: Option<Box<WriteFn>>,
    pub before_verify: Option<Box<VerifyFn>>,
    pub before_finalize: Option<Box<FinFn>>,
    pub after_finalize: Option<Box<FinFn>>,
}

impl IoHooks for Hooks {
    fn after_write(&self, rel: &str, d: usize, w: u64) -> Result<(), Fault> {
        self.after_write.as_ref().map_or(Ok(()), |f| f(rel, d, w))
    }
    fn before_verify(&self, rel: &str, d: usize, t: &Path) -> Result<(), Fault> {
        self.before_verify.as_ref().map_or(Ok(()), |f| f(rel, d, t))
    }
    fn before_finalize(&self, rel: &str, d: usize) -> Result<(), Fault> {
        self.before_finalize.as_ref().map_or(Ok(()), |f| f(rel, d))
    }
    fn after_finalize(&self, rel: &str, d: usize) -> Result<(), Fault> {
        self.after_finalize.as_ref().map_or(Ok(()), |f| f(rel, d))
    }
}

/// Fire a fault only the first time the predicate matches.
pub struct Once(pub AtomicUsize);
impl Once {
    pub fn new() -> Arc<Once> {
        Arc::new(Once(AtomicUsize::new(0)))
    }
    pub fn first(&self) -> bool {
        self.0.fetch_add(1, Ordering::SeqCst) == 0
    }
}

pub fn snapshot(root: &Path) -> Vec<(String, Vec<u8>, std::time::SystemTime)> {
    all_files(root)
        .into_iter()
        .map(|r| {
            let p = root.join(&r);
            (
                r,
                std::fs::read(&p).unwrap(),
                std::fs::metadata(&p).unwrap().modified().unwrap(),
            )
        })
        .collect()
}

pub type Shared<T> = Arc<Mutex<T>>;
