#![cfg(test)]

use std::sync::Once;

/// Initialize the global tracing subscriber once (used by tests that run with `RUST_LOG`).
pub fn init_tracing_from_env() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        let subscriber = tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .with_writer(std::io::stdout);
        let _ = subscriber.try_init();
    });
}
