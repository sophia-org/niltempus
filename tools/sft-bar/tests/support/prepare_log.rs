use std::io::{self, Write};

const HOST_LINE: &[u8] = b"sophia_live_cadence_sample schema=1 uptime_msec=5000\n";

#[derive(Default)]
struct SharedLog {
    bytes: Vec<u8>,
    writes: usize,
}

impl Write for SharedLog {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.writes += 1;
        self.bytes.extend_from_slice(bytes);
        self.bytes.extend_from_slice(HOST_LINE);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn another_writer_cannot_split_a_preparation_record_between_format_arguments() {
    for (generation, resource, start, end) in [
        (266, 2, 420673477023, 420673477053),
        (0, 0, 0, 0),
        (u64::MAX, u64::MAX, u64::MAX, u64::MAX),
    ] {
        let mut log = SharedLog::default();
        super::write(&mut log, generation, resource, start, end).unwrap();
        let expected = format!(
            "gpu_bar_prepare schema=1 arm=owned_ring generation={generation} resource_id={resource} start_usec={start} end_usec={end}\n"
        );
        assert_eq!(log.writes, 1, "one complete receipt must reach the writer");
        assert_eq!(log.bytes, [expected.as_bytes(), HOST_LINE].concat());
    }
}

#[test]
fn receipt_bytes_and_write_errors_are_preserved() {
    let expected = b"gpu_bar_prepare schema=1 arm=owned_ring generation=1 resource_id=2 start_usec=3 end_usec=4\n";
    let mut bytes = [0; 256];
    let mut destination = &mut bytes[..expected.len()];
    super::write(&mut destination, 1, 2, 3, 4).unwrap();
    assert_eq!(&bytes[..expected.len()], expected);
    let mut too_short = &mut bytes[..expected.len() - 1];
    assert_eq!(
        super::write(&mut too_short, 1, 2, 3, 4).unwrap_err().kind(),
        io::ErrorKind::WriteZero
    );
}
