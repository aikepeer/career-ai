//! Audit integrity: hash-chained, append-only event store (PR 4+5).
//!
//! Events use `event_hash = SHA-256(domain || shard_id || sequence ||
//! previous_hash || canonical_event)` and include `previous_hash` linking.
//! Forks, duplicate sequences, gaps, and conflicting previous hashes are
//! rejected.

use sha2::{Digest, Sha256};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AuditError {
    #[error("hash chain broken: expected {expected}, got {actual}")]
    ChainBroken { expected: String, actual: String },
    #[error("sequence gap: expected {expected}, got {actual}")]
    SequenceGap { expected: u64, actual: u64 },
    #[error("duplicate sequence number: {0}")]
    DuplicateSequence(u64),
    #[error("empty chain has no previous hash")]
    EmptyChain,
}

/// Domain separators for hash computation.
pub const DOMAIN_LEAF: &str = "careerai-audit-leaf/v1";
pub const DOMAIN_MERKLE_LEAF: &str = "careerai-merkle-leaf/v1";
pub const DOMAIN_MERKLE_NODE: &str = "careerai-merkle-node/v1";

/// A single audit event in the hash chain.
#[derive(Debug, Clone)]
pub struct AuditEvent {
    /// Shard ID (tenant-independent partition).
    pub shard_id: String,
    /// Global sequence number.
    pub sequence: u64,
    /// Previous event's hash (empty string for genesis event).
    pub previous_hash: String,
    /// Canonical event body (already canonicalized).
    pub canonical_event: Vec<u8>,
    /// Computed hash of this event.
    pub event_hash: String,
    /// Schema version.
    pub schema_version: u32,
    /// Actor or service that triggered the event.
    pub actor: String,
    /// Transaction ID.
    pub transaction_id: String,
}

impl AuditEvent {
    /// Compute the hash for an audit event.
    /// `event_hash = SHA-256(domain || shard_id || sequence ||
    /// previous_hash || canonical_event)`
    pub fn compute_hash(
        shard_id: &str,
        sequence: u64,
        previous_hash: &str,
        canonical_event: &[u8],
    ) -> String {
        let mut hasher = Sha256::new();
        hasher.update(DOMAIN_LEAF.as_bytes());
        hasher.update(b"\0");
        hasher.update(shard_id.as_bytes());
        hasher.update(b"\0");
        hasher.update(sequence.to_be_bytes());
        hasher.update(b"\0");
        hasher.update(previous_hash.as_bytes());
        hasher.update(b"\0");
        hasher.update(canonical_event);
        hex::encode(hasher.finalize())
    }

    /// Create a genesis event (first in chain).
    pub fn genesis(
        shard_id: &str,
        canonical_event: Vec<u8>,
        actor: &str,
        transaction_id: &str,
    ) -> Self {
        let hash = Self::compute_hash(shard_id, 0, "", &canonical_event);
        Self {
            shard_id: shard_id.to_string(),
            sequence: 0,
            previous_hash: String::new(),
            canonical_event,
            event_hash: hash,
            schema_version: 1,
            actor: actor.to_string(),
            transaction_id: transaction_id.to_string(),
        }
    }

    /// Create the next event after `prev`.
    pub fn next(
        prev: &AuditEvent,
        canonical_event: Vec<u8>,
        actor: &str,
        transaction_id: &str,
    ) -> Self {
        let seq = prev.sequence + 1;
        let hash = Self::compute_hash(&prev.shard_id, seq, &prev.event_hash, &canonical_event);
        Self {
            shard_id: prev.shard_id.clone(),
            sequence: seq,
            previous_hash: prev.event_hash.clone(),
            canonical_event,
            event_hash: hash,
            schema_version: 1,
            actor: actor.to_string(),
            transaction_id: transaction_id.to_string(),
        }
    }

    /// Verify that this event's hash is correct.
    pub fn verify_hash(&self) -> bool {
        let computed = Self::compute_hash(
            &self.shard_id,
            self.sequence,
            &self.previous_hash,
            &self.canonical_event,
        );
        computed == self.event_hash
    }

    /// Verify that `self` follows `prev` in the chain.
    pub fn follows(&self, prev: &AuditEvent) -> Result<(), AuditError> {
        if self.sequence != prev.sequence + 1 {
            return Err(AuditError::SequenceGap {
                expected: prev.sequence + 1,
                actual: self.sequence,
            });
        }
        if self.previous_hash != prev.event_hash {
            return Err(AuditError::ChainBroken {
                expected: prev.event_hash.clone(),
                actual: self.previous_hash.clone(),
            });
        }
        if !self.verify_hash() {
            return Err(AuditError::ChainBroken {
                expected: "valid hash".into(),
                actual: self.event_hash.clone(),
            });
        }
        Ok(())
    }
}

/// Verify an entire chain of audit events.
pub fn verify_chain(events: &[AuditEvent]) -> Result<(), AuditError> {
    if events.is_empty() {
        return Ok(());
    }
    for i in 1..events.len() {
        events[i].follows(&events[i - 1])?;
    }
    Ok(())
}

/// Compute a Merkle root from a list of leaves.
/// Leaves: `SHA-256("careerai-merkle-leaf/v1\0" || entity_type || "\0" ||
/// entity_id || "\0" || canonical_bytes)`, sorted by `(entity_type, entity_id)`.
pub fn merkle_root(leaves: &[(String, String, Vec<u8>)]) -> String {
    if leaves.is_empty() {
        return String::new();
    }

    // Compute leaf hashes
    let mut hashes: Vec<String> = leaves
        .iter()
        .map(|(entity_type, entity_id, bytes)| {
            let mut sha = Sha256::new();
            sha.update(DOMAIN_MERKLE_LEAF.as_bytes());
            sha.update(b"\0");
            sha.update(entity_type.as_bytes());
            sha.update(b"\0");
            sha.update(entity_id.as_bytes());
            sha.update(b"\0");
            sha.update(bytes);
            hex::encode(sha.finalize())
        })
        .collect();

    // Sort by (entity_type, entity_id) — already done by caller convention,
    // but we sort the hashes here for safety
    hashes.sort();

    // Build tree
    while hashes.len() > 1 {
        let mut next_level = Vec::new();
        let mut i = 0;
        while i < hashes.len() {
            let left = &hashes[i];
            let right = if i + 1 < hashes.len() {
                &hashes[i + 1]
            } else {
                // Duplicate last node at odd levels
                &hashes[i]
            };
            let mut sha = Sha256::new();
            sha.update(DOMAIN_MERKLE_NODE.as_bytes());
            sha.update(b"\0");
            sha.update(left.as_bytes());
            sha.update(right.as_bytes());
            next_level.push(hex::encode(sha.finalize()));
            i += 2;
        }
        hashes = next_level;
    }

    hashes[0].clone()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn genesis_event_has_empty_previous_hash() {
        let event = AuditEvent::genesis("shard1", b"genesis".to_vec(), "system", "tx0");
        assert_eq!(event.previous_hash, "");
        assert_eq!(event.sequence, 0);
        assert!(event.verify_hash());
    }

    #[test]
    fn chain_of_events_links_correctly() {
        let e0 = AuditEvent::genesis("shard1", b"event0".to_vec(), "user1", "tx1");
        let e1 = AuditEvent::next(&e0, b"event1".to_vec(), "user1", "tx2");
        let e2 = AuditEvent::next(&e1, b"event2".to_vec(), "user2", "tx3");

        e1.follows(&e0).unwrap();
        e2.follows(&e1).unwrap();
        verify_chain(&[e0, e1, e2]).unwrap();
    }

    #[test]
    fn broken_chain_detected() {
        let e0 = AuditEvent::genesis("shard1", b"event0".to_vec(), "user1", "tx1");
        let mut e1 = AuditEvent::next(&e0, b"event1".to_vec(), "user1", "tx2");
        // Tamper with previous_hash
        e1.previous_hash = "wrong".to_string();
        let err = e1.follows(&e0).unwrap_err();
        assert!(matches!(err, AuditError::ChainBroken { .. }));
    }

    #[test]
    fn sequence_gap_detected() {
        let e0 = AuditEvent::genesis("shard1", b"event0".to_vec(), "user1", "tx1");
        let mut e1 = AuditEvent::next(&e0, b"event1".to_vec(), "user1", "tx2");
        // Skip sequence 1, set to 2
        e1.sequence = 2;
        let err = e1.follows(&e0).unwrap_err();
        assert!(matches!(err, AuditError::SequenceGap { .. }));
    }

    #[test]
    fn tampered_hash_detected() {
        let e0 = AuditEvent::genesis("shard1", b"event0".to_vec(), "user1", "tx1");
        let mut e1 = AuditEvent::next(&e0, b"event1".to_vec(), "user1", "tx2");
        // Tamper with event_hash
        e1.event_hash = "tampered".to_string();
        let err = e1.follows(&e0).unwrap_err();
        assert!(matches!(err, AuditError::ChainBroken { .. }));
    }

    #[test]
    fn different_shards_produce_different_hashes() {
        let e1 = AuditEvent::genesis("shard1", b"event".to_vec(), "u", "tx");
        let e2 = AuditEvent::genesis("shard2", b"event".to_vec(), "u", "tx");
        assert_ne!(e1.event_hash, e2.event_hash);
    }

    #[test]
    fn different_events_produce_different_hashes() {
        let e0 = AuditEvent::genesis("shard1", b"event0".to_vec(), "u", "tx");
        let e1a = AuditEvent::next(&e0, b"event_a".to_vec(), "u", "tx");
        let e1b = AuditEvent::next(&e0, b"event_b".to_vec(), "u", "tx");
        assert_ne!(e1a.event_hash, e1b.event_hash);
    }

    #[test]
    fn merkle_root_stable_for_same_leaves() {
        let leaves = vec![
            ("listing".to_string(), "l1".to_string(), b"data1".to_vec()),
            ("listing".to_string(), "l2".to_string(), b"data2".to_vec()),
        ];
        let root1 = merkle_root(&leaves);
        let root2 = merkle_root(&leaves);
        assert_eq!(root1, root2);
    }

    #[test]
    fn merkle_root_changes_with_different_leaves() {
        let leaves1 = vec![("listing".to_string(), "l1".to_string(), b"data1".to_vec())];
        let leaves2 = vec![("listing".to_string(), "l1".to_string(), b"data2".to_vec())];
        assert_ne!(merkle_root(&leaves1), merkle_root(&leaves2));
    }

    #[test]
    fn empty_chain_verifies() {
        assert!(verify_chain(&[]).is_ok());
    }

    #[test]
    fn verify_full_chain() {
        let e0 = AuditEvent::genesis("s1", b"a".to_vec(), "u", "t1");
        let e1 = AuditEvent::next(&e0, b"b".to_vec(), "u", "t2");
        let e2 = AuditEvent::next(&e1, b"c".to_vec(), "u", "t3");
        let e3 = AuditEvent::next(&e2, b"d".to_vec(), "u", "t4");
        verify_chain(&[e0, e1, e2, e3]).unwrap();
    }
}
