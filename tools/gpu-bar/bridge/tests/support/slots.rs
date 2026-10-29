use super::*;
use std::cell::Cell;
use std::rc::Rc;

struct Frame(Rc<Cell<u32>>);
impl Drop for Frame {
    fn drop(&mut self) {
        self.0.set(self.0.get() + 1);
    }
}

fn add(slots: &mut Slots<Frame>, generation: u64, drops: &Rc<Cell<u32>>) -> u64 {
    let (index, token) = slots.prepare(generation).unwrap();
    slots.commit(index, token, generation, Frame(drops.clone()));
    token
}

#[test]
fn two_frames_wait_for_exact_release_and_never_reuse_a_token() {
    let drops = Rc::new(Cell::new(0));
    let mut slots = Slots::new();
    assert!(slots.prepare(0).is_err());
    assert!(slots.prepare(2).is_err());
    let first = add(&mut slots, 1, &drops);
    assert!(slots.prepare(1).is_err());
    let second = add(&mut slots, 2, &drops);
    assert!(slots.prepare(3).is_err());
    assert!(slots.can_close().is_err());
    assert!(slots.release(0).is_err());
    let mut other = Slots::new();
    let foreign = add(&mut other, 1, &drops);
    assert!(slots.release(foreign).is_err());
    assert_eq!(drops.get(), 0);
    drop(slots.release(second).unwrap()); // Out-of-order release is valid.
    assert_eq!(drops.get(), 1);
    assert!(slots.release(second).is_err());
    let third = add(&mut slots, 3, &drops);
    assert_ne!(third, second);
    assert!(slots.release(second).is_err());
    assert!(slots.can_close().is_err());
    drop(slots.release(first).unwrap());
    drop(slots.release(third).unwrap());
    assert!(slots.can_close().is_ok());
    assert_eq!(drops.get(), 3);
    drop(other.release(foreign).unwrap());
}

#[test]
fn a_renderer_failure_retains_custody_and_token_exhaustion_never_wraps() {
    let drops = Rc::new(Cell::new(0));
    let mut slots = Slots::new();
    let held = add(&mut slots, 1, &drops);
    slots.poisoned = true;
    assert!(slots.prepare(2).is_err());
    assert!(slots.can_close().is_err());
    assert_eq!(drops.get(), 0);
    drop(slots.release(held).unwrap());
    assert!(slots.can_close().is_err());
    let counter = AtomicU64::new(u64::MAX - 1);
    assert_eq!(next_token(&counter).unwrap(), u64::MAX - 1);
    assert!(next_token(&counter).is_err());
    assert!(next_token(&counter).is_err());
    assert_eq!(counter.load(Ordering::Relaxed), u64::MAX);
}
