//! Supervision des tâches critiques (audit robustesse §2).
//!
//! Les modules logiques sont lancés en tâches de fond. Auparavant, si l'un
//! d'eux se terminait (retour de sa boucle), il mourait silencieusement et le
//! process restait « up » alors qu'une fonction était éteinte — le watchdog
//! systemd continuait pourtant de battre (fausse confiance). Ce helper convertit
//! la fin inattendue d'une tâche critique en arrêt du process → redémarrage
//! propre par systemd (`Restart=on-failure`).

/// Lance une tâche critique longue durée. Si son future se termine (retour) —
/// ce qui ne doit jamais arriver pour une boucle de service — on log une erreur
/// fatale et on arrête le process. Les panics sont déjà fatals via
/// `panic = "abort"` (profil release) ; ce helper couvre les retours propres.
///
/// NB : réservé aux boucles longue durée — jamais aux tâches transitoires
/// (one-shot, timers différés) qui se terminent normalement.
pub fn spawn_critical<F>(fut: F)
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    tokio::task::spawn(async move {
        fut.await;
        tracing::error!(
            "Une tâche critique s'est terminée de façon inattendue — \
             arrêt du process pour redémarrage par systemd"
        );
        // Laisse le temps aux logs de partir avant l'exit.
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        std::process::exit(1);
    });
}
