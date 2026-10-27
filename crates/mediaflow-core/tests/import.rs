//! Integration tests of the import pipeline: success, failures, interruptions,
//! resume, duplicates, conflicts, multi-destination and corruption.

mod common;

use std::io;
use std::sync::Arc;

use common::*;
use mediaflow_core::executor::Fault;
use mediaflow_core::hash::hash_bytes;
use mediaflow_core::plan::PlanAction;
use mediaflow_core::profile::ConflictPolicy;
use mediaflow_core::state::{OpState, SessionState};
use mediaflow_core::{Control, Engine, MfError};

fn op_states(e: &Engine, id: &str) -> Vec<(String, OpState, Option<String>)> {
    let (files, ops) = e.session_ops(id).unwrap();
    ops.iter()
        .map(|o| {
            (
                files[o.file_id as usize].rel_path.clone(),
                o.state,
                o.error_code.clone(),
            )
        })
        .collect()
}

#[test]
fn copy_success_verified_and_source_untouched() {
    let fx = Fx::new();
    fx.write_src("DCIM/100/C0001.MP4", &data(1, 3 * 1024 * 1024 + 17));
    fx.write_src("DCIM/100/C0002.MP4", &data(2, 1024));
    fx.write_src("AUDIO/zoom.wav", &data(3, 0)); // empty file
    let before = snapshot(&fx.src);
    let e = fx.engine();
    let id = plan_and_create(&e, &request(&fx, mirror(), vec![fx.dst.clone()]));
    assert_eq!(
        run(&e, &id, &Control::new(), None).unwrap(),
        SessionState::Completed
    );

    for (rel, content, _) in &before {
        assert_eq!(&std::fs::read(fx.dst.join(rel)).unwrap(), content, "{rel}");
    }
    assert_eq!(
        snapshot(&fx.src),
        before,
        "source must be byte- and mtime-identical"
    );
    assert!(temps(&fx.dst).is_empty());
    // mtime preserved on copies
    let src_m = std::fs::metadata(fx.src.join("DCIM/100/C0002.MP4"))
        .unwrap()
        .modified()
        .unwrap();
    let dst_m = std::fs::metadata(fx.dst.join("DCIM/100/C0002.MP4"))
        .unwrap()
        .modified()
        .unwrap();
    assert_eq!(src_m, dst_m);

    let r = e.report(&id).unwrap();
    assert!(r.success);
    assert_eq!(r.totals.copied_and_verified, 3);
    assert!(r.files.iter().all(|f| f.sha256.is_some()));
    let f = r
        .files
        .iter()
        .find(|f| f.source_rel == "DCIM/100/C0002.MP4")
        .unwrap();
    assert_eq!(
        f.sha256.as_deref(),
        Some(hash_bytes(&data(2, 1024)).as_str())
    );
    // Library index updated
    assert_eq!(e.library("", 100).unwrap().len(), 3);
}

#[test]
fn source_modified_between_plan_and_run() {
    let fx = Fx::new();
    let p = fx.write_src("a.mov", &data(1, 1000));
    fx.write_src("b.mov", &data(2, 1000));
    let e = fx.engine();
    let id = plan_and_create(&e, &request(&fx, mirror(), vec![fx.dst.clone()]));
    std::fs::write(&p, data(9, 1001)).unwrap();
    assert_eq!(
        run(&e, &id, &Control::new(), None).unwrap(),
        SessionState::CompletedWithErrors
    );
    let st = op_states(&e, &id);
    assert_eq!(st[0].1, OpState::Failed);
    assert_eq!(st[0].2.as_deref(), Some("source_changed"));
    assert_eq!(st[1].1, OpState::Done);
    assert!(!fx.dst.join("a.mov").exists());
    assert!(!e.report(&id).unwrap().success);
}

#[test]
fn source_modified_during_copy() {
    let fx = Fx::new();
    let p = fx.write_src("big.mov", &data(1, 9 * 1024 * 1024));
    let e = fx.engine();
    let id = plan_and_create(&e, &request(&fx, mirror(), vec![fx.dst.clone()]));
    let once = Once::new();
    let hooks = Hooks {
        after_write: Some(Box::new(move |_, _, _| {
            if once.first() {
                // Another program appends to the clip while we copy it.
                use std::io::Write;
                std::fs::OpenOptions::new()
                    .append(true)
                    .open(&p)
                    .unwrap()
                    .write_all(b"more")
                    .unwrap();
            }
            Ok(())
        })),
        ..Default::default()
    };
    assert_eq!(
        run(&e, &id, &Control::new(), Some(Arc::new(hooks))).unwrap(),
        SessionState::CompletedWithErrors
    );
    assert_eq!(op_states(&e, &id)[0].2.as_deref(), Some("source_changed"));
    assert!(!fx.dst.join("big.mov").exists());
    assert!(temps(&fx.dst).is_empty());
}

// Unix only: Windows refuses to rename a folder while a file inside it is
// open, so the unplug is simulated between write and verify there (below).
#[cfg(unix)]
#[test]
fn destination_disconnected_then_resumed() {
    let fx = Fx::new();
    for i in 0..4 {
        fx.write_src(&format!("clip{i}.mp4"), &data(i, 200_000));
    }
    let e = fx.engine();
    let id = plan_and_create(&e, &request(&fx, mirror(), vec![fx.dst.clone()]));
    let dst = fx.dst.clone();
    let parked = fx.dst.with_extension("unplugged");
    let parked2 = parked.clone();
    let hooks = Hooks {
        after_write: Some(Box::new(move |rel, _, _| {
            if rel == "clip2.mp4" {
                // Simulate the drive being unplugged mid-write.
                std::fs::rename(&dst, &parked2).unwrap();
                return Err(Fault::Io(io::Error::new(
                    io::ErrorKind::NotFound,
                    "device gone",
                )));
            }
            Ok(())
        })),
        ..Default::default()
    };
    assert_eq!(
        run(&e, &id, &Control::new(), Some(Arc::new(hooks))).unwrap(),
        SessionState::Interrupted
    );
    let s = e.session(&id).unwrap();
    assert!(s.last_error.unwrap().contains("no longer available"));
    // While unplugged, resume refuses to fake progress.
    assert_eq!(
        run(&e, &id, &Control::new(), None).unwrap(),
        SessionState::Interrupted
    );
    // Reconnect and resume.
    std::fs::rename(&parked, &fx.dst).unwrap();
    assert_eq!(
        run(&e, &id, &Control::new(), None).unwrap(),
        SessionState::Completed
    );
    for i in 0..4 {
        assert_eq!(
            std::fs::read(fx.dst.join(format!("clip{i}.mp4"))).unwrap(),
            data(i, 200_000)
        );
    }
    assert!(temps(&fx.dst).is_empty());
}

#[test]
fn destination_disconnected_before_verify_then_resumed() {
    let fx = Fx::new();
    for i in 0..4 {
        fx.write_src(&format!("clip{i}.mp4"), &data(i, 200_000));
    }
    let e = fx.engine();
    let id = plan_and_create(&e, &request(&fx, mirror(), vec![fx.dst.clone()]));
    let dst = fx.dst.clone();
    let parked = fx.dst.with_extension("unplugged");
    let parked2 = parked.clone();
    let hooks = Hooks {
        before_verify: Some(Box::new(move |rel, _, _| {
            if rel == "clip2.mp4" {
                // Drive unplugged after the copy was written, before it is re-read.
                std::fs::rename(&dst, &parked2).unwrap();
            }
            Ok(())
        })),
        ..Default::default()
    };
    assert_eq!(
        run(&e, &id, &Control::new(), Some(Arc::new(hooks))).unwrap(),
        SessionState::Interrupted
    );
    let s = e.session(&id).unwrap();
    assert!(s.last_error.unwrap().contains("no longer available"));
    // While unplugged, resume refuses to fake progress.
    assert_eq!(
        run(&e, &id, &Control::new(), None).unwrap(),
        SessionState::Interrupted
    );
    // Reconnect and resume.
    std::fs::rename(&parked, &fx.dst).unwrap();
    assert_eq!(
        run(&e, &id, &Control::new(), None).unwrap(),
        SessionState::Completed
    );
    for i in 0..4 {
        assert_eq!(
            std::fs::read(fx.dst.join(format!("clip{i}.mp4"))).unwrap(),
            data(i, 200_000)
        );
    }
    assert!(temps(&fx.dst).is_empty());
}

#[test]
fn destination_full_fails_only_that_file() {
    let fx = Fx::new();
    fx.write_src("a.mov", &data(1, 5000));
    fx.write_src("b.mov", &data(2, 5000));
    let e = fx.engine();
    let id = plan_and_create(&e, &request(&fx, mirror(), vec![fx.dst.clone()]));
    let hooks = Hooks {
        after_write: Some(Box::new(|rel, _, _| {
            if rel == "a.mov" {
                return Err(Fault::Io(io::Error::from(io::ErrorKind::StorageFull)));
            }
            Ok(())
        })),
        ..Default::default()
    };
    assert_eq!(
        run(&e, &id, &Control::new(), Some(Arc::new(hooks))).unwrap(),
        SessionState::CompletedWithErrors
    );
    let st = op_states(&e, &id);
    assert_eq!(st[0].2.as_deref(), Some("storage_full"));
    assert_eq!(st[1].1, OpState::Done);
    assert!(!fx.dst.join("a.mov").exists());
    assert!(temps(&fx.dst).is_empty());
    let msg = e.report(&id).unwrap().files[0].error.clone().unwrap();
    assert!(msg.contains("is full"), "{msg}");
    // Space freed: resume retries the failed file.
    assert_eq!(
        run(&e, &id, &Control::new(), None).unwrap(),
        SessionState::Completed
    );
}

#[test]
fn permission_denied_reported() {
    let fx = Fx::new();
    fx.write_src("a.mov", &data(1, 10));
    let e = fx.engine();
    let id = plan_and_create(&e, &request(&fx, mirror(), vec![fx.dst.clone()]));
    let hooks = Hooks {
        after_write: Some(Box::new(|_, _, _| {
            Err(Fault::Io(io::Error::from(io::ErrorKind::PermissionDenied)))
        })),
        ..Default::default()
    };
    assert_eq!(
        run(&e, &id, &Control::new(), Some(Arc::new(hooks))).unwrap(),
        SessionState::CompletedWithErrors
    );
    assert_eq!(
        op_states(&e, &id)[0].2.as_deref(),
        Some("permission_denied")
    );
}

#[cfg(unix)]
#[test]
fn real_permission_denied_when_not_root() {
    use std::os::unix::fs::PermissionsExt;
    if unsafe { libc_geteuid() } == 0 {
        eprintln!("skipped: running as root bypasses permissions");
        return;
    }
    let fx = Fx::new();
    fx.write_src("sub/a.mov", &data(1, 10));
    std::fs::create_dir(fx.dst.join("sub")).unwrap();
    std::fs::set_permissions(fx.dst.join("sub"), std::fs::Permissions::from_mode(0o555)).unwrap();
    let e = fx.engine();
    let id = plan_and_create(&e, &request(&fx, mirror(), vec![fx.dst.clone()]));
    assert_eq!(
        run(&e, &id, &Control::new(), None).unwrap(),
        SessionState::CompletedWithErrors
    );
    assert_eq!(
        op_states(&e, &id)[0].2.as_deref(),
        Some("permission_denied")
    );
    std::fs::set_permissions(fx.dst.join("sub"), std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[cfg(unix)]
unsafe extern "C" {
    #[link_name = "geteuid"]
    fn libc_geteuid() -> u32;
}

#[test]
fn identical_file_already_present_is_skipped_after_verification() {
    let fx = Fx::new();
    fx.write_src("a.mov", &data(1, 4000));
    write(&fx.dst, "a.mov", &data(1, 4000));
    let before = std::fs::metadata(fx.dst.join("a.mov"))
        .unwrap()
        .modified()
        .unwrap();
    let e = fx.engine();
    let req = request(&fx, mirror(), vec![fx.dst.clone()]);
    let plan = e.plan(&req, &Control::new(), |_, _, _| {}).unwrap();
    assert!(matches!(
        plan.items[0].targets[0].action,
        PlanAction::SkipDuplicate { .. }
    ));
    let id = e.create_session(&plan).unwrap();
    assert_eq!(
        run(&e, &id, &Control::new(), None).unwrap(),
        SessionState::Completed
    );
    assert_eq!(op_states(&e, &id)[0].1, OpState::SkippedDuplicate);
    assert_eq!(
        std::fs::metadata(fx.dst.join("a.mov"))
            .unwrap()
            .modified()
            .unwrap(),
        before,
        "existing file untouched"
    );
    assert_eq!(all_files(&fx.dst), vec!["a.mov"]);
}

#[test]
fn duplicate_with_different_name_detected_by_content() {
    let fx = Fx::new();
    fx.write_src("C0001.MP4", &data(7, 8000));
    write(&fx.dst, "renamed/my_clip.mp4", &data(7, 8000));
    let e = fx.engine();
    let req = request(&fx, mirror(), vec![fx.dst.clone()]);
    let plan = e.plan(&req, &Control::new(), |_, _, _| {}).unwrap();
    match &plan.items[0].targets[0].action {
        PlanAction::SkipDuplicate {
            existing_rel,
            level,
        } => {
            assert_eq!(existing_rel, "renamed/my_clip.mp4");
            assert!(level.is_proven_identical());
        }
        a => panic!("{a:?}"),
    }
    let id = e.create_session(&plan).unwrap();
    assert_eq!(
        run(&e, &id, &Control::new(), None).unwrap(),
        SessionState::Completed
    );
    assert_eq!(all_files(&fx.dst), vec!["renamed/my_clip.mp4"]);
}

#[test]
fn same_size_different_content_is_not_a_duplicate() {
    let fx = Fx::new();
    fx.write_src("C0001.MP4", &data(1, 8000));
    write(&fx.dst, "other.mp4", &data(2, 8000)); // same size, different bytes
    let e = fx.engine();
    let id = plan_and_create(&e, &request(&fx, mirror(), vec![fx.dst.clone()]));
    assert_eq!(
        run(&e, &id, &Control::new(), None).unwrap(),
        SessionState::Completed
    );
    assert_eq!(all_files(&fx.dst), vec!["C0001.MP4", "other.mp4"]);
}

#[test]
fn quick_plan_duplicate_reverified_at_execution() {
    // Without plan-time hashing, a same-name same-size file is only a
    // candidate. Execution must hash both and copy if they differ.
    let fx = Fx::new();
    fx.write_src("a.mov", &data(1, 4000));
    write(&fx.dst, "a.mov", &data(2, 4000));
    let e = fx.engine();
    let mut req = request(&fx, mirror(), vec![fx.dst.clone()]);
    req.hash_candidates = false;
    let plan = e.plan(&req, &Control::new(), |_, _, _| {}).unwrap();
    assert!(matches!(
        plan.items[0].targets[0].action,
        PlanAction::SkipDuplicate { .. }
    ));
    let id = e.create_session(&plan).unwrap();
    assert_eq!(
        run(&e, &id, &Control::new(), None).unwrap(),
        SessionState::Completed
    );
    assert_eq!(
        std::fs::read(fx.dst.join("a.mov")).unwrap(),
        data(2, 4000),
        "existing untouched"
    );
    assert_eq!(
        std::fs::read(fx.dst.join("a (2).mov")).unwrap(),
        data(1, 4000)
    );
}

#[test]
fn same_name_different_content_is_renamed_never_overwritten() {
    let fx = Fx::new();
    fx.write_src("a.mov", &data(1, 4000));
    write(&fx.dst, "a.mov", &data(2, 3000));
    let e = fx.engine();
    let id = plan_and_create(&e, &request(&fx, mirror(), vec![fx.dst.clone()]));
    assert_eq!(
        run(&e, &id, &Control::new(), None).unwrap(),
        SessionState::Completed
    );
    assert_eq!(std::fs::read(fx.dst.join("a.mov")).unwrap(), data(2, 3000));
    assert_eq!(
        std::fs::read(fx.dst.join("a (2).mov")).unwrap(),
        data(1, 4000)
    );
}

#[test]
fn same_name_different_content_flag_policy() {
    let fx = Fx::new();
    fx.write_src("a.mov", &data(1, 4000));
    write(&fx.dst, "a.mov", &data(2, 3000));
    let e = fx.engine();
    let mut p = mirror();
    p.conflicts = ConflictPolicy::Flag;
    let id = plan_and_create(&e, &request(&fx, p, vec![fx.dst.clone()]));
    assert_eq!(
        run(&e, &id, &Control::new(), None).unwrap(),
        SessionState::CompletedWithErrors
    );
    assert_eq!(op_states(&e, &id)[0].1, OpState::Conflict);
    assert_eq!(all_files(&fx.dst), vec!["a.mov"]);
    assert_eq!(std::fs::read(fx.dst.join("a.mov")).unwrap(), data(2, 3000));
}

#[test]
fn source_collisions_in_flat_layout_are_renamed() {
    // Cameras roll over folders: DCIM/100/IMG_0001.JPG and DCIM/101/IMG_0001.JPG.
    let fx = Fx::new();
    fx.write_src("DCIM/100/IMG_0001.JPG", &data(1, 100));
    fx.write_src("DCIM/101/IMG_0001.JPG", &data(2, 100));
    fx.write_src("DCIM/102/img_0001.jpg", &data(3, 100)); // case-insensitive collision
    let e = fx.engine();
    let mut p = mirror();
    p.template.folders = "".into();
    let id = plan_and_create(&e, &request(&fx, p, vec![fx.dst.clone()]));
    assert_eq!(
        run(&e, &id, &Control::new(), None).unwrap(),
        SessionState::Completed
    );
    assert_eq!(
        all_files(&fx.dst),
        vec!["IMG_0001 (2).JPG", "IMG_0001.JPG", "img_0001 (3).jpg"]
    );
}

#[test]
fn cancel_mid_transfer() {
    let fx = Fx::new();
    let before_data: Vec<_> = (0..3).map(|i| data(i, 300_000)).collect();
    for (i, d) in before_data.iter().enumerate() {
        fx.write_src(&format!("c{i}.mp4"), d);
    }
    let before = snapshot(&fx.src);
    let e = fx.engine();
    let id = plan_and_create(&e, &request(&fx, mirror(), vec![fx.dst.clone()]));
    let control = Control::new();
    let c2 = control.clone();
    let hooks = Hooks {
        after_write: Some(Box::new(move |rel, _, _| {
            if rel == "c1.mp4" {
                c2.cancel();
            }
            Ok(())
        })),
        ..Default::default()
    };
    assert_eq!(
        run(&e, &id, &control, Some(Arc::new(hooks))).unwrap(),
        SessionState::Cancelled
    );
    let st = op_states(&e, &id);
    assert_eq!(st[0].1, OpState::Done);
    assert_eq!(st[1].1, OpState::Cancelled);
    assert_eq!(st[2].1, OpState::Cancelled);
    assert_eq!(
        all_files(&fx.dst),
        vec!["c0.mp4"],
        "no partial or temp file left"
    );
    assert_eq!(snapshot(&fx.src), before);
    // Cancelled is terminal.
    assert!(matches!(
        run(&e, &id, &Control::new(), None),
        Err(MfError::InvalidTransition { .. })
    ));
}

#[test]
fn pause_and_resume() {
    let fx = Fx::new();
    for i in 0..3 {
        fx.write_src(&format!("c{i}.mp4"), &data(i, 100_000));
    }
    let e = fx.engine();
    let id = plan_and_create(&e, &request(&fx, mirror(), vec![fx.dst.clone()]));
    let control = Control::new();
    let c2 = control.clone();
    let hooks = Hooks {
        after_write: Some(Box::new(move |rel, _, _| {
            if rel == "c1.mp4" {
                c2.pause();
            }
            Ok(())
        })),
        ..Default::default()
    };
    assert_eq!(
        run(&e, &id, &control, Some(Arc::new(hooks))).unwrap(),
        SessionState::Paused
    );
    assert_eq!(all_files(&fx.dst), vec!["c0.mp4"]);
    assert_eq!(
        run(&e, &id, &Control::new(), None).unwrap(),
        SessionState::Completed
    );
    assert_eq!(all_files(&fx.dst), vec!["c0.mp4", "c1.mp4", "c2.mp4"]);
}

#[test]
fn crash_and_restart_resumes_without_false_completion() {
    let fx = Fx::new();
    for i in 0..4 {
        fx.write_src(&format!("c{i}.mp4"), &data(i, 150_000));
    }
    let id;
    {
        let e = fx.engine();
        id = plan_and_create(&e, &request(&fx, mirror(), vec![fx.dst.clone()]));
        let hooks = Hooks {
            after_write: Some(Box::new(|rel, _, _| {
                if rel == "c2.mp4" {
                    Err(Fault::Crash)
                } else {
                    Ok(())
                }
            })),
            ..Default::default()
        };
        assert!(matches!(
            run(&e, &id, &Control::new(), Some(Arc::new(hooks))),
            Err(MfError::SimulatedCrash)
        ));
        // A partial temp file is left behind, exactly as after a kill.
        assert_eq!(temps(&fx.dst).len(), 1);
        assert!(!fx.dst.join("c2.mp4").exists());
    } // engine dropped = process gone
    let e = Engine::open(&fx.db).unwrap();
    let s = e.session(&id).unwrap();
    assert_eq!(s.state, SessionState::Interrupted);
    assert_eq!(s.counts.done, 2, "only verified files count as done");
    assert_eq!(
        run(&e, &id, &Control::new(), None).unwrap(),
        SessionState::Completed
    );
    assert!(temps(&fx.dst).is_empty());
    for i in 0..4 {
        assert_eq!(
            std::fs::read(fx.dst.join(format!("c{i}.mp4"))).unwrap(),
            data(i, 150_000)
        );
    }
    let ev = e.events(&id).unwrap();
    assert!(ev.iter().any(|x| x.code == "temp_discarded"));
}

#[test]
fn crash_after_finalize_before_journal() {
    let fx = Fx::new();
    fx.write_src("c0.mp4", &data(0, 50_000));
    let id;
    {
        let e = fx.engine();
        id = plan_and_create(&e, &request(&fx, mirror(), vec![fx.dst.clone()]));
        let hooks = Hooks {
            after_finalize: Some(Box::new(|_, _| Err(Fault::Crash))),
            ..Default::default()
        };
        assert!(run(&e, &id, &Control::new(), Some(Arc::new(hooks))).is_err());
        assert!(fx.dst.join("c0.mp4").exists());
    }
    let e = Engine::open(&fx.db).unwrap();
    assert_eq!(
        e.session(&id).unwrap().counts.done,
        0,
        "journal never said done"
    );
    assert_eq!(
        run(&e, &id, &Control::new(), None).unwrap(),
        SessionState::Completed
    );
    // Recognized as the same content: no "(2)" duplicate created.
    assert_eq!(all_files(&fx.dst), vec!["c0.mp4"]);
    assert!(e
        .events(&id)
        .unwrap()
        .iter()
        .any(|x| x.code == "already_present_verified"));
}

#[test]
fn abandoned_temp_before_verify_is_discarded() {
    let fx = Fx::new();
    fx.write_src("x/c0.mp4", &data(0, 50_000));
    let id;
    {
        let e = fx.engine();
        id = plan_and_create(&e, &request(&fx, mirror(), vec![fx.dst.clone()]));
        let hooks = Hooks {
            before_verify: Some(Box::new(|_, _, _| Err(Fault::Crash))),
            ..Default::default()
        };
        assert!(run(&e, &id, &Control::new(), Some(Arc::new(hooks))).is_err());
        assert_eq!(temps(&fx.dst).len(), 1);
    }
    let e = Engine::open(&fx.db).unwrap();
    assert_eq!(
        run(&e, &id, &Control::new(), None).unwrap(),
        SessionState::Completed
    );
    assert_eq!(all_files(&fx.dst), vec!["x/c0.mp4"]);
}

#[test]
fn foreign_tempfile_like_name_is_never_deleted() {
    // A user file that merely looks like ours, not recorded in the journal.
    let fx = Fx::new();
    fx.write_src("a.mov", &data(1, 10));
    write(&fx.dst, ".mfpart-userfile", b"keep me");
    let e = fx.engine();
    let id = plan_and_create(&e, &request(&fx, mirror(), vec![fx.dst.clone()]));
    assert_eq!(
        run(&e, &id, &Control::new(), None).unwrap(),
        SessionState::Completed
    );
    assert_eq!(
        std::fs::read(fx.dst.join(".mfpart-userfile")).unwrap(),
        b"keep me"
    );
}

#[test]
fn corruption_detected_after_copy() {
    let fx = Fx::new();
    fx.write_src("a.mov", &data(1, 100_000));
    let before = snapshot(&fx.src);
    let e = fx.engine();
    let id = plan_and_create(&e, &request(&fx, mirror(), vec![fx.dst.clone()]));
    let once = Once::new();
    let hooks = Hooks {
        before_verify: Some(Box::new(move |_, _, temp| {
            if once.first() {
                // Flip one bit in the written copy (bad cable, failing flash...).
                let mut b = std::fs::read(temp).unwrap();
                b[5000] ^= 0x01;
                std::fs::write(temp, b).unwrap();
            }
            Ok(())
        })),
        ..Default::default()
    };
    assert_eq!(
        run(&e, &id, &Control::new(), Some(Arc::new(hooks))).unwrap(),
        SessionState::CompletedWithErrors
    );
    assert_eq!(
        op_states(&e, &id)[0].2.as_deref(),
        Some("verification_failed")
    );
    assert!(
        all_files(&fx.dst).is_empty(),
        "corrupted copy discarded, never finalized"
    );
    assert_eq!(snapshot(&fx.src), before);
    // Retry succeeds.
    assert_eq!(
        run(&e, &id, &Control::new(), None).unwrap(),
        SessionState::Completed
    );
    assert_eq!(
        std::fs::read(fx.dst.join("a.mov")).unwrap(),
        data(1, 100_000)
    );
}

#[test]
fn multiple_destinations_independent_failures() {
    let fx = Fx::new();
    fx.write_src("a.mov", &data(1, 70_000));
    fx.write_src("b.mov", &data(2, 70_000));
    let e = fx.engine();
    let id = plan_and_create(
        &e,
        &request(&fx, mirror(), vec![fx.dst.clone(), fx.dst2.clone()]),
    );
    let hooks = Hooks {
        before_verify: Some(Box::new(|rel, d, temp| {
            if rel == "a.mov" && d == 1 {
                let mut b = std::fs::read(temp).unwrap();
                b[0] ^= 0xFF;
                std::fs::write(temp, b).unwrap();
            }
            Ok(())
        })),
        ..Default::default()
    };
    assert_eq!(
        run(&e, &id, &Control::new(), Some(Arc::new(hooks))).unwrap(),
        SessionState::CompletedWithErrors
    );
    assert_eq!(all_files(&fx.dst), vec!["a.mov", "b.mov"]);
    assert_eq!(all_files(&fx.dst2), vec!["b.mov"]);
    let r = e.report(&id).unwrap();
    assert_eq!(r.totals.copied_and_verified, 3);
    assert_eq!(r.totals.failed, 1);
    assert_eq!(
        run(&e, &id, &Control::new(), None).unwrap(),
        SessionState::Completed
    );
    assert_eq!(all_files(&fx.dst2), vec!["a.mov", "b.mov"]);
}

#[test]
fn unicode_and_long_names() {
    let fx = Fx::new();
    let long = format!("{}.mov", "é".repeat(120)); // 240 bytes + ext
    let names = [
        "Été 2024/Caméra A/clip été.mov",
        "日本語/ファイル.mp4",
        "emoji 🎬/take 1.wav",
        "deep/a/b/c/d/e/f/g/h/i/j/k/x.mov",
    ];
    for (i, n) in names.iter().enumerate() {
        fx.write_src(n, &data(i as u64, 1000));
    }
    fx.write_src(&format!("long/{long}"), &data(99, 10));
    let e = fx.engine();
    let id = plan_and_create(&e, &request(&fx, mirror(), vec![fx.dst.clone()]));
    assert_eq!(
        run(&e, &id, &Control::new(), None).unwrap(),
        SessionState::Completed
    );
    for (i, n) in names.iter().enumerate() {
        assert_eq!(std::fs::read(fx.dst.join(n)).unwrap(), data(i as u64, 1000));
    }
    assert!(fx.dst.join("long").join(&long).exists());
}

#[test]
fn excluded_files_are_reported_not_hidden() {
    let fx = Fx::new();
    fx.write_src("DCIM/.DS_Store", b"junk");
    fx.write_src("DCIM/C0001.MP4", &data(1, 100));
    let e = fx.engine();
    let req = request(&fx, mirror(), vec![fx.dst.clone()]);
    let plan = e.plan(&req, &Control::new(), |_, _, _| {}).unwrap();
    assert_eq!(plan.summary.excluded, 1);
    let ex = plan.items.iter().find(|i| i.excluded.is_some()).unwrap();
    assert_eq!(ex.source.rel_path, "DCIM/.DS_Store");
    let id = e.create_session(&plan).unwrap();
    assert_eq!(
        run(&e, &id, &Control::new(), None).unwrap(),
        SessionState::Completed
    );
    let r = e.report(&id).unwrap();
    assert_eq!(r.excluded.len(), 1);
    assert!(r.to_csv().unwrap().contains("DCIM/.DS_Store"));
}

#[test]
fn paranoid_final_pass_detects_tampering() {
    let fx = Fx::new();
    fx.write_src("a.mov", &data(1, 10_000));
    fx.write_src("b.mov", &data(2, 10_000));
    let e = fx.engine();
    let mut p = mirror();
    p.verify = mediaflow_core::profile::VerifyLevel::Paranoid;
    p.preserve_mtime = true;
    let id = plan_and_create(&e, &request(&fx, p, vec![fx.dst.clone()]));
    let dst = fx.dst.clone();
    let hooks = Hooks {
        after_finalize: Some(Box::new(move |rel, _| {
            if rel == "b.mov" {
                // Same size, different content, original mtime restored: only a re-read can tell.
                let path = dst.join("a.mov");
                let m = std::fs::metadata(&path).unwrap().modified().unwrap();
                let mut b = std::fs::read(&path).unwrap();
                b[10] ^= 1;
                std::fs::write(&path, b).unwrap();
                filetime_set(&path, m);
            }
            Ok(())
        })),
        ..Default::default()
    };
    assert_eq!(
        run(&e, &id, &Control::new(), Some(Arc::new(hooks))).unwrap(),
        SessionState::CompletedWithErrors
    );
    let st = op_states(&e, &id);
    assert_eq!(st[0].2.as_deref(), Some("verification_failed"));
}

fn filetime_set(p: &std::path::Path, t: std::time::SystemTime) {
    let f = std::fs::OpenOptions::new().write(true).open(p).unwrap();
    f.set_modified(t).unwrap();
}

#[test]
fn done_file_deleted_before_resume_is_recopied() {
    let fx = Fx::new();
    for i in 0..3 {
        fx.write_src(&format!("c{i}.mp4"), &data(i, 1000));
    }
    let e = fx.engine();
    let id = plan_and_create(&e, &request(&fx, mirror(), vec![fx.dst.clone()]));
    let control = Control::new();
    let c2 = control.clone();
    let hooks = Hooks {
        after_write: Some(Box::new(move |rel, _, _| {
            if rel == "c2.mp4" {
                c2.pause();
            }
            Ok(())
        })),
        ..Default::default()
    };
    assert_eq!(
        run(&e, &id, &control, Some(Arc::new(hooks))).unwrap(),
        SessionState::Paused
    );
    std::fs::remove_file(fx.dst.join("c0.mp4")).unwrap(); // test fixture only
    assert_eq!(
        run(&e, &id, &Control::new(), None).unwrap(),
        SessionState::Completed
    );
    assert_eq!(all_files(&fx.dst), vec!["c0.mp4", "c1.mp4", "c2.mp4"]);
    assert!(e
        .events(&id)
        .unwrap()
        .iter()
        .any(|x| x.code == "copy_missing"));
}

#[test]
fn second_engine_on_same_library_is_refused() {
    let fx = Fx::new();
    let _a = Engine::open(&fx.db).unwrap();
    assert!(matches!(Engine::open(&fx.db), Err(MfError::Locked(_))));
}

#[test]
fn progress_is_monotonic_and_reaches_one() {
    let fx = Fx::new();
    for i in 0..5 {
        fx.write_src(&format!("c{i}.mp4"), &data(i, 500_000));
    }
    let e = fx.engine();
    let id = plan_and_create(&e, &request(&fx, mirror(), vec![fx.dst.clone()]));
    let mut fr = vec![];
    let st = e
        .run_session(&id, &Control::new(), Default::default(), &mut |p| {
            fr.push((p.fraction, p.ops_done))
        })
        .unwrap();
    assert_eq!(st, SessionState::Completed);
    assert!(fr.windows(2).all(|w| w[1].0 >= w[0].0 - 1e-9), "{fr:?}");
    assert!((fr.last().unwrap().0 - 1.0).abs() < 1e-9);
    assert_eq!(fr.last().unwrap().1, 5);
}
