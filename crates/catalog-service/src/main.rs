// SPDX-License-Identifier: MIT

//! The `catalog-service` binary: reads the config contract, migrates the
//! database, then serves the HTTP API until SIGTERM or SIGINT.

use catalog_service::cache::{self, ListCache};
use catalog_service::config::{self, Config};
use catalog_service::server::AppState;
use catalog_service::store::PgCatalogs;
use catalog_service::{db, server};
use redis_cache::Cache;
use service_basics::{environment, runtime, shutdown};
use std::process::ExitCode;
use std::sync::Arc;
use telemetry::logging::Logger;
use telemetry::tracing::{SdkTracer, Tracing};

fn main() -> ExitCode {
    let config = match config::from_env(&environment::variables()) {
        Ok(config) => config,
        Err(error) => {
            eprintln!("{SERVICE}: {error}");
            return ExitCode::FAILURE;
        }
    };
    // Built outside the async runtime: the OTLP exporter's blocking HTTP
    // client must not be created (or dropped) on a runtime thread.
    let tracing = match Tracing::new(
        &config.service_name,
        config.tracing.otlp_endpoint.as_deref(),
        config.tracing.sampling_ratio,
    ) {
        Ok(tracing) => tracing,
        Err(error) => {
            eprintln!("{SERVICE}: {error}");
            return ExitCode::FAILURE;
        }
    };
    let runtime = match runtime::multi_thread() {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("{SERVICE}: can't start the async runtime: {error}");
            return ExitCode::FAILURE;
        }
    };
    let code = runtime.block_on(serve(&config, tracing.tracer()));
    drop(runtime);
    tracing.shutdown();
    code
}

/// The service's name in its stderr lines and SIGTERM warning.
const SERVICE: &str = "catalog-service";

async fn serve(config: &Config, tracer: SdkTracer) -> ExitCode {
    let pool = match db::connect(&config.postgres).await {
        Ok(pool) => pool,
        Err(error) => {
            eprintln!(
                "{SERVICE}: database {}:{}/{} unreachable: {error}",
                config.postgres.host, config.postgres.port, config.postgres.database
            );
            return ExitCode::FAILURE;
        }
    };
    if let Err(error) = db::migrate(&pool).await {
        eprintln!("{SERVICE}: database migration failed: {error}");
        return ExitCode::FAILURE;
    }
    let logger = Logger::stdout(&config.service_name);
    logger.info(
        &[("port", &config.port.to_string())],
        "catalog-service starting",
    );
    let cache = match Cache::new(&config.redis_uri, cache::TIMEOUT) {
        Ok(cache) => Arc::new(cache),
        Err(error) => {
            eprintln!("{SERVICE}: {error}");
            return ExitCode::FAILURE;
        }
    };
    let state = AppState {
        store: PgCatalogs::new(pool),
        cache: ListCache::new(cache, config.list_ttl),
        logger: logger.clone(),
    };
    // No spans of its own: kept alive for the provider's lifetime.
    let _tracer = tracer;
    match server::run(config.port, state, shutdown::signal(SERVICE)).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            logger.error(&[("error", &error.to_string())], "HTTP server failed");
            ExitCode::FAILURE
        }
    }
}
