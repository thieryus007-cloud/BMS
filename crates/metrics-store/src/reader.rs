//! Reader skeleton — snapshots MVCC lock-free. Cf. plan §5.4.
//!
//! L'implémentation `query_range` arrivera dans le ticket suivant (Phase 0.5),
//! après que `tables.rs` et `encoding.rs` aient été couverts par des tests
//! d'intégration. Pour l'instant on fige uniquement la forme du handle.

use std::sync::Arc;

use redb::Database;

#[derive(Clone)]
pub struct Reader {
    pub(crate) db: Arc<Database>,
}

impl Reader {
    /// Ouvre une transaction de lecture (snapshot MVCC). Exposé pour permettre
    /// aux tests d'inspecter la base avant que `query_range` ne soit câblé.
    pub fn begin_read(&self) -> anyhow::Result<redb::ReadTransaction> {
        Ok(self.db.begin_read()?)
    }
}
