//! Structured logging to a rolling file (and the console in debug builds).
//!
//! Secrets never reach the log: the engine only logs host names, key fingerprints and
//! error kinds, and password/passphrase values are wrapped in types that print as
//! `<N bytes>`.

use aether_core::AppPaths;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// Initialises logging; keep the returned guard alive for the life of the process.
pub fn init(paths: &AppPaths) -> Option<WorkerGuard> {
    let default_filter = if cfg!(debug_assertions) {
        "info,aether_app=debug,aether_ssh=debug,aether_core=debug,russh=warn"
    } else {
        "info,russh=warn"
    };
    let filter = EnvFilter::try_from_env("AETHER_LOG").unwrap_or_else(|_| EnvFilter::new(default_filter));

    let appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix("aether")
        .filename_suffix("log")
        .max_log_files(7)
        .build(&paths.logs_dir);

    let registry = tracing_subscriber::registry().with(filter);
    let guard = match appender {
        Ok(appender) => {
            let (writer, guard) = tracing_appender::non_blocking(appender);
            let file_layer = tracing_subscriber::fmt::layer().with_writer(writer).with_ansi(false).with_target(true);
            if cfg!(debug_assertions) {
                let console = tracing_subscriber::fmt::layer().with_target(false).with_span_events(FmtSpan::NONE);
                registry.with(file_layer).with(console).try_init().ok();
            } else {
                registry.with(file_layer).try_init().ok();
            }
            Some(guard)
        }
        Err(e) => {
            eprintln!("could not open the log file: {e}");
            registry.with(tracing_subscriber::fmt::layer().with_target(false)).try_init().ok();
            None
        }
    };

    // Panics are logged with a backtrace before the process unwinds/aborts.
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        tracing::error!(panic = %info, backtrace = %std::backtrace::Backtrace::force_capture(), "panic");
        previous(info);
    }));
    guard
}
