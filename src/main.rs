mod components;
#[rustfmt::skip]
mod config;
mod app;
mod backend;
mod localize;
mod screenshot;
mod cosmic_workspace_capture;
mod subscriptions;
use tracing::info;

use localize::localize;

use crate::config::VERSION;

fn main() -> cosmic::iced::Result {
    init_logging();

    println!("DEBUG: Starting cosmic-launcher");
    info!(
        "cosmic-launcher ({})",
        <app::CosmicLauncher as cosmic::Application>::APP_ID
    );
    info!("Version: {} ({})", VERSION, config::profile());
    println!("DEBUG: Version: {} ({})", VERSION, config::profile());

    // Prepare i18n
    localize();

    println!("DEBUG: Running app");
    app::run()
}

fn init_logging() {
    use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

    // Initialize logger
    #[cfg(feature = "console")]
    if std::env::var("TOKIO_CONSOLE").as_deref() == Ok("1") {
        std::env::set_var("RUST_LOG", "trace");
        console_subscriber::init();
    }

    let filter_layer = EnvFilter::try_from_default_env().unwrap_or(if cfg!(debug_assertions) {
        EnvFilter::new(format!("warn,{}=debug", env!("CARGO_CRATE_NAME")))
    } else {
        EnvFilter::new("warn")
    });

    let fmt_layer = fmt::layer().with_target(false);

    if let Ok(journal_layer) = tracing_journald::layer() {
        tracing_subscriber::registry()
            .with(journal_layer)
            .with(filter_layer)
            .init();
    } else {
        tracing_subscriber::registry()
            .with(fmt_layer)
            .with(filter_layer)
            .init();
    }
}
