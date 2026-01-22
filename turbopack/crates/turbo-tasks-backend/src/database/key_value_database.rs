use anyhow::Result;

use crate::database::write_batch::{
    ConcurrentWriteBatch, SerialWriteBatch, UnimplementedWriteBatch, WriteBatch,
};

#[derive(Debug, Clone, Copy)]
pub enum KeySpace {
    Infra = 0,
    TaskMeta = 1,
    TaskData = 2,
    TaskCache = 3,
    TaskIdToTaskTypeHash = 4,
}

pub trait KeyValueDatabase {
    type ReadTransaction<'l>
    where
        Self: 'l;

    fn begin_read_transaction(&self) -> Result<Self::ReadTransaction<'_>>;

    fn is_empty(&self) -> bool {
        false
    }

    type ValueBuffer<'l>: std::borrow::Borrow<[u8]>
    where
        Self: 'l;

    fn get<'l, 'db: 'l>(
        &'l self,
        transaction: &'l Self::ReadTransaction<'db>,
        key_space: KeySpace,
        key: &[u8],
    ) -> Result<Option<Self::ValueBuffer<'l>>>;

    fn batch_get<'l, 'db: 'l>(
        &'l self,
        transaction: &'l Self::ReadTransaction<'db>,
        key_space: KeySpace,
        keys: &[&[u8]],
    ) -> Result<Vec<Option<Self::ValueBuffer<'l>>>> {
        let mut results = Vec::with_capacity(keys.len());
        for key in keys {
            let value = self.get(transaction, key_space, key)?;
            results.push(value);
        }
        Ok(results)
    }

    /// Looks up a key by its hash, confirming the match by comparing the value.
    ///
    /// This is useful for reverse lookups where you have a secondary index mapping
    /// values (e.g., TaskIds) back to key hashes. Instead of comparing keys (which may
    /// be large), this method finds entries with matching hash and confirms by comparing
    /// values.
    ///
    /// Returns the key bytes if an entry with matching hash and value is found.
    ///
    /// Default implementation returns None (not supported).
    fn lookup_key_by_hash_and_value<'l, 'db: 'l>(
        &'l self,
        _transaction: &'l Self::ReadTransaction<'db>,
        _key_space: KeySpace,
        _key_hash: u64,
        _expected_value: &[u8],
    ) -> Result<Option<Self::ValueBuffer<'l>>> {
        Ok(None)
    }

    type SerialWriteBatch<'l>: SerialWriteBatch<'l>
        = UnimplementedWriteBatch
    where
        Self: 'l;
    type ConcurrentWriteBatch<'l>: ConcurrentWriteBatch<'l>
        = UnimplementedWriteBatch
    where
        Self: 'l;

    fn write_batch(
        &self,
    ) -> Result<WriteBatch<'_, Self::SerialWriteBatch<'_>, Self::ConcurrentWriteBatch<'_>>>;

    /// Called when the database has been invalidated via
    /// [`crate::backing_storage::BackingStorage::invalidate`]
    ///
    /// This typically means that we'll restart the process or `turbo-tasks` soon with a fresh
    /// database. If this happens, there's no point in writing anything else to disk, or flushing
    /// during [`KeyValueDatabase::shutdown`].
    ///
    /// This is a best-effort optimization hint, and the database may choose to ignore this and
    /// continue file writes. This happens after the database is invalidated, so it is valid for
    /// this to leave the database in a half-updated and corrupted state.
    fn prevent_writes(&self) {
        // this is an optional performance hint to the database
    }

    fn shutdown(&self) -> Result<()> {
        Ok(())
    }
}
