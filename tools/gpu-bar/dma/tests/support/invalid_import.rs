use super::*;
use rustix::fs::{SeekFrom, fcntl_get_seals, seek};

#[test]
fn only_one_unprofiled_dma_frame_is_admitted() {
    let text =
        r#"{"wire":"files","output":1,"frames":1,"height":24,"hold_msec":0,"source":"dma_buf"}"#;
    let parse = super::super::parse_config;
    validate_config(&parse(text).unwrap()).unwrap();
    for bad in [
        text.replace("\"frames\":1", "\"frames\":3"),
        text.replace("\"height\":24", "\"height\":512"),
        text.replace("\"hold_msec\":0", "\"hold_msec\":1"),
        text.replace(
            "\"source\":\"dma_buf\"",
            "\"source\":\"owned_ring\",\"ring_slots\":2",
        ),
    ] {
        assert!(validate_config(&parse(&bad).unwrap()).is_err());
    }
    let mut config = parse(text).unwrap();
    config.capture_profile = true;
    assert!(validate_config(&config).is_err());
    config.capture_profile = false;
    config.dma_reuse = true;
    assert!(validate_config(&config).is_err());
    config.dma_reuse = false;
    config.producer = Producer::Cpu;
    assert!(validate_config(&config).is_err());
}

#[test]
fn replacement_is_bounded_non_dma_and_immutable() {
    for bytes in [184320, 196608] {
        let fd = replacement(bytes).unwrap();
        assert_eq!(seek(&fd, SeekFrom::End(0)).unwrap(), bytes);
        assert_eq!(
            fcntl_get_seals(&fd).unwrap(),
            SealFlags::GROW | SealFlags::SHRINK | SealFlags::WRITE | SealFlags::SEAL
        );
        assert_eq!(
            rustix::io::pwrite(&fd, b"x", 0),
            Err(rustix::io::Errno::PERM)
        );
        assert!(ftruncate(&fd, bytes + 1).is_err());
        assert!(ftruncate(&fd, bytes - 1).is_err());
        let mut bytes = [1; 4];
        assert_eq!(rustix::io::pread(&fd, &mut bytes, 0).unwrap(), 4);
        assert_eq!(bytes, [0; 4]);
    }
    for bytes in [0, 184319, u64::MAX] {
        assert!(replacement(bytes).is_err());
    }
}

#[test]
fn only_the_matching_remote_import_error_qualifies() {
    let grant = ContentGrant {
        connection_epoch: 1,
        content_grant_epoch: 2,
    };
    let resource = ContentResourceId {
        id: 3,
        generation: 4,
    };
    let status = ContentResourceStatus {
        grant,
        resource,
        status: 3,
        reason: ContentReason::RendererFailed as u16,
        next_ordinal: 0,
        admitted_bytes: 0,
    };
    for reason in [ContentReason::Malformed, ContentReason::RendererFailed] {
        let mut value = status.clone();
        value.reason = reason as u16;
        refusal(ShellContentRecord::ResourceStatus(value), grant, resource).unwrap();
    }
    for which in 0..10 {
        let mut value = status.clone();
        match which {
            0 => value.grant.connection_epoch += 1,
            1 => value.grant.content_grant_epoch += 1,
            2 => value.resource.id += 1,
            3 => value.resource.generation += 1,
            4 => value.status = 2,
            5 => value.status = 4,
            6 => value.reason = ContentReason::Timeout as u16,
            7 => value.reason = ContentReason::Budget as u16,
            8 => value.next_ordinal = 1,
            9 => value.admitted_bytes = 184320,
            _ => unreachable!(),
        }
        assert!(refusal(ShellContentRecord::ResourceStatus(value), grant, resource).is_err());
    }
}
