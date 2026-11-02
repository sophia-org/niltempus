//! Explicit diagnostic opt-ins through the native launcher's cleared environment.
use std::ffi::OsString;

pub(super) fn selected(
    value: impl Fn(&str) -> Option<OsString>,
) -> Vec<(&'static str, &'static str)> {
    ["SOPHIA_OWNER_PASS_COST", "SOPHIA_SHELL_WORK_STATS"]
        .into_iter()
        .filter(|key| value(key).is_some_and(|value| value == "1"))
        .map(|key| (key, "1"))
        .collect()
}

#[cfg(test)]
#[path = "../../tests/support/shell_timing_cost_environment.rs"]
mod tests;
