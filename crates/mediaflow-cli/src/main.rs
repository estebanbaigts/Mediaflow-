//! `mediaflow` — command-line front end to the MediaFlow engine.
//!
//! Paths given on the command line are explicit user intent: they are
//! registered as authorized folders (source or destination) before use.

use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use clap::{Parser, Subcommand, ValueEnum};
use mediaflow_core::compare::{CompareMode, Presence};
use mediaflow_core::db::GrantKind;
use mediaflow_core::plan::{ImportPlan, PlanAction, PlanNote};
use mediaflow_core::profile::{builtin, builtin_profiles};
use mediaflow_core::report::ReportFormat;
use mediaflow_core::state::SessionState;
use mediaflow_core::util::human_bytes;
use mediaflow_core::{Control, Engine, MfError, PlanRequest, RunOptions};

#[derive(Parser)]
#[command(
    name = "mediaflow",
    version,
    about = "Import, verify and organize media. Offline. Never deletes your files."
)]
struct Cli {
    /// Library database (default: per-user data folder).
    #[arg(long, global = true)]
    library: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// List mounted volumes and free space.
    Volumes,
    /// List built-in profiles.
    Profiles,
    /// Inventory a folder or card (no hashing, nothing written).
    Scan { source: PathBuf },
    /// Show what an import would do, without copying anything.
    Plan(ImportArgs),
    /// Plan, confirm, copy and verify.
    Import {
        #[command(flatten)]
        args: ImportArgs,
        /// Do not ask for confirmation.
        #[arg(long, short)]
        yes: bool,
        #[arg(long, hide = true)]
        debug_throttle_ms: Option<u64>,
    },
    /// Resume a paused, interrupted or partially failed import.
    Resume {
        session: String,
        #[arg(long, hide = true)]
        debug_throttle_ms: Option<u64>,
    },
    /// Cancel a paused or interrupted import (copied files are kept).
    Cancel { session: String },
    /// List import sessions.
    Sessions {
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// Print or export a session report.
    Report {
        session: String,
        #[arg(long, value_enum, default_value_t = Format::Json)]
        format: Format,
        /// Write to this file (never overwrites an existing file).
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Check that destinations hold every file of a source.
    Verify {
        source: PathBuf,
        #[arg(long = "dest", short, required = true)]
        dest: Vec<PathBuf>,
        /// Names and sizes only (fast, proves nothing).
        #[arg(long)]
        quick: bool,
        /// Apply this profile's exclusions (e.g. system files). Excluded
        /// files are listed, not silently ignored.
        #[arg(long)]
        profile: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Search files imported and verified by MediaFlow.
    Library {
        #[arg(default_value = "")]
        query: String,
    },
}

#[derive(clap::Args, Clone)]
struct ImportArgs {
    source: PathBuf,
    #[arg(long = "dest", short, required = true)]
    dest: Vec<PathBuf>,
    #[arg(long, default_value = "youtube")]
    profile: String,
    #[arg(long, default_value = "")]
    project: String,
    /// Source label used by {camera}/{source} (default: folder name).
    #[arg(long)]
    label: Option<String>,
    /// Skip hashing duplicate candidates while planning (they are still
    /// verified before being skipped).
    #[arg(long)]
    quick: bool,
    /// Print the full plan as JSON.
    #[arg(long)]
    json: bool,
}

#[derive(Clone, Copy, ValueEnum)]
enum Format {
    Json,
    Csv,
}

fn default_library() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("MediaFlow")
        .join("library.sqlite")
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match real_main(cli) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {}", e.user_message());
            if std::env::var_os("MEDIAFLOW_DEBUG").is_some() {
                eprintln!("debug: {e:?}");
            }
            ExitCode::from(1)
        }
    }
}

fn exit_for(state: SessionState) -> ExitCode {
    match state {
        SessionState::Completed => ExitCode::SUCCESS,
        SessionState::CompletedWithErrors => ExitCode::from(2),
        SessionState::Paused | SessionState::Interrupted => ExitCode::from(3),
        SessionState::Cancelled => ExitCode::from(4),
        _ => ExitCode::from(1),
    }
}

fn real_main(cli: Cli) -> mediaflow_core::Result<ExitCode> {
    match &cli.cmd {
        Cmd::Volumes => {
            for v in mediaflow_core::volumes::list_volumes() {
                println!(
                    "{:<30} {:>10} free of {:>10}  {}{}{}",
                    v.mount_point,
                    human_bytes(v.available_bytes),
                    human_bytes(v.total_bytes),
                    v.file_system,
                    if v.removable { "  removable" } else { "" },
                    if v.read_only { "  read-only" } else { "" }
                );
            }
            return Ok(ExitCode::SUCCESS);
        }
        Cmd::Profiles => {
            for p in builtin_profiles() {
                println!("{:<12} {:<22} {}", p.id, p.name, p.description);
                println!(
                    "{:<12} layout: {}/{}",
                    "", p.template.folders, p.template.file_name
                );
            }
            return Ok(ExitCode::SUCCESS);
        }
        _ => {}
    }
    let engine = Engine::open(&cli.library.clone().unwrap_or_else(default_library))?;
    match cli.cmd {
        Cmd::Volumes | Cmd::Profiles => unreachable!(),
        Cmd::Scan { source } => {
            engine.authorize(&source, GrantKind::Source)?;
            let r = engine.scan(&source, &Control::new(), |_, _| {})?;
            for f in &r.files {
                println!(
                    "{:>10}  {:<10} {:<14} {}",
                    human_bytes(f.size),
                    f.class.category.as_str(),
                    format!("{:?}", f.class.confidence).to_lowercase(),
                    f.rel_path
                );
            }
            for i in &r.issues {
                println!(
                    "{:>10}  {:<10} {:<14} {}  ({})",
                    "-",
                    "skipped",
                    format!("{:?}", i.kind).to_lowercase(),
                    i.rel_path,
                    i.message
                );
            }
            println!(
                "\n{} files, {}{}",
                r.files.len(),
                human_bytes(r.total_bytes),
                if r.complete {
                    ""
                } else {
                    "  (INCOMPLETE: some folders could not be read)"
                }
            );
            Ok(ExitCode::SUCCESS)
        }
        Cmd::Plan(args) => {
            let plan = make_plan(&engine, &args)?;
            if args.json {
                println!("{}", serde_json::to_string_pretty(&plan)?);
            } else {
                print_plan(&plan, true);
            }
            Ok(ExitCode::SUCCESS)
        }
        Cmd::Import {
            args,
            yes,
            debug_throttle_ms,
        } => {
            let plan = make_plan(&engine, &args)?;
            print_plan(&plan, false);
            if plan.destinations.iter().any(|d| !d.space_ok) {
                eprintln!("\nwarning: at least one destination may not have enough free space.");
            }
            if !yes && !confirm("\nStart the import?")? {
                println!("Nothing was copied.");
                return Ok(ExitCode::SUCCESS);
            }
            let id = engine.create_session(&plan)?;
            println!("Session {id}");
            run(&engine, &id, debug_throttle_ms)
        }
        Cmd::Resume {
            session,
            debug_throttle_ms,
        } => run(&engine, &session, debug_throttle_ms),
        Cmd::Cancel { session } => {
            engine.cancel_session(&session)?;
            println!("Cancelled. Files already copied and verified were kept.");
            Ok(ExitCode::SUCCESS)
        }
        Cmd::Sessions { limit } => {
            for s in engine.sessions(limit)? {
                println!(
                    "{}  {:<22} {:<18} {} → {}  done {} · skipped {} · failed {} · conflicts {} · pending {}",
                    s.created_at.format("%Y-%m-%d %H:%M"),
                    s.state.as_str(),
                    if s.project.is_empty() { "-" } else { &s.project },
                    s.source_label,
                    s.destinations.iter().map(|d| d.label.as_str()).collect::<Vec<_>>().join(", "),
                    s.counts.done,
                    s.counts.skipped_duplicate,
                    s.counts.failed,
                    s.counts.conflict,
                    s.counts.pending
                );
                println!("    id {}", s.id);
            }
            Ok(ExitCode::SUCCESS)
        }
        Cmd::Report {
            session,
            format,
            out,
        } => {
            let r = engine.report(&session)?;
            let f = match format {
                Format::Json => ReportFormat::Json,
                Format::Csv => ReportFormat::Csv,
            };
            match out {
                Some(p) => {
                    r.write(&p, f)?;
                    println!("Report written to {}", p.display());
                }
                None => print!(
                    "{}",
                    if matches!(f, ReportFormat::Json) {
                        r.to_json()? + "\n"
                    } else {
                        r.to_csv()?
                    }
                ),
            }
            Ok(ExitCode::SUCCESS)
        }
        Cmd::Verify {
            source,
            dest,
            quick,
            profile,
            json,
        } => {
            let exclusions = match &profile {
                Some(id) => {
                    builtin(id)
                        .ok_or_else(|| MfError::InvalidInput(format!("unknown profile {id:?}")))?
                        .exclusions
                }
                None => vec![],
            };
            engine.authorize(&source, GrantKind::Source)?;
            for d in &dest {
                if d.is_dir() {
                    engine.authorize(d, GrantKind::Source)?;
                }
            }
            let mode = if quick {
                CompareMode::Quick
            } else {
                CompareMode::Verified
            };
            let r = engine.compare(
                &source,
                &dest,
                mode,
                &exclusions,
                &Control::new(),
                |_, _| {},
            )?;
            if json {
                println!("{}", serde_json::to_string_pretty(&r)?);
            } else {
                for e in &r.entries {
                    let bad: Vec<String> = e
                        .per_destination
                        .iter()
                        .enumerate()
                        .filter(|(_, p)| !matches!(p, Presence::PresentVerified { .. }))
                        .map(|(i, p)| format!("{}: {}", r.destinations[i].root, presence_label(p)))
                        .collect();
                    if !bad.is_empty() {
                        println!("{}  →  {}", e.source_rel, bad.join("; "));
                    }
                }
                for x in &r.excluded {
                    println!("{x}  →  excluded by profile, not checked");
                }
                println!();
                for d in &r.destinations {
                    println!(
                        "{} {}\n    {}",
                        if d.complete {
                            "✔ COMPLETE"
                        } else {
                            "✘ NOT PROVEN COMPLETE"
                        },
                        d.root,
                        d.explanation
                    );
                }
                if let Some(t) = r.previous_check_at {
                    println!(
                        "\nPrevious check of this source: {}",
                        t.format("%Y-%m-%d %H:%M")
                    );
                }
            }
            Ok(if r.destinations.iter().all(|d| d.complete) {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(2)
            })
        }
        Cmd::Library { query } => {
            for l in engine.library(&query, 200)? {
                println!(
                    "{}  {:>10}  {}  {}/{}",
                    l.verified_at.format("%Y-%m-%d"),
                    human_bytes(l.size),
                    &l.sha256[..12],
                    l.dest_root,
                    l.rel_path
                );
            }
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn presence_label(p: &Presence) -> String {
    match p {
        Presence::PresentVerified { rel_path } => format!("verified ({rel_path})"),
        Presence::PresentUnverified { rel_path } => {
            format!("name+size match, NOT verified ({rel_path})")
        }
        Presence::Modified { rel_path } => format!("DIFFERENT content ({rel_path})"),
        Presence::Corrupted { rel_path } => format!("CORRUPTED copy ({rel_path})"),
        Presence::Missing => "MISSING".into(),
        Presence::Undetermined => "undetermined (folder unreadable)".into(),
        Presence::DestinationUnavailable => "destination not connected".into(),
        Presence::SourceUnreadable { message } => format!("source unreadable: {message}"),
    }
}

fn make_plan(engine: &Engine, args: &ImportArgs) -> mediaflow_core::Result<ImportPlan> {
    let profile = builtin(&args.profile).ok_or_else(|| {
        MfError::InvalidInput(format!(
            "unknown profile {:?} (see `mediaflow profiles`)",
            args.profile
        ))
    })?;
    engine.authorize(&args.source, GrantKind::Source)?;
    for d in &args.dest {
        std::fs::create_dir_all(d).map_err(|e| MfError::io(d, e))?;
        engine.authorize(d, GrantKind::Destination)?;
    }
    let req = PlanRequest {
        source: args.source.clone(),
        source_label: args.label.clone(),
        destinations: args.dest.clone(),
        project: args.project.clone(),
        profile,
        hash_candidates: !args.quick,
    };
    let tty = std::io::stderr().is_terminal();
    engine
        .plan(&req, &Control::new(), |phase, i, n| {
            if tty && (i % 200 == 0 || i == n) {
                eprint!(
                    "\r{phase:<22} {i}{}        ",
                    if n > 0 {
                        format!("/{n}")
                    } else {
                        String::new()
                    }
                );
            }
        })
        .inspect(|_| {
            if tty {
                eprint!("\r{:<60}\r", "");
            }
        })
}

fn print_plan(p: &ImportPlan, detailed: bool) {
    let s = &p.summary;
    println!(
        "Source   {}  ({} files, {})",
        p.source_root.display(),
        s.files,
        human_bytes(s.bytes)
    );
    for d in &p.destinations {
        println!(
            "Dest     {}  needs {} · free {}{}",
            d.root.display(),
            human_bytes(d.required_bytes),
            d.available_bytes
                .map(human_bytes)
                .unwrap_or_else(|| "unknown".into()),
            if d.space_ok {
                ""
            } else {
                "  ⚠ NOT ENOUGH SPACE"
            }
        );
    }
    println!(
        "Profile  {}  ·  layout {}/{}",
        p.profile.name, p.profile.template.folders, p.profile.template.file_name
    );
    println!(
        "\n  copy {} ({})  ·  skip verified duplicates {}  ·  conflicts {}  ·  excluded {}  ·  renamed {}  ·  possible duplicates {}  ·  type mismatches {}  ·  not importable {}",
        s.to_copy,
        human_bytes(s.bytes_to_copy),
        s.skip_duplicates,
        s.conflicts,
        s.excluded,
        s.renamed,
        s.possible_duplicates,
        s.type_mismatches,
        s.scan_issues
    );
    if !p.source_scan_complete {
        println!("  ⚠ Some source folders could not be read; the inventory is incomplete.");
    }
    for item in &p.items {
        if let Some(x) = &item.excluded {
            println!(
                "  EXCLUDED  {}  — {} ({})",
                item.source.rel_path, x.reason, x.pattern
            );
            continue;
        }
        for t in &item.targets {
            let flagged = !t.notes.is_empty() || !matches!(t.action, PlanAction::Copy);
            if !detailed && !flagged {
                continue;
            }
            let action = match &t.action {
                PlanAction::Copy => "COPY     ".to_string(),
                PlanAction::SkipDuplicate {
                    existing_rel,
                    level,
                } => format!("SKIP     (= {existing_rel}, {})", level.label()),
                PlanAction::Conflict {
                    existing_rel,
                    level,
                } => format!("CONFLICT (≠ {existing_rel}, {})", level.label()),
            };
            let notes: Vec<String> = t
                .notes
                .iter()
                .map(|n| match n {
                    PlanNote::RenamedExistingDifferent { original_rel } => {
                        format!("renamed: {original_rel} holds different content")
                    }
                    PlanNote::RenamedPlanCollision { original_rel } => {
                        format!("renamed: several source files map to {original_rel}")
                    }
                    PlanNote::PossibleDuplicate {
                        existing_rel,
                        level,
                    } => format!("possible duplicate of {existing_rel} ({})", level.label()),
                    PlanNote::DuplicateInSource { other_rel } => {
                        format!("same content as {other_rel} in the source")
                    }
                    PlanNote::TypeMismatch => "content does not match the extension".into(),
                })
                .collect();
            println!(
                "  {action}  {}  →  [{}] {}{}",
                item.source.rel_path,
                t.dest_index,
                t.target_rel,
                if notes.is_empty() {
                    String::new()
                } else {
                    format!("   ({})", notes.join("; "))
                }
            );
        }
    }
    for i in &p.scan_issues {
        println!("  NOT IMPORTED  {}  — {}", i.rel_path, i.message);
    }
}

fn confirm(q: &str) -> mediaflow_core::Result<bool> {
    if !std::io::stdin().is_terminal() {
        return Err(MfError::InvalidInput(
            "not a terminal: pass --yes to confirm".into(),
        ));
    }
    print!("{q} [y/N] ");
    let _ = std::io::stdout().flush();
    let mut s = String::new();
    std::io::stdin()
        .read_line(&mut s)
        .map_err(|e| MfError::io(Path::new("stdin"), e))?;
    Ok(matches!(s.trim(), "y" | "Y" | "yes" | "o" | "oui"))
}

fn run(engine: &Engine, id: &str, throttle_ms: Option<u64>) -> mediaflow_core::Result<ExitCode> {
    let control = Control::new();
    let c = control.clone();
    // Ctrl-C pauses (resumable) instead of killing mid-write.
    let _ = ctrlc::set_handler(move || {
        if c.is_paused() {
            c.cancel();
        } else {
            eprintln!("\nPausing after the current chunk… (press Ctrl-C again to cancel)");
            c.pause();
        }
    });
    let tty = std::io::stderr().is_terminal();
    let opts = RunOptions {
        throttle: throttle_ms.map(Duration::from_millis),
        ..Default::default()
    };
    let mut last_outcome: Option<(i64, i64)> = None;
    let state = engine.run_session(id, &control, opts, &mut |p| {
        if let Some(o) = &p.last_outcome {
            if last_outcome != Some((o.file_id, o.dest_idx)) {
                last_outcome = Some((o.file_id, o.dest_idx));
                if o.state.as_str() == "failed" || o.state.as_str() == "conflict" {
                    eprintln!(
                        "\r{:<100}\r  ✘ {} — {}",
                        "",
                        o.rel_path,
                        o.message.clone().unwrap_or_default()
                    );
                }
            }
        }
        if tty {
            let rate = p
                .bytes_per_second
                .map(|r| format!("{}/s", human_bytes(r as u64)))
                .unwrap_or_default();
            let eta = p
                .eta_seconds
                .map(|s| format!("~{}s left", s.round() as u64))
                .unwrap_or_default();
            let cur = p
                .current
                .as_ref()
                .map(|c| format!("{:?} {}", c.phase, c.rel_path).to_lowercase())
                .unwrap_or_default();
            let line = format!(
                "{:>5.1}%  {} ok · {} failed  {rate} {eta}  {cur}",
                p.fraction * 100.0,
                p.ops_done + p.ops_skipped,
                p.ops_failed
            );
            let line: String = line.chars().take(110).collect();
            eprint!("\r{line:<110}");
        }
    })?;
    if tty {
        eprintln!();
    }
    let info = engine.session(id)?;
    let c = &info.counts;
    let verdict = match state {
        SessionState::Completed => {
            "✔ Completed — every file copied and verified (SHA-256).".to_string()
        }
        SessionState::CompletedWithErrors => format!(
            "✘ Finished WITH PROBLEMS — {}",
            info.last_error.clone().unwrap_or_default()
        ),
        SessionState::Paused => format!("⏸ Paused. Resume with: mediaflow resume {id}"),
        SessionState::Interrupted => format!(
            "⚠ Interrupted — {} Resume with: mediaflow resume {id}",
            info.last_error.clone().unwrap_or_default()
        ),
        SessionState::Cancelled => {
            "Cancelled. Verified copies were kept; the source was not modified.".into()
        }
        other => format!("State: {}", other.as_str()),
    };
    println!("{verdict}");
    println!(
        "  copied+verified {} ({}) · verified duplicates skipped {} · conflicts {} · failed {} · cancelled {} · pending {}",
        c.done,
        human_bytes(c.bytes_done),
        c.skipped_duplicate,
        c.conflict,
        c.failed,
        c.cancelled,
        c.pending
    );
    Ok(exit_for(state))
}
