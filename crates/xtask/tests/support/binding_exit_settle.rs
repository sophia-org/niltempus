use super::*;

fn facts(state: char) -> StatFacts {
    StatFacts {
        identity: ProcessIdentity {
            pid: 42,
            start_time_ticks: 100,
        },
        parent_pid: 1,
        state,
    }
}

fn check(sequence: Vec<Result<Option<StatFacts>, String>>) -> Result<BindingRecheck, String> {
    settle(
        facts('R').identity,
        sequence.into_iter().chain(std::iter::from_fn(|| {
            panic!("pulled after the deciding observation")
        })),
    )
}

#[test]
fn missing_executable_settles_after_a_live_stat() {
    for state in ['Z', 'X', 'x'] {
        assert_eq!(
            check(vec![Ok(Some(facts('R'))), Ok(Some(facts(state)))]),
            Ok(BindingRecheck::Ended)
        );
    }
    assert_eq!(
        check(vec![Ok(Some(facts('R'))), Ok(None)]),
        Ok(BindingRecheck::Ended)
    );
}

#[test]
fn identity_and_read_errors_refuse_after_a_live_stat() {
    let mut reused = facts('Z');
    reused.identity.start_time_ticks += 1;
    assert!(
        check(vec![Ok(Some(facts('R'))), Ok(Some(reused))])
            .unwrap_err()
            .contains("was reused")
    );
    assert_eq!(
        check(vec![Ok(Some(facts('R'))), Err("stat unreadable".into())]),
        Err("stat unreadable".into())
    );
}

#[test]
fn immediate_exit_needs_no_more_observations() {
    assert_eq!(check(vec![Ok(None)]), Ok(BindingRecheck::Ended));
    assert_eq!(check(vec![Ok(Some(facts('Z')))]), Ok(BindingRecheck::Ended));
}

#[test]
fn persistent_live_missing_executable_refuses_at_the_bound() {
    let mut pulls = 0;
    let result = settle(
        facts('R').identity,
        (0..8).map(|_| {
            pulls += 1;
            Ok(Some(facts('R')))
        }),
    );
    assert!(
        result
            .unwrap_err()
            .contains("executable missing while still live")
    );
    assert_eq!(pulls, 8);
}
