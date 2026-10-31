use super::*;
use std::cell::Cell;
use std::rc::Rc;
use std::time::Instant;

struct Stamp {
    start: Instant,
    dropped: Rc<Cell<Option<Duration>>>,
}
impl Drop for Stamp {
    fn drop(&mut self) {
        self.dropped.set(Some(self.start.elapsed()));
    }
}

#[test]
fn export_drop_follows_the_delay_not_the_release_observation() {
    let dropped = Rc::new(Cell::new(None));
    release(
        Stamp {
            start: Instant::now(),
            dropped: dropped.clone(),
        },
        ContentGrant {
            connection_epoch: 2,
            content_grant_epoch: 3,
        },
        ContentResourceId {
            id: 1,
            generation: 1,
        },
        1,
    )
    .unwrap();
    assert!(dropped.get().unwrap() >= Duration::from_millis(100));
}

#[test]
fn bad_identity_does_not_drop_the_export() {
    for which in 0..5 {
        let dropped = Rc::new(Cell::new(None));
        let mut grant = ContentGrant {
            connection_epoch: 2,
            content_grant_epoch: 3,
        };
        let mut resource = ContentResourceId {
            id: 1,
            generation: 1,
        };
        let mut offer = 1;
        match which {
            0 => grant.connection_epoch = 0,
            1 => grant.content_grant_epoch = 0,
            2 => resource.id = 2,
            3 => resource.generation = 2,
            4 => offer = 0,
            _ => unreachable!(),
        }
        assert!(
            release(
                Stamp {
                    start: Instant::now(),
                    dropped: dropped.clone()
                },
                grant,
                resource,
                offer
            )
            .is_err()
        );
        assert!(dropped.get().is_none());
    }
}

#[test]
fn only_one_unprofiled_dma_frame_is_admitted() {
    let text =
        r#"{"wire":"files","output":1,"frames":1,"height":24,"hold_msec":0,"source":"dma_buf"}"#;
    let parse = super::super::parse_config;
    validate_config(&parse(text).unwrap()).unwrap();
    for which in 0..7 {
        let mut config = parse(text).unwrap();
        match which {
            0 => config.frames = 2,
            1 => config.height = 512,
            2 => config.hold_msec = 1,
            3 => config.capture_profile = true,
            4 => config.dma_reuse = true,
            5 => config.source = Source::OwnedRing { slots: 2 },
            6 => config.producer = Producer::Cpu,
            _ => unreachable!(),
        }
        assert!(validate_config(&config).is_err());
    }
}
