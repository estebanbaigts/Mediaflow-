//! Completeness check tests.

mod common;

use common::*;
use mediaflow_core::compare::{CompareMode, Presence};
use mediaflow_core::profile::builtin;
use mediaflow_core::Control;

#[test]
fn verified_completeness_after_reorganized_import() {
    let fx = Fx::new();
    fx.write_src("DCIM/C0001.MP4", &data(1, 5000));
    fx.write_src("DCIM/C0002.MP4", &data(2, 5000));
    let e = fx.engine();
    // Default layout reorganizes files: content matching must still find them.
    let id = plan_and_create(
        &e,
        &request(&fx, builtin("youtube").unwrap(), vec![fx.dst.clone()]),
    );
    run(&e, &id, &Control::new(), None).unwrap();

    let r = e
        .compare(
            &fx.src,
            &[fx.dst.clone(), fx.dst2.clone()],
            CompareMode::Verified,
            &Control::new(),
            |_, _| {},
        )
        .unwrap();
    assert!(r.destinations[0].complete, "{:?}", r.destinations[0]);
    assert_eq!(r.destinations[0].present_verified, 2);
    assert!(!r.destinations[1].complete);
    assert_eq!(r.destinations[1].missing, 2);
    assert_eq!(r.everywhere, 0);
}

#[test]
fn quick_mode_never_claims_complete() {
    let fx = Fx::new();
    fx.write_src("a.mov", &data(1, 100));
    write(&fx.dst, "a.mov", &data(2, 100)); // same name & size, different bytes
    let e = fx.engine();
    let r = e
        .compare(
            &fx.src,
            std::slice::from_ref(&fx.dst),
            CompareMode::Quick,
            &Control::new(),
            |_, _| {},
        )
        .unwrap();
    assert!(matches!(
        r.entries[0].per_destination[0],
        Presence::PresentUnverified { .. }
    ));
    assert!(!r.destinations[0].complete);
    let r = e
        .compare(
            &fx.src,
            std::slice::from_ref(&fx.dst),
            CompareMode::Verified,
            &Control::new(),
            |_, _| {},
        )
        .unwrap();
    assert!(matches!(
        r.entries[0].per_destination[0],
        Presence::Modified { .. }
    ));
    assert!(!r.destinations[0].complete);
    assert!(r.previous_check_at.is_some());
}

#[test]
fn disconnected_destination_is_not_missing() {
    let fx = Fx::new();
    fx.write_src("a.mov", &data(1, 100));
    let e = fx.engine();
    let gone = fx.tmp.path().join("unplugged");
    let r = e
        .compare(
            &fx.src,
            &[gone],
            CompareMode::Verified,
            &Control::new(),
            |_, _| {},
        )
        .unwrap();
    assert_eq!(
        r.entries[0].per_destination[0],
        Presence::DestinationUnavailable
    );
    assert!(!r.destinations[0].available);
    assert!(!r.destinations[0].complete);
    assert_eq!(r.destinations[0].missing, 0);
}

#[test]
fn corrupted_copy_detected() {
    let fx = Fx::new();
    fx.write_src("a.mov", &data(1, 1000));
    let e = fx.engine();
    let id = plan_and_create(&e, &request(&fx, mirror(), vec![fx.dst.clone()]));
    run(&e, &id, &Control::new(), None).unwrap();
    // Bit rot on the backup (test fixture).
    let p = fx.dst.join("a.mov");
    let mut b = std::fs::read(&p).unwrap();
    b[3] ^= 0x10;
    std::fs::write(&p, b).unwrap();
    let r = e
        .compare(
            &fx.src,
            std::slice::from_ref(&fx.dst),
            CompareMode::Verified,
            &Control::new(),
            |_, _| {},
        )
        .unwrap();
    assert!(
        matches!(r.entries[0].per_destination[0], Presence::Corrupted { .. }),
        "{:?}",
        r.entries[0]
    );
    assert_eq!(r.destinations[0].corrupted, 1);
}
