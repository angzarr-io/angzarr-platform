//! The journal's entries and the operations they record: what each store
//! operation is journaled as, and how a journaled entry is applied again.

use angzarr_ports::angzarr_proto::v1::Snapshot;
use angzarr_ports::memory::MemoryStore;
use angzarr_ports::{DeadLetterKey, StreamRef, Uuid, Write};

use crate::schema;

/// What an accepted operation appends.
pub(crate) enum Plan {
    /// Nothing: the operation changes nothing (a write that only checked).
    Skip,
    /// This entry, one line.
    Line(schema::Entry),
    /// The operation cannot be written as an entry (a dead letter about a
    /// stream that is no stream); the store refuses it, so it is run only
    /// to say how.
    Unencodable,
}

/// Why a journaled entry cannot be applied again.
#[derive(Debug)]
pub(crate) enum Replay {
    /// The entry is not one a store writes: a field it needs is missing or
    /// malformed. Names the field.
    Malformed(&'static str),
    /// The in-memory store refused the recorded operation.
    Refused(angzarr_ports::StoreError),
    /// The in-memory store did not complete at once.
    Suspended,
}

pub(crate) fn write(_write: &Write<'_>) -> Plan {
    todo!()
}

pub(crate) fn snapshot(_stream: StreamRef<'_>, _snapshot: &Snapshot) -> Plan {
    todo!()
}

pub(crate) fn purge(_stream: StreamRef<'_>, _expected: u64) -> Plan {
    todo!()
}

pub(crate) fn remove_edition_stream(_stream: StreamRef<'_>) -> Plan {
    todo!()
}

pub(crate) fn clear(_ids: &[Uuid]) -> Plan {
    todo!()
}

pub(crate) fn drain(_stream: StreamRef<'_>) -> Plan {
    todo!()
}

pub(crate) fn remove_dead_letters(_keys: &[DeadLetterKey<'_>]) -> Plan {
    todo!()
}

/// Apply `entry` to `inner` as its operation was first applied.
pub(crate) fn replay(_inner: &MemoryStore, _entry: schema::Entry) -> Result<(), Replay> {
    todo!()
}
