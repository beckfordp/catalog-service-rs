// SPDX-License-Identifier: MIT

//! Shared by the integration tests: gluon-rs `service-testing`'s throwaway
//! PostgreSQL cluster, this service's store over it and its binary under
//! test, an in-memory store for route tests, and a logger whose lines the
//! test can read back.

#![allow(
    dead_code,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unused_async_trait_impl
)]

use std::collections::HashMap;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use catalog_service::config::{PORT_VARIABLE, PostgresConfig};
use catalog_service::db;
use catalog_service::domain::{Catalog, CatalogDetails, CatalogId, NewCatalog, Page};
use catalog_service::store::{Catalogs, PgCatalogs, StoreError};
use serde_json::Value;
use telemetry::logging::Logger;
use time::OffsetDateTime;
use time::macros::datetime;

// Not every test file uses both.
#[allow(unused_imports)]
pub use service_testing::{Postgres, free_port};

/// The catalog-service binary, on a fresh port.
pub struct Service(service_testing::Service);

impl Service {
    /// Starts it with exactly `extra` (plus its port); does not wait.
    pub fn spawn(extra: &[(&str, String)]) -> Self {
        Self(service_testing::Service::spawn(
            env!("CARGO_BIN_EXE_catalog-service"),
            PORT_VARIABLE,
            extra,
        ))
    }

    /// Starts it and waits until `/health` answers.
    pub fn start(extra: &[(&str, String)]) -> Self {
        Self(service_testing::Service::start(
            env!("CARGO_BIN_EXE_catalog-service"),
            PORT_VARIABLE,
            extra,
        ))
    }
}

impl Deref for Service {
    type Target = service_testing::Service;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for Service {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

/// The service's store over `postgres`, migrated as the service migrates it.
pub async fn store(postgres: &Postgres) -> PgCatalogs {
    let env: HashMap<_, _> = postgres.env().into_iter().collect();
    let config = PostgresConfig {
        host: env["POSTGRES_HOST"].clone(),
        port: env["POSTGRES_PORT"].parse().unwrap(),
        database: env["POSTGRES_DB"].clone(),
        user: env["POSTGRES_USER"].clone(),
        password: env["POSTGRES_PASSWORD"].clone(),
    };
    let pool = db::connect(&config).await.unwrap();
    db::migrate(&pool).await.unwrap();
    PgCatalogs::new(pool)
}

/// An in-memory `catalog` table (Postgres's order: newest first, ties by
/// id); `failing` makes every call fail as a database would, `down` makes the
/// ping fail.
#[derive(Clone, Default)]
pub struct Memory {
    rows: Arc<Mutex<Vec<Catalog>>>,
    clock: Arc<Mutex<i64>>,
    pub failing: Arc<AtomicBool>,
    pub down: Arc<AtomicBool>,
}

impl Memory {
    fn fail(&self) -> Result<(), StoreError> {
        if self.failing.load(Ordering::SeqCst) {
            return Err(StoreError::Database(sqlx::Error::PoolClosed));
        }
        Ok(())
    }

    /// A distinct "now" per write, a second apart.
    fn now(&self) -> OffsetDateTime {
        let mut tick = self.clock.lock().unwrap();
        *tick += 1;
        datetime!(2026-11-22 12:00:00 UTC) + time::Duration::seconds(*tick)
    }
}

impl Catalogs for Memory {
    async fn create(&self, id: CatalogId, new: &NewCatalog) -> Result<Catalog, StoreError> {
        self.fail()?;
        let now = self.now();
        let entry = Catalog {
            id,
            name: new.details.name.clone(),
            description: new.details.description.clone(),
            price_cents: new.details.price_cents,
            sku: new.sku.clone(),
            created_at: now,
            updated_at: now,
        };
        self.rows.lock().unwrap().push(entry.clone());
        Ok(entry)
    }

    async fn get(&self, id: CatalogId) -> Result<Option<Catalog>, StoreError> {
        self.fail()?;
        Ok(self
            .rows
            .lock()
            .unwrap()
            .iter()
            .find(|row| row.id == id)
            .cloned())
    }

    async fn update(
        &self,
        id: CatalogId,
        details: &CatalogDetails,
    ) -> Result<Option<Catalog>, StoreError> {
        self.fail()?;
        let now = self.now();
        let mut rows = self.rows.lock().unwrap();
        Ok(rows.iter_mut().find(|row| row.id == id).map(|row| {
            row.name.clone_from(&details.name);
            row.description.clone_from(&details.description);
            row.price_cents = details.price_cents;
            row.updated_at = now;
            row.clone()
        }))
    }

    async fn delete(&self, id: CatalogId) -> Result<bool, StoreError> {
        self.fail()?;
        let mut rows = self.rows.lock().unwrap();
        let before = rows.len();
        rows.retain(|row| row.id != id);
        Ok(rows.len() < before)
    }

    async fn list(&self, page: Page) -> Result<(Vec<Catalog>, i64), StoreError> {
        self.fail()?;
        let mut rows = self.rows.lock().unwrap().clone();
        rows.sort_by_key(|row| std::cmp::Reverse((row.created_at, row.id.uuid())));
        let total = i64::try_from(rows.len()).unwrap();
        let skip = usize::try_from(page.offset).unwrap();
        let take = usize::try_from(page.limit).unwrap();
        Ok((rows.into_iter().skip(skip).take(take).collect(), total))
    }

    async fn ping(&self, _within: Duration) -> bool {
        !self.down.load(Ordering::SeqCst)
    }
}

/// Where a [`captured_logger`] writes.
pub type LogBuffer = Arc<Mutex<Vec<u8>>>;

/// A logger named as the service's, writing to a buffer the test reads back.
pub fn captured_logger() -> (Logger, LogBuffer) {
    let buffer = LogBuffer::default();
    (Logger::to_writer("catalog-service", buffer.clone()), buffer)
}

/// The lines logged to `buffer` so far, each as JSON.
pub fn log_lines(buffer: &LogBuffer) -> Vec<Value> {
    let text = String::from_utf8(buffer.lock().unwrap().clone()).unwrap();
    text.lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}
