//! Status snapshot used by the menu bar and the macOS widget.

mod common;

use common::*;
use mediaflow_core::executor::Fault;
use mediaflow_core::state::SessionState;
use mediaflow_core::Control;
use std::sync::Arc;

#[test]
fn snapshot_reports_last_import_and_attention() {
    let fx = Fx::new();
    fx.write_src("a.mov", &data(1, 1000));
    let e = fx.engine();
    let id = plan_and_create(&e, &request(&fx, mirror(), vec![fx.dst.clone()]));
    assert_eq!(
        run(&e, &id, &Control::new(), None).unwrap(),
        SessionState::Completed
    );

    let s = e.status_snapshot(None).unwrap();
    let last = s.last.unwrap();
    assert!(last.verified_ok);
    assert_eq!(last.files_done, 1);
    assert_eq!(last.problems, 0);
    assert_eq!(s.attention, 0);
    assert!(s
        .destinations
        .iter()
        .any(|d| d.label == "backup" && d.available));

    // A second import that fails on its only file needs attention and is
    // never reported as verified.
    fx.write_src("b.mov", &data(2, 1000));
    let id2 = plan_and_create(&e, &request(&fx, mirror(), vec![fx.dst.clone()]));
    let hooks = Hooks {
        after_write: Some(Box::new(|_, _, _| {
            Err(Fault::Io(std::io::Error::from(
                std::io::ErrorKind::StorageFull,
            )))
        })),
        ..Default::default()
    };
    assert_eq!(
        run(&e, &id2, &Control::new(), Some(Arc::new(hooks))).unwrap(),
        SessionState::CompletedWithErrors
    );
    let s = e.status_snapshot(None).unwrap();
    let last = s.last.unwrap();
    assert_eq!(last.session_id, id2);
    assert!(!last.verified_ok);
    assert!(last.problems >= 1);
    assert_eq!(s.attention, 1);
}
