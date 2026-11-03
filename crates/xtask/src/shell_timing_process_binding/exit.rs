//! Resolve an ENOENT executable without trusting the binding again.
use super::{Binder, BindingRecheck, ProcessIdentity, StatFacts, ended};

impl Binder {
    pub(super) fn settle_exit(&self, identity: ProcessIdentity) -> Result<BindingRecheck, String> {
        // exe may disappear before stat reports exit; never re-establish Bound here.
        settle(
            identity,
            (0..8).map(|attempt| {
                if attempt != 0 {
                    std::thread::sleep(std::time::Duration::from_micros(250));
                }
                self.stat(identity.pid)
            }),
        )
    }
}

fn settle(
    identity: ProcessIdentity,
    observations: impl IntoIterator<Item = Result<Option<StatFacts>, String>>,
) -> Result<BindingRecheck, String> {
    for observation in observations {
        let Some(facts) = observation? else {
            return Ok(BindingRecheck::Ended);
        };
        if facts.identity != identity {
            return Err(format!(
                "pid {} was reused: start_time {} replaced {}",
                identity.pid, facts.identity.start_time_ticks, identity.start_time_ticks
            ));
        }
        if ended(facts.state) {
            return Ok(BindingRecheck::Ended);
        }
    }
    Err(format!(
        "pid {}: executable missing while still live after bounded observations",
        identity.pid
    ))
}

#[cfg(test)]
#[path = "../../tests/support/binding_exit_settle.rs"]
mod tests;
