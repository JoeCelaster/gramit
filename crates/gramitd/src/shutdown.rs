use tokio::sync::watch;

/// Fans a shutdown signal out to the accept loop and every open connection.
///
/// Cloneable and cheap: each waiter subscribes to the same watch channel, so a
/// `Shutdown` request on one connection stops the whole daemon.
#[derive(Clone)]
pub struct Shutdown {
    tx: watch::Sender<bool>,
}

impl Shutdown {
    pub fn new() -> Self {
        let (tx, _rx) = watch::channel(false);
        Self { tx }
    }

    pub fn trigger(&self) {
        // `send` reports failure — and skips the update — when no receiver is currently
        // subscribed, which is the normal state before the accept loop starts waiting.
        // `send_replace` stores the flag either way.
        self.tx.send_replace(true);
    }

    /// Only the tests consult this today; the hotkey loop in Module 2c will too.
    #[allow(dead_code)]
    pub fn is_triggered(&self) -> bool {
        *self.tx.borrow()
    }

    /// Resolves once shutdown has been requested (immediately, if it already has).
    pub async fn wait(&self) {
        let mut rx = self.tx.subscribe();
        loop {
            if *rx.borrow_and_update() {
                return;
            }
            if rx.changed().await.is_err() {
                return;
            }
        }
    }
}

impl Default for Shutdown {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn wait_resolves_after_trigger() {
        let shutdown = Shutdown::new();
        assert!(!shutdown.is_triggered());

        let waiter = shutdown.clone();
        let task = tokio::spawn(async move { waiter.wait().await });

        shutdown.trigger();
        tokio::time::timeout(std::time::Duration::from_secs(2), task)
            .await
            .expect("wait should resolve promptly")
            .unwrap();

        assert!(shutdown.is_triggered());
    }

    #[tokio::test]
    async fn wait_resolves_immediately_when_already_triggered() {
        let shutdown = Shutdown::new();
        shutdown.trigger();

        tokio::time::timeout(std::time::Duration::from_secs(2), shutdown.wait())
            .await
            .expect("already-triggered wait must not block");
    }
}
