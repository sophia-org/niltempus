use super::*;

#[test]
fn only_exact_named_diagnostic_opt_ins_survive() {
    for value in [
        None,
        Some(""),
        Some("0"),
        Some("true"),
        Some("01"),
        Some("1 "),
    ] {
        assert!(selected(|_| value.map(OsString::from)).is_empty());
    }
    for name in ["SOPHIA_OWNER_PASS_COST", "SOPHIA_SHELL_WORK_STATS"] {
        assert_eq!(
            selected(|key| (key == name).then(|| OsString::from("1"))),
            vec![(name, "1")]
        );
    }
    assert_eq!(
        selected(|_| Some(OsString::from("1"))),
        vec![
            ("SOPHIA_OWNER_PASS_COST", "1"),
            ("SOPHIA_SHELL_WORK_STATS", "1")
        ]
    );
}
