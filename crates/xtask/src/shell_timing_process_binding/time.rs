//! The time-namespace half of a binding: host, peer and capturer must
//! read the kernel's initial time namespace with a zero monotonic offset.
use super::*;

pub(super) fn monotonic_usec() -> Result<u64, String> {
    let now = rustix::time::clock_gettime(rustix::time::ClockId::Monotonic);
    let seconds = u64::try_from(now.tv_sec).map_err(|_| "monotonic clock before boot")?;
    let nanos = u64::try_from(now.tv_nsec).map_err(|_| "monotonic clock nanoseconds")?;
    seconds
        .checked_mul(1_000_000)
        .and_then(|usec| usec.checked_add(nanos / 1000))
        .ok_or_else(|| "monotonic clock overflow".into())
}

impl Binder {
    pub(super) fn namespace_evidence(&self, stat: &StatFacts) -> Result<NamespaceEvidence, String> {
        let pid = stat.identity.pid;
        let link = self.proc_path(&format!("{pid}/ns/time"));
        let target = fs::read_link(&link).map_err(|e| format!("{}: {e}", link.display()))?;
        let text = target.to_string_lossy().into_owned();
        let inode: u64 = text
            .strip_prefix("time:[")
            .and_then(|rest| rest.strip_suffix(']'))
            .and_then(|digits| digits.parse().ok())
            .ok_or_else(|| {
                format!(
                    "{}: unexpected time namespace target {text:?}",
                    link.display()
                )
            })?;
        if inode == 0 {
            return Err(format!(
                "{}: time namespace inode must be nonzero",
                link.display()
            ));
        }
        Ok(NamespaceEvidence {
            pid,
            identity: stat.identity,
            time_namespace_inode: inode,
            origin: link.display().to_string(),
            observed_target: text,
            captured_usec: monotonic_usec()?,
        })
    }

    /// Namespace proof for a selected host and peer. The host and peer
    /// stat reads bracketing the namespace reads must still name the
    /// bound identities, so the proof covers the same processes.
    pub(super) fn time_proof(
        &self,
        host: &StatFacts,
        peer: &StatFacts,
    ) -> Result<TimeNamespaceProof, String> {
        let capturer_stat = self.live_stat(self.capturer_pid, "capturer")?;
        let host_evidence = self.namespace_evidence(host)?;
        let peer_evidence = self.namespace_evidence(peer)?;
        let capturer_evidence = self.namespace_evidence(&capturer_stat)?;
        // Closing bracket: the namespace reads above must not have
        // raced a host or peer replacement.
        let host_close = self.live_stat(host.identity.pid, "host")?;
        let peer_close = self.live_stat(peer.identity.pid, "peer")?;
        if host_close.identity != host.identity || peer_close.identity != peer.identity {
            return Err("host or peer identity changed during namespace capture".into());
        }
        let initial = INITIAL_TIME_NAMESPACE_INODE;
        let (host_inode, peer_inode, capturer_inode) = (
            host_evidence.time_namespace_inode,
            peer_evidence.time_namespace_inode,
            capturer_evidence.time_namespace_inode,
        );
        if [host_inode, peer_inode, capturer_inode] != [initial; 3] {
            return Err(format!(
                "time namespace inodes differ: initial {initial}, host {host_inode}, \
                 peer {peer_inode}, capturer {capturer_inode}"
            ));
        }
        let offsets_path = self.proc_path(&format!("{}/timens_offsets", self.capturer_pid));
        let offsets = read_bounded(&offsets_path, MAX_READ_BYTES)?;
        let mut monotonic: Option<(i64, i64)> = None;
        for line in offsets.lines() {
            let mut parts = line.split_whitespace();
            if parts.next() == Some("monotonic") {
                if monotonic.is_some() {
                    return Err("timens_offsets: duplicate monotonic line".into());
                }
                let seconds: i64 = parts
                    .next()
                    .and_then(|v| v.parse().ok())
                    .ok_or("timens_offsets: bad monotonic seconds")?;
                let nanos: i64 = parts
                    .next()
                    .and_then(|v| v.parse().ok())
                    .ok_or("timens_offsets: bad monotonic nanoseconds")?;
                if parts.next().is_some() {
                    return Err("timens_offsets: trailing fields on monotonic line".into());
                }
                monotonic = Some((seconds, nanos));
            }
        }
        let monotonic = monotonic.ok_or("timens_offsets: no monotonic line")?;
        if monotonic != (0, 0) {
            return Err(format!(
                "timens_offsets: monotonic offset must be exactly zero, got {} {}",
                monotonic.0, monotonic.1
            ));
        }
        Ok(TimeNamespaceProof {
            reference: InitialTimeNamespace { inode: initial },
            host: host_evidence,
            peer: peer_evidence,
            capturer: capturer_evidence,
            capturer_timens_offsets_origin: offsets_path.display().to_string(),
            capturer_timens_offsets: offsets,
            monotonic_offset: monotonic,
        })
    }
}

/// Semantic proof equality: every fact except per-read capture stamps.
pub(super) fn proof_matches(a: &TimeNamespaceProof, b: &TimeNamespaceProof) -> bool {
    fn evidence_matches(a: &NamespaceEvidence, b: &NamespaceEvidence) -> bool {
        a.pid == b.pid
            && a.identity == b.identity
            && a.time_namespace_inode == b.time_namespace_inode
            && a.origin == b.origin
            && a.observed_target == b.observed_target
    }
    a.reference == b.reference
        && evidence_matches(&a.host, &b.host)
        && evidence_matches(&a.peer, &b.peer)
        && evidence_matches(&a.capturer, &b.capturer)
        && a.capturer_timens_offsets == b.capturer_timens_offsets
        && a.capturer_timens_offsets_origin == b.capturer_timens_offsets_origin
        && a.monotonic_offset == b.monotonic_offset
}
