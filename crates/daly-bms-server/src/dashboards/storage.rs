//! Persistance des layouts dashboard côté serveur (SQLite).
//!
//! Schéma : un row JSON par user (1 user "default" pour l'instant — extensible).
//! Stockage : fichier SQLite séparé `dashboards.db` dans le même dossier que `alerts.db`.

use rusqlite::{Connection, params, OptionalExtension};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use chrono::Utc;

/// Storage clonable (Arc<Mutex>) pour les layouts dashboard.
#[derive(Clone)]
pub struct LayoutStorage {
    conn: Arc<Mutex<Connection>>,
}

/// Récupère le lock du mutex en gérant le cas du mutex empoisonné.
/// Plutôt que paniquer (.unwrap()), on récupère la guard depuis l'erreur
/// — l'état SQLite reste consistant car rusqlite a son propre verrouillage interne.
fn lock_conn(m: &Mutex<Connection>) -> MutexGuard<'_, Connection> {
    m.lock().unwrap_or_else(|poisoned| {
        tracing::warn!("dashboard storage: mutex poisoned, récupération de la guard");
        poisoned.into_inner()
    })
}

impl LayoutStorage {
    /// Ouvre (ou crée) la base à `path`. Initialise le schéma.
    pub fn open(path: impl AsRef<Path>) -> rusqlite::Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let conn = Connection::open(path)?;
        init_schema(&conn)?;
        Ok(Self { conn: Arc::new(Mutex::new(conn)) })
    }

    /// Dérive le chemin par défaut depuis le chemin de la DB alertes.
    /// Ex: `/var/lib/daly-bms/alerts.db` → `/var/lib/daly-bms/dashboards.db`.
    /// Si `alerts_db_path` est vide, utilise `./dashboards.db`.
    pub fn default_path(alerts_db_path: &str) -> PathBuf {
        if alerts_db_path.is_empty() {
            return PathBuf::from("dashboards.db");
        }
        let p = Path::new(alerts_db_path);
        let dir = p.parent().unwrap_or_else(|| Path::new("."));
        dir.join("dashboards.db")
    }

    /// Récupère le layout JSON brut pour un user. None si aucun layout sauvé.
    pub fn get(&self, user_id: &str) -> rusqlite::Result<Option<String>> {
        let conn = lock_conn(&self.conn);
        conn.query_row(
            "SELECT layout_json FROM dashboard_layouts WHERE user_id = ?1",
            params![user_id],
            |row| row.get::<_, String>(0),
        ).optional()
    }

    /// Écrit (upsert) le layout JSON pour un user. Le contenu n'est pas validé ici —
    /// l'appelant garantit que c'est du JSON.
    pub fn set(&self, user_id: &str, layout_json: &str) -> rusqlite::Result<()> {
        let conn = lock_conn(&self.conn);
        let now = Utc::now().timestamp();
        conn.execute(
            "INSERT INTO dashboard_layouts (user_id, layout_json, updated_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(user_id) DO UPDATE SET
                layout_json = excluded.layout_json,
                updated_at  = excluded.updated_at",
            params![user_id, layout_json, now],
        )?;
        Ok(())
    }

    /// Supprime le layout d'un user (retour à l'état par défaut côté UI).
    pub fn delete(&self, user_id: &str) -> rusqlite::Result<()> {
        let conn = lock_conn(&self.conn);
        conn.execute(
            "DELETE FROM dashboard_layouts WHERE user_id = ?1",
            params![user_id],
        )?;
        Ok(())
    }
}

fn init_schema(conn: &Connection) -> rusqlite::Result<()> {
    // Pas d'index sur updated_at : aucune requête actuelle ne le filtre/trie
    // et un index coûte sur chaque write.
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS dashboard_layouts (
            user_id     TEXT PRIMARY KEY,
            layout_json TEXT NOT NULL,
            updated_at  INTEGER NOT NULL
         );"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crud_round_trip() {
        let path = std::env::temp_dir().join(format!("daly-test-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let storage = LayoutStorage::open(&path).unwrap();
        assert!(storage.get("alice").unwrap().is_none());
        storage.set("alice", r#"[{"id":"1","x":0,"y":0,"w":12,"h":8}]"#).unwrap();
        let got = storage.get("alice").unwrap().unwrap();
        assert!(got.contains("\"id\":\"1\""));
        storage.set("alice", r#"[]"#).unwrap();
        assert_eq!(storage.get("alice").unwrap().unwrap(), "[]");
        storage.delete("alice").unwrap();
        assert!(storage.get("alice").unwrap().is_none());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn default_path_alongside_alerts() {
        assert_eq!(
            LayoutStorage::default_path("/var/lib/daly-bms/alerts.db"),
            PathBuf::from("/var/lib/daly-bms/dashboards.db"),
        );
        assert_eq!(
            LayoutStorage::default_path(""),
            PathBuf::from("dashboards.db"),
        );
    }
}
