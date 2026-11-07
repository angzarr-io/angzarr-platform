//! The store: the in-memory store, and the journal that makes it durable.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use angzarr_ports::angzarr_proto::v1::{Cover, EditionSummary, EventBook, EventPage, Snapshot};
use angzarr_ports::memory::{MemoryRead, MemoryStore};
use angzarr_ports::{
    ChangeToken, DeadLetter, DeadLetterKey, DeadLetterStore, Editions, EventStore, Head,
    ListStreams, Outbox, OutboxEntry, PositionKey, PositionStore, Processed, Purge,
    ReadByCorrelation, ReadStore, ReadUntilTime, Slice, SnapshotStore, StoreError, StreamRef, Uuid,
    Watch, Write, Written,
};
use file_system::Fs;
use prost_types::Timestamp;

use crate::entry::{self, Plan};
use crate::error::{OpenError, TornTail};

/// Whether an append is synced before the call returns.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum Durability {
    /// `sync_all` after every append: an operation that returned is on
    /// disk. The default.
    #[default]
    Durable,
    /// No sync: an operation that returned is in the operating system's
    /// hands, and survives the process but not a power cut. For tests and
    /// ephemeral stores.
    Unsynced,
}

/// How a store is opened.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct Options {
    pub durability: Durability,
}

/// An opened store and what opening had to repair.
#[derive(Debug)]
pub struct Opened {
    pub store: JsonlStore,
    /// The torn last line opening removed, if there was one.
    pub recovered: Option<TornTail>,
}

/// The store. Clones are handles on one store and share its lock; the
/// journal is released when the last one is dropped.
#[derive(Clone, Debug)]
pub struct JsonlStore {
    shared: Arc<Shared>,
}

#[derive(Debug)]
struct Shared {
    path: PathBuf,
}

/// Where an accepted operation's line is appended: the journal file, or in
/// a test a stand-in that fails or tears.
pub(crate) trait Sink: Send {
    /// Append `line` (it ends with its line feed) to the journal, whole or
    /// not at all, and make it durable if the store was opened durable.
    fn append(&mut self, line: &[u8]) -> io::Result<()>;
}

impl JsonlStore {
    /// Open as [`JsonlStore::open_with`], with the journal's sink passed
    /// through `wrap`: how the tests fail or tear an append.
    pub(crate) fn open_wrapping(
        _fs: Arc<dyn Fs>,
        _journal: &Path,
        _options: Options,
        _wrap: impl FnOnce(Box<dyn Sink>) -> Box<dyn Sink>,
    ) -> Result<Opened, OpenError> {
        todo!()
    }

    /// Open the journal at `journal`, an absolute path, with the default
    /// [`Options`]. See [`JsonlStore::open_with`].
    pub fn open(fs: Arc<dyn Fs>, journal: &Path) -> Result<Opened, OpenError> {
        JsonlStore::open_with(fs, journal, Options::default())
    }

    /// Open the journal at `journal`, an absolute path, making it (and its
    /// directory) if it is not there: take its exclusive lock, replay it
    /// into memory, and repair a torn last line.
    pub fn open_with(
        _fs: Arc<dyn Fs>,
        _journal: &Path,
        _options: Options,
    ) -> Result<Opened, OpenError> {
        todo!()
    }

    /// The journal's path.
    pub fn journal(&self) -> &Path {
        &self.shared.path
    }

    /// Run a read against the in-memory store, under the lock the writes
    /// take, so it sees only what the journal holds.
    fn observe<T>(
        &self,
        _read: impl AsyncFnOnce(&MemoryStore) -> Result<T, StoreError>,
    ) -> Result<T, StoreError> {
        todo!()
    }

    /// Run an operation against the in-memory store and, if it accepted it,
    /// append `plan`'s line before returning.
    fn commit<T>(
        &self,
        _plan: Plan,
        _op: impl AsyncFnOnce(&MemoryStore) -> Result<T, StoreError>,
    ) -> Result<T, StoreError> {
        todo!()
    }
}

impl EventStore for JsonlStore {
    async fn write(&self, write: Write<'_>) -> Result<Written, StoreError> {
        self.commit(entry::write(&write), async |inner| inner.write(write).await)
    }

    async fn read(
        &self,
        stream: StreamRef<'_>,
        from: u64,
        to: Option<u64>,
    ) -> Result<Slice, StoreError> {
        self.observe(async |inner| inner.read(stream, from, to).await)
    }

    async fn head(&self, stream: StreamRef<'_>) -> Result<Head, StoreError> {
        self.observe(async |inner| inner.head(stream).await)
    }

    async fn find_command(
        &self,
        stream: StreamRef<'_>,
        command_id: Uuid,
    ) -> Result<Vec<EventPage>, StoreError> {
        self.observe(async |inner| inner.find_command(stream, command_id).await)
    }

    async fn covers(&self, domain: &str) -> Result<Vec<Cover>, StoreError> {
        self.observe(async |inner| inner.covers(domain).await)
    }
}

impl SnapshotStore for JsonlStore {
    async fn read_snapshot(&self, stream: StreamRef<'_>) -> Result<Option<Snapshot>, StoreError> {
        self.observe(async |inner| inner.read_snapshot(stream).await)
    }

    async fn write_snapshot(
        &self,
        stream: StreamRef<'_>,
        snapshot: &Snapshot,
    ) -> Result<(), StoreError> {
        self.commit(entry::snapshot(stream, snapshot), async |inner| {
            inner.write_snapshot(stream, snapshot).await
        })
    }
}

impl PositionStore for JsonlStore {
    async fn position(&self, key: PositionKey<'_>) -> Result<u64, StoreError> {
        self.observe(async |inner| inner.position(key).await)
    }
}

impl Purge for JsonlStore {
    async fn purge(&self, stream: StreamRef<'_>, expected: u64) -> Result<u64, StoreError> {
        self.commit(entry::purge(stream, expected), async |inner| {
            inner.purge(stream, expected).await
        })
    }
}

impl Editions for JsonlStore {
    async fn list_editions(&self) -> Result<Vec<EditionSummary>, StoreError> {
        self.observe(async |inner| inner.list_editions().await)
    }

    async fn remove_edition_stream(&self, stream: StreamRef<'_>) -> Result<u64, StoreError> {
        self.commit(entry::remove_edition_stream(stream), async |inner| {
            inner.remove_edition_stream(stream).await
        })
    }
}

impl ReadStore for JsonlStore {
    type Reads<'s> = MemoryRead;

    async fn begin_read(&self) -> Result<MemoryRead, StoreError> {
        self.observe(async |inner| inner.begin_read().await)
    }
}

impl Watch for JsonlStore {
    async fn change_token(&self) -> Result<ChangeToken, StoreError> {
        self.observe(async |inner| inner.change_token().await)
    }
}

impl ReadUntilTime for JsonlStore {
    async fn read_until_time(
        &self,
        stream: StreamRef<'_>,
        until: &Timestamp,
    ) -> Result<Slice, StoreError> {
        self.observe(async |inner| inner.read_until_time(stream, until).await)
    }
}

impl ReadByCorrelation for JsonlStore {
    async fn read_by_correlation(
        &self,
        domain: Option<&str>,
        edition: Option<&str>,
        correlation_id: Uuid,
    ) -> Result<Vec<EventBook>, StoreError> {
        self.observe(async |inner| {
            inner
                .read_by_correlation(domain, edition, correlation_id)
                .await
        })
    }
}

impl ListStreams for JsonlStore {
    async fn list_streams(&self) -> Result<Vec<Cover>, StoreError> {
        self.observe(async |inner| inner.list_streams().await)
    }
}

impl Processed for JsonlStore {
    async fn processed(&self, handler: &str) -> Result<Vec<(Cover, u64)>, StoreError> {
        self.observe(async |inner| inner.processed(handler).await)
    }
}

impl Outbox for JsonlStore {
    async fn pending(&self, limit: usize) -> Result<Vec<OutboxEntry>, StoreError> {
        self.observe(async |inner| inner.pending(limit).await)
    }

    async fn clear(&self, ids: &[Uuid]) -> Result<u64, StoreError> {
        self.commit(entry::clear(ids), async |inner| inner.clear(ids).await)
    }
}

impl DeadLetterStore for JsonlStore {
    async fn dead_letters(&self, stream: StreamRef<'_>) -> Result<Vec<DeadLetter>, StoreError> {
        self.observe(async |inner| inner.dead_letters(stream).await)
    }

    async fn dead_letter(&self, key: DeadLetterKey<'_>) -> Result<Option<DeadLetter>, StoreError> {
        self.observe(async |inner| inner.dead_letter(key).await)
    }

    async fn dead_lettered(&self, domain: &str) -> Result<Vec<Cover>, StoreError> {
        self.observe(async |inner| inner.dead_lettered(domain).await)
    }

    async fn drain_dead_letters(
        &self,
        stream: StreamRef<'_>,
    ) -> Result<Vec<DeadLetter>, StoreError> {
        self.commit(entry::drain(stream), async |inner| {
            inner.drain_dead_letters(stream).await
        })
    }

    async fn remove_dead_letters(&self, keys: &[DeadLetterKey<'_>]) -> Result<u64, StoreError> {
        self.commit(entry::remove_dead_letters(keys), async |inner| {
            inner.remove_dead_letters(keys).await
        })
    }
}

#[cfg(test)]
#[path = "store.contract.test.rs"]
mod contract_tests;

#[cfg(test)]
#[path = "store.test.rs"]
mod tests;
