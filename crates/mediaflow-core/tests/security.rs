//! Security boundary tests: authorization, traversal, symlinks, TOCTOU.

mod common;

use std::sync::Arc;

use common::*;
use mediaflow_core::db::GrantKind;
use mediaflow_core::state::SessionState;
use mediaflow_core::{Control, Engine, MfError};

#[test]
fn unauthorized_paths_refused() {
    let fx = Fx::new();
    fx.write_src("a.mov", b"x");
    let e = Engine::open(&fx.db).unwrap();
    // Nothing granted yet.
    assert!(matches!(
        e.scan(&fx.src, &Control::new(), |_, _| {}),
        Err(MfError::NotAuthorized(_))
    ));
    e.authorize(&fx.src, GrantKind::Source).unwrap();
    let req = request(&fx, mirror(), vec![fx.dst.clone()]);
    assert!(matches!(
        e.plan(&req, &Control::new(), |_, _, _| {}),
        Err(MfError::NotAuthorized(_))
    ));
    // A source grant does not allow writing.
    e.authorize(&fx.dst, GrantKind::Source).unwrap();
    assert!(matches!(
        e.plan(&req, &Control::new(), |_, _, _| {}),
        Err(MfError::NotAuthorized(_))
    ));
}

#[test]
fn traversal_via_dotdot_is_contained() {
    let fx = Fx::new();
    let e = fx.engine();
    // `card/../backup2` canonicalizes to an authorized folder, but
    // `card/../../` would escape: refused.
    let outside = fx.src.join("..").join("..");
    assert!(e.check_authorized(&outside, false).is_err());
    assert!(e
        .check_authorized(&fx.src.join("..").join("backup"), true)
        .is_ok());
}

#[test]
fn revoked_grant_blocks_execution() {
    let fx = Fx::new();
    fx.write_src("a.mov", b"x");
    let e = fx.engine();
    let id = plan_and_create(&e, &request(&fx, mirror(), vec![fx.dst.clone()]));
    let g = e
        .grants()
        .unwrap()
        .into_iter()
        .find(|g| g.path == fx.dst.to_string_lossy())
        .unwrap();
    e.revoke(&g.id).unwrap();
    assert!(matches!(
        run(&e, &id, &Control::new(), None),
        Err(MfError::NotAuthorized(_))
    ));
    assert!(all_files(&fx.dst).is_empty());
}

#[test]
fn forbidden_destinations() {
    let fx = Fx::new();
    let e = fx.engine();
    assert!(e
        .authorize(std::path::Path::new("/"), GrantKind::Destination)
        .is_err());
    #[cfg(unix)]
    assert!(e
        .authorize(std::path::Path::new("/etc"), GrantKind::Destination)
        .is_err());
}

#[test]
fn source_and_destination_must_not_nest() {
    let fx = Fx::new();
    fx.write_src("a.mov", b"x");
    let e = fx.engine();
    let inner = fx.src.join("inner");
    std::fs::create_dir(&inner).unwrap();
    e.authorize(&inner, GrantKind::Destination).unwrap();
    let req = request(&fx, mirror(), vec![inner]);
    assert!(matches!(
        e.plan(&req, &Control::new(), |_, _, _| {}),
        Err(MfError::PathRejected { .. })
    ));
}

#[cfg(unix)]
#[test]
fn symlinks_in_source_are_reported_not_followed() {
    let fx = Fx::new();
    fx.write_src("a.mov", b"x");
    let secret = fx.tmp.path().join("secret.txt");
    std::fs::write(&secret, b"top secret").unwrap();
    std::os::unix::fs::symlink(&secret, fx.src.join("link.mov")).unwrap();
    std::os::unix::fs::symlink(fx.tmp.path(), fx.src.join("dirlink")).unwrap();
    let e = fx.engine();
    let plan = e
        .plan(
            &request(&fx, mirror(), vec![fx.dst.clone()]),
            &Control::new(),
            |_, _, _| {},
        )
        .unwrap();
    assert_eq!(plan.items.len(), 1);
    assert_eq!(plan.scan_issues.len(), 2);
    let id = e.create_session(&plan).unwrap();
    assert_eq!(
        run(&e, &id, &Control::new(), None).unwrap(),
        SessionState::Completed
    );
    assert_eq!(all_files(&fx.dst), vec!["a.mov"]);
    let r = e.report(&id).unwrap();
    assert!(r.limitations.iter().any(|l| l.contains("symbolic links")));
}

#[cfg(unix)]
#[test]
fn symlink_planted_in_destination_after_planning() {
    // Classic TOCTOU: after the plan, an attacker replaces a destination
    // folder with a symlink to somewhere else.
    let fx = Fx::new();
    fx.write_src("sub/a.mov", b"payload");
    std::fs::create_dir(fx.dst.join("sub")).unwrap();
    let e = fx.engine();
    let id = plan_and_create(&e, &request(&fx, mirror(), vec![fx.dst.clone()]));
    let elsewhere = fx.tmp.path().join("elsewhere");
    std::fs::create_dir(&elsewhere).unwrap();
    std::fs::remove_dir(fx.dst.join("sub")).unwrap();
    std::os::unix::fs::symlink(&elsewhere, fx.dst.join("sub")).unwrap();
    assert_eq!(
        run(&e, &id, &Control::new(), None).unwrap(),
        SessionState::CompletedWithErrors
    );
    let (_, ops) = e.session_ops(&id).unwrap();
    assert_eq!(ops[0].state, mediaflow_core::state::OpState::Failed);
    assert_eq!(ops[0].error_code.as_deref(), Some("path_rejected"));
    assert!(
        std::fs::read_dir(&elsewhere).unwrap().next().is_none(),
        "nothing written through the symlink"
    );
}

#[cfg(unix)]
#[test]
fn source_file_swapped_for_symlink_after_planning() {
    let fx = Fx::new();
    let p = fx.write_src("a.mov", b"original");
    let secret = fx.tmp.path().join("secret");
    std::fs::write(&secret, b"secret").unwrap();
    let e = fx.engine();
    let id = plan_and_create(&e, &request(&fx, mirror(), vec![fx.dst.clone()]));
    std::fs::remove_file(&p).unwrap();
    std::os::unix::fs::symlink(&secret, &p).unwrap();
    assert_eq!(
        run(&e, &id, &Control::new(), None).unwrap(),
        SessionState::CompletedWithErrors
    );
    assert!(all_files(&fx.dst).is_empty());
}

#[test]
fn target_created_between_plan_and_execution_is_not_overwritten() {
    let fx = Fx::new();
    fx.write_src("a.mov", &data(1, 100));
    let e = fx.engine();
    let id = plan_and_create(&e, &request(&fx, mirror(), vec![fx.dst.clone()]));
    write(&fx.dst, "a.mov", b"someone else's file");
    assert_eq!(
        run(&e, &id, &Control::new(), None).unwrap(),
        SessionState::Completed
    );
    assert_eq!(
        std::fs::read(fx.dst.join("a.mov")).unwrap(),
        b"someone else's file"
    );
    assert_eq!(
        std::fs::read(fx.dst.join("a (2).mov")).unwrap(),
        data(1, 100)
    );
    assert!(e
        .events(&id)
        .unwrap()
        .iter()
        .any(|x| x.code == "renamed_at_execution"));
}

#[test]
fn target_created_during_copy_is_not_overwritten() {
    let fx = Fx::new();
    fx.write_src("a.mov", &data(1, 100));
    let e = fx.engine();
    let id = plan_and_create(&e, &request(&fx, mirror(), vec![fx.dst.clone()]));
    let dst = fx.dst.clone();
    let hooks = Hooks {
        before_finalize: Some(Box::new(move |_, _| {
            std::fs::write(dst.join("a.mov"), b"raced").unwrap();
            Ok(())
        })),
        ..Default::default()
    };
    assert_eq!(
        run(&e, &id, &Control::new(), Some(Arc::new(hooks))).unwrap(),
        SessionState::Completed
    );
    assert_eq!(std::fs::read(fx.dst.join("a.mov")).unwrap(), b"raced");
    assert_eq!(
        std::fs::read(fx.dst.join("a (2).mov")).unwrap(),
        data(1, 100)
    );
    let r = e.report(&id).unwrap();
    assert_eq!(r.files[0].target_rel.as_deref(), Some("a (2).mov"));
}

#[test]
fn hostile_file_names_from_card() {
    // Names that are legal on Linux but dangerous or invalid elsewhere are
    // sanitized by the template, never used to escape the destination.
    let fx = Fx::new();
    if cfg!(unix) {
        fx.write_src("weird:name?.mov", b"1");
        fx.write_src("=cmd|' calc'!A0.mov", b"2");
    }
    fx.write_src("CON.mov", b"3");
    let e = fx.engine();
    let mut p = mirror();
    p.template.folders = "{project}".into();
    let id = plan_and_create(&e, &request(&fx, p, vec![fx.dst.clone()]));
    assert_eq!(
        run(&e, &id, &Control::new(), None).unwrap(),
        SessionState::Completed
    );
    let files = all_files(&fx.dst);
    assert!(files.iter().all(|f| f.starts_with("Test/")), "{files:?}");
    assert!(
        files
            .iter()
            .all(|f| !f.contains(':') && !f.contains('?') && !f.contains('|')),
        "{files:?}"
    );
    let csv = e.report(&id).unwrap().to_csv().unwrap();
    assert!(
        !csv.lines().any(|l| l.starts_with('=')),
        "CSV formula injection: {csv}"
    );
}

#[test]
fn report_export_never_overwrites() {
    let fx = Fx::new();
    fx.write_src("a.mov", b"x");
    let e = fx.engine();
    let id = plan_and_create(&e, &request(&fx, mirror(), vec![fx.dst.clone()]));
    run(&e, &id, &Control::new(), None).unwrap();
    let r = e.report(&id).unwrap();
    let out = fx.tmp.path().join("report.json");
    r.write(&out, mediaflow_core::report::ReportFormat::Json)
        .unwrap();
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap();
    assert_eq!(v["state"], "completed");
    assert_eq!(v["totals"]["copied_and_verified"], 1);
    assert!(r
        .write(&out, mediaflow_core::report::ReportFormat::Csv)
        .is_err());
}

#[test]
fn detected_card_ids_only_grant_read_access_to_that_card() {
    use mediaflow_core::volumes::{DetectionRegistry, Volume};
    let fx = Fx::new();
    fx.write_src("DCIM/a.jpg", b"x");
    let e = Engine::open(&fx.db).unwrap();
    let card = Volume {
        name: "CARD".into(),
        mount_point: fx.src.to_string_lossy().into_owned(),
        file_system: "exfat".into(),
        total_bytes: 1,
        available_bytes: 0,
        removable: true,
        read_only: false,
    };
    let mut reg = DetectionRegistry::new(&[]);
    let id = reg.update(&[card])[0].id.clone();

    // Anything that is not an id minted by the registry is refused,
    // including the card's own path.
    for bogus in ["", "CARD", fx.src.to_str().unwrap(), "../../etc"] {
        assert!(
            matches!(e.authorize_detected(&reg, bogus), Err(MfError::NotFound(_))),
            "{bogus:?}"
        );
    }
    assert!(e.grants().unwrap().is_empty());

    let g = e.authorize_detected(&reg, &id).unwrap();
    assert_eq!(g.kind, GrantKind::Source);
    // A source grant never allows writing there.
    assert!(matches!(
        e.check_authorized(&fx.src, true),
        Err(MfError::NotAuthorized(_))
    ));

    // Once unplugged, the id stops working.
    reg.update(&[]);
    assert!(e.authorize_detected(&reg, &id).is_err());
}
