//! The product preparer refuses unknown products and ambiguous inputs before
//! anything is read or built.
use std::path::Path;
use xtask::product_artifact::{PRODUCTS, product, run};

fn args(values: &[&str]) -> Vec<String> {
    values.iter().map(|v| (*v).to_owned()).collect()
}

#[test]
fn every_product_has_one_recipe() {
    let names = PRODUCTS.iter().map(|p| p.name).collect::<Vec<_>>();
    assert_eq!(names, ["lom", "provlita", "hagia", "narthex"]);
    assert_eq!(
        product("lom").unwrap().config,
        Some("examples/minimal/live-shell.kdl")
    );
    assert_eq!(product("hagia").unwrap().config, None);
    assert!(
        product("bemenu").is_err(),
        "Bemenu has its own SDK-pinned preparer"
    );
}

#[test]
fn unknown_product_ambiguous_revision_and_existing_destination_are_refused() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).display().to_string();
    assert!(run(&[]).unwrap_err().contains("usage"));
    // Narthex ships only with Hagia, as one pair.
    assert!(
        run(&args(&["narthex", &root, &"0".repeat(40), "unused"]))
            .unwrap_err()
            .contains("prepare-wm-pair")
    );
    let zero = "0".repeat(40);
    assert!(
        run(&args(&["firefox", &root, &zero, "unused"]))
            .unwrap_err()
            .contains("unknown product")
    );
    assert!(
        run(&args(&["lom", &root, "HEAD", "unused"]))
            .unwrap_err()
            .contains("40 lowercase hex")
    );
    assert!(
        run(&args(&["hagia", &root, &zero, &root]))
            .unwrap_err()
            .contains("already exists")
    );
}

/// Builds default to every CPU and honor a caller's positive
/// `CARGO_BUILD_JOBS`; anything else is refused rather than recorded in an
/// argv that did not run.
#[test]
fn build_jobs_default_to_every_cpu_and_honor_a_positive_override() {
    use std::ffi::OsStr;
    use xtask::product_artifact::jobs;
    assert_eq!(jobs(None, 12), Ok(12));
    assert_eq!(jobs(None, 0), Ok(1));
    assert_eq!(jobs(Some(OsStr::new("3")), 12), Ok(3));
    assert_eq!(jobs(Some(OsStr::new("64")), 2), Ok(64));
    for refused in ["0", "01", "-1", "+2", "", "two", "1.5", " 2"] {
        assert!(jobs(Some(OsStr::new(refused)), 12).is_err(), "{refused:?}");
    }
}
