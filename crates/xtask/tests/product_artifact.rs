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
    assert_eq!(names, ["lom", "provlita", "hagia"]);
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
