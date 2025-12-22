//! Incan compiler CLI entry point

fn main() {
    // Initialize structured logging with env-based filter, defaulting to info
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .try_init();

    incan::cli::run();
}
