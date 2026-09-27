//! The verdict: every error examined, exact stage tokens, overflow, failure,
//! exit status and the deadline stop.
use quickshell_probe::{ClientEnd, Observed, ObservedError, Renderer, allowed, evaluate};

fn error(code: &str, major: u8, minor: u16, resource: u32) -> ObservedError {
    ObservedError {
        code: code.to_owned(),
        major,
        minor,
        resource,
        sequence: 7,
    }
}

fn present_unselect() -> ObservedError {
    error("BadWindow", 138, 3, 0x0020_0004)
}

fn connected(renderer: Renderer) -> Observed {
    Observed {
        requests: 329,
        connections: 1,
        stages: renderer
            .required_stages()
            .iter()
            .map(|stage| (*stage).to_owned())
            .collect(),
        ..Observed::default()
    }
}

#[test]
fn a_clean_trace_stopped_at_the_deadline_is_accepted() {
    for renderer in [Renderer::Gpu, Renderer::Software] {
        let line = evaluate(renderer, &connected(renderer), ClientEnd::StoppedAtDeadline)
            .unwrap_or_else(|refusal| panic!("{refusal}"));
        assert!(line.ends_with("status=accepted"), "{line}");
        assert!(line.contains("end=stopped_at_deadline"), "{line}");
    }
    for end in [
        ClientEnd::StoppedAfterTransaction,
        ClientEnd::Exited {
            code: Some(0),
            signal: None,
        },
    ] {
        evaluate(Renderer::Software, &connected(Renderer::Software), end).unwrap();
    }
}

#[test]
fn an_allowed_error_does_not_hide_a_later_one() {
    // The old harness kept only the first error: an allowed BadWindow first
    // hid anything after it.
    let mut observed = connected(Renderer::Gpu);
    observed.errors = vec![present_unselect(), error("BadMatch", 130, 2, 0x0020_0009)];
    let refusal = evaluate(Renderer::Gpu, &observed, ClientEnd::StoppedAtDeadline).unwrap_err();
    assert!(
        refusal.contains("x_error=BadMatch:major=130:minor=2"),
        "{refusal}"
    );
    assert!(!refusal.contains("x_error=BadWindow"), "{refusal}");
    // Every disallowed error is named, not only the first.
    observed.errors = vec![
        error("BadAccess", 2, 0, 1),
        present_unselect(),
        error("BadValue", 12, 0, 2),
    ];
    let refusal = evaluate(Renderer::Gpu, &observed, ClientEnd::StoppedAtDeadline).unwrap_err();
    assert!(refusal.contains("x_error=BadAccess"), "{refusal}");
    assert!(refusal.contains("x_error=BadValue"), "{refusal}");
    // Only the allowed one: accepted, and counted.
    observed.errors = vec![present_unselect(), present_unselect()];
    let line = evaluate(Renderer::Gpu, &observed, ClientEnd::StoppedAtDeadline).unwrap();
    assert!(line.contains("allowed_errors=2"), "{line}");
}

#[test]
fn only_the_two_carried_shapes_are_allowed() {
    for renderer in [Renderer::Gpu, Renderer::Software] {
        assert!(allowed(renderer, &error("BadWindow", 3, 0, 0)));
        assert!(allowed(renderer, &error("BadWindow", 14, 0, 0)));
        for other in [
            error("BadWindow", 3, 0, 1),
            error("BadWindow", 14, 1, 0),
            error("BadWindow", 4, 0, 0),
            error("BadDrawable", 14, 0, 0),
            error("BadWindow", 138, 2, 0x0020_0004),
            error("BadMatch", 138, 3, 0x0020_0004),
        ] {
            assert!(!allowed(renderer, &other), "{other:?}");
        }
    }
    // The Present unselect after destroy is the GPU path's; the software
    // variant never had that tolerance.
    assert!(allowed(Renderer::Gpu, &present_unselect()));
    assert!(!allowed(Renderer::Software, &present_unselect()));
    let mut observed = connected(Renderer::Software);
    observed.errors = vec![present_unselect()];
    assert!(evaluate(Renderer::Software, &observed, ClientEnd::StoppedAtDeadline).is_err());
}

#[test]
fn stage_tokens_are_compared_exactly() {
    for stages in [
        vec![],
        vec!["RENDER:QueryPictFormatsX"],
        vec!["XRENDER:QueryPictFormats"],
        vec!["RENDER:Composite", "GLX:QueryServerString"],
    ] {
        let mut observed = connected(Renderer::Gpu);
        observed.stages = stages.iter().map(|stage| (*stage).to_owned()).collect();
        let refusal = evaluate(Renderer::Gpu, &observed, ClientEnd::StoppedAtDeadline).unwrap_err();
        assert!(
            refusal.contains("missing_stage=RENDER:QueryPictFormats"),
            "{stages:?}: {refusal}"
        );
    }
    assert!(Renderer::Software.required_stages().is_empty());
}

#[test]
fn overflow_failure_and_harness_faults_are_refused() {
    let base = connected(Renderer::Software);
    let cases: [(&str, Observed); 4] = [
        (
            "trace_overflow",
            Observed {
                overflowed: true,
                ..base.clone()
            },
        ),
        (
            "dispatch_failure=ParseRejected",
            Observed {
                failures: vec!["ParseRejected:major=200:minor=0".to_owned()],
                ..base.clone()
            },
        ),
        (
            "server_error=",
            Observed {
                server_error: Some("accept failed".to_owned()),
                ..base.clone()
            },
        ),
        (
            "client_log_exceeded",
            Observed {
                client_log_exceeded: true,
                ..base.clone()
            },
        ),
    ];
    for (reason, observed) in cases {
        let refusal =
            evaluate(Renderer::Software, &observed, ClientEnd::StoppedAtDeadline).unwrap_err();
        assert!(refusal.contains(reason), "{reason}: {refusal}");
    }
}

#[test]
fn a_failed_client_or_one_that_never_connected_is_refused() {
    let observed = connected(Renderer::Software);
    for (end, reason) in [
        (
            ClientEnd::Exited {
                code: Some(1),
                signal: None,
            },
            "client_failed=exited:1",
        ),
        (
            ClientEnd::Exited {
                code: None,
                signal: Some(11),
            },
            "client_failed=signalled:11",
        ),
    ] {
        let refusal = evaluate(Renderer::Software, &observed, end).unwrap_err();
        assert!(refusal.contains(reason), "{refusal}");
    }
    for never in [
        Observed::default(),
        Observed {
            connections: 1,
            ..Observed::default()
        },
    ] {
        let refusal = evaluate(
            Renderer::Software,
            &never,
            ClientEnd::Exited {
                code: Some(0),
                signal: None,
            },
        )
        .unwrap_err();
        assert!(refusal.contains("client_never_connected"), "{refusal}");
    }
}
