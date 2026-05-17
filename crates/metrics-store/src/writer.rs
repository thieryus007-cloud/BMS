//! Writer skeleton — implémentation complète en §5.3 du plan.
//!
//! Pour cette première étape (squelette de crate) on expose le handle [`Writer`]
//! et le type [`Sample`] pour figer l'API publique. La boucle `writer_loop` qui
//! batch les samples et commit toutes les ~250 ms sera ajoutée dans le ticket
//! suivant (Phase 0.4).

use smallvec::SmallVec;
use tokio::sync::mpsc;

/// Échantillon de métrique à ingérer.
#[derive(Debug, Clone)]
pub struct Sample {
    pub metric: String,
    pub labels: SmallVec<[(String, String); 4]>,
    pub ts_ms: i64,
    pub value: f64,
}

/// Handle clonable côté producteur — envoie les samples vers la tâche d'écriture.
#[derive(Clone)]
pub struct Writer {
    pub(crate) tx: mpsc::Sender<Sample>,
}

impl Writer {
    pub async fn write(&self, sample: Sample) -> Result<(), mpsc::error::SendError<Sample>> {
        self.tx.send(sample).await
    }

    pub fn try_write(&self, sample: Sample) -> Result<(), mpsc::error::TrySendError<Sample>> {
        self.tx.try_send(sample)
    }
}
