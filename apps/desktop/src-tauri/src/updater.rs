use std::future::Future;
use std::time::Duration;

use tauri::AppHandle;
use tauri_plugin_updater::UpdaterExt;

const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PollDecision {
    Continue,
    Stop,
}

async fn poll_for_updates<F, Fut>(interval: Duration, mut check: F)
where
    F: FnMut() -> Fut,
    Fut: Future<Output = PollDecision>,
{
    loop {
        if check().await == PollDecision::Stop {
            return;
        }
        tokio::time::sleep(interval).await;
    }
}

async fn check_and_install(app: &AppHandle) -> PollDecision {
    let updater = match app.updater() {
        Ok(updater) => updater,
        Err(_) => {
            tracing::warn!("auto-update indisponível; configuração pública ainda não está válida");
            return PollDecision::Continue;
        }
    };

    let update = match updater.check().await {
        Ok(update) => update,
        Err(_) => {
            tracing::warn!(
                "não foi possível consultar atualizações; nova tentativa no próximo ciclo"
            );
            return PollDecision::Continue;
        }
    };

    let Some(update) = update else {
        tracing::debug!("OpenJarvisBR já está na versão mais recente");
        return PollDecision::Continue;
    };

    let version = update.version.clone();
    tracing::info!(version = %version, "nova versão encontrada; baixando atualização assinada");

    match update.download_and_install(|_, _| {}, || {}).await {
        Ok(()) => {
            tracing::info!(
                version = %version,
                "atualização instalada; a nova versão abrirá no próximo início do app"
            );
            PollDecision::Stop
        }
        Err(_) => {
            tracing::warn!(
                version = %version,
                "não foi possível instalar a atualização; nova tentativa no próximo ciclo"
            );
            PollDecision::Continue
        }
    }
}

pub(crate) fn start(app: AppHandle) {
    tauri::async_runtime::spawn(poll_for_updates(CHECK_INTERVAL, move || {
        let app = app.clone();
        async move { check_and_install(&app).await }
    }));
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    use super::{poll_for_updates, PollDecision};

    #[tokio::test]
    async fn checks_immediately_and_repeats_after_the_interval() {
        let checks = Arc::new(AtomicUsize::new(0));
        let observed = checks.clone();

        let task = tokio::spawn(poll_for_updates(Duration::from_millis(10), move || {
            let observed = observed.clone();
            async move {
                observed.fetch_add(1, Ordering::SeqCst);
                PollDecision::Continue
            }
        }));

        tokio::task::yield_now().await;
        assert_eq!(
            checks.load(Ordering::SeqCst),
            1,
            "a checagem inicial deve acontecer sem esperar o intervalo"
        );

        tokio::time::timeout(Duration::from_millis(50), async {
            while checks.load(Ordering::SeqCst) < 2 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("uma nova checagem deve acontecer depois do intervalo");

        task.abort();
    }

    #[tokio::test]
    async fn stops_polling_after_an_update_is_installed() {
        let checks = Arc::new(AtomicUsize::new(0));
        let observed = checks.clone();

        poll_for_updates(Duration::from_millis(1), move || {
            let observed = observed.clone();
            async move {
                observed.fetch_add(1, Ordering::SeqCst);
                PollDecision::Stop
            }
        })
        .await;

        assert_eq!(checks.load(Ordering::SeqCst), 1);
    }
}
