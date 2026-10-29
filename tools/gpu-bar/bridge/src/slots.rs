//! Two retained frames; generation order is independent of consumer release order.
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TOKEN: AtomicU64 = AtomicU64::new(1);

pub(crate) struct Slots<T> {
    entries: [Option<(u64, T)>; 2],
    generation: u64,
    pub(crate) poisoned: bool,
}

impl<T> Slots<T> {
    pub(crate) fn new() -> Self {
        Self {
            entries: [None, None],
            generation: 0,
            poisoned: false,
        }
    }

    pub(crate) fn prepare(&self, generation: u64) -> Result<(usize, u64), String> {
        if self.poisoned {
            return Err("renderer failed; terminate the run".into());
        }
        if self.generation.checked_add(1) != Some(generation) {
            return Err("generation must start at one and advance exactly once".into());
        }
        let index = self
            .entries
            .iter()
            .position(Option::is_none)
            .ok_or("two exported frames are still retained")?;
        Ok((index, next_token(&NEXT_TOKEN)?))
    }

    pub(crate) fn commit(&mut self, index: usize, token: u64, generation: u64, frame: T) {
        assert!(self.entries[index].is_none());
        self.entries[index] = Some((token, frame));
        self.generation = generation;
    }

    pub(crate) fn release(&mut self, token: u64) -> Result<T, String> {
        let index = self
            .entries
            .iter()
            .position(|entry| entry.as_ref().is_some_and(|(id, _)| *id == token))
            .ok_or("unknown, stale or foreign frame token")?;
        Ok(self.entries[index].take().unwrap().1)
    }

    pub(crate) fn can_close(&self) -> Result<(), String> {
        if self.poisoned || self.entries.iter().any(Option::is_some) {
            return Err(
                "renderer or consumer custody remains; retain until release or process exit".into(),
            );
        }
        Ok(())
    }
}

fn next_token(counter: &AtomicU64) -> Result<u64, String> {
    counter
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
        .map_err(|_| "frame token space exhausted".into())
}

#[cfg(test)]
#[path = "../tests/support/slots.rs"]
mod tests;
