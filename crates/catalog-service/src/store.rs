// SPDX-License-Identifier: MIT

//! The `catalog` table, with the Scala `CatalogStore`'s SQL: Postgres stamps
//! `created_at`/`updated_at`; a page and its total come from one query
//! (`count(*) OVER()`), the total alone when the page is empty. Handlers reach
//! it through [`Catalogs`], so they are testable over a store in memory.

use std::future::Future;
use std::time::Duration;

use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::domain::{Catalog, CatalogDetails, CatalogId, NewCatalog, Page};

/// Why a store call failed.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// The database refused or failed.
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

/// What the service needs from `catalog`.
pub trait Catalogs: Clone + Send + Sync + 'static {
    /// Stores `entry` as `id`.
    fn create(
        &self,
        id: CatalogId,
        entry: &NewCatalog,
    ) -> impl Future<Output = Result<Catalog, StoreError>> + Send;

    /// The entry, if there is one.
    fn get(
        &self,
        id: CatalogId,
    ) -> impl Future<Output = Result<Option<Catalog>, StoreError>> + Send;

    /// Sets `details`; the entry, if there is one.
    fn update(
        &self,
        id: CatalogId,
        details: &CatalogDetails,
    ) -> impl Future<Output = Result<Option<Catalog>, StoreError>> + Send;

    /// Deletes the entry; whether there was one.
    fn delete(&self, id: CatalogId) -> impl Future<Output = Result<bool, StoreError>> + Send;

    /// One page, newest first (ties by id), and the total.
    fn list(
        &self,
        page: Page,
    ) -> impl Future<Output = Result<(Vec<Catalog>, i64), StoreError>> + Send;

    /// True when the database answers within `within` (readiness).
    fn ping(&self, within: Duration) -> impl Future<Output = bool> + Send;
}

/// `catalog` in Postgres.
#[derive(Debug, Clone)]
pub struct PgCatalogs {
    pool: PgPool,
}

impl PgCatalogs {
    /// Over `pool`, migrated.
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

/// A row and the page's total (`count(*) OVER()`).
type PageRow = (
    Uuid,
    String,
    String,
    i32,
    String,
    OffsetDateTime,
    OffsetDateTime,
    i64,
);

type Row = (
    Uuid,
    String,
    String,
    i32,
    String,
    OffsetDateTime,
    OffsetDateTime,
);

fn entry((id, name, description, price_cents, sku, created_at, updated_at): Row) -> Catalog {
    Catalog {
        id: id.into(),
        name,
        description,
        price_cents,
        sku,
        created_at,
        updated_at,
    }
}

const COLUMNS: &str = "id, name, description, price_cents, sku, created_at, updated_at";

impl Catalogs for PgCatalogs {
    async fn create(&self, id: CatalogId, new: &NewCatalog) -> Result<Catalog, StoreError> {
        let row: Row = sqlx::query_as(&format!(
            r#"INSERT INTO "catalog" (id, name, description, price_cents, sku)
               VALUES ($1, $2, $3, $4, $5) RETURNING {COLUMNS}"#
        ))
        .bind(id.uuid())
        .bind(&new.details.name)
        .bind(&new.details.description)
        .bind(new.details.price_cents)
        .bind(&new.sku)
        .fetch_one(&self.pool)
        .await?;
        Ok(entry(row))
    }

    async fn get(&self, id: CatalogId) -> Result<Option<Catalog>, StoreError> {
        let row: Option<Row> =
            sqlx::query_as(&format!(r#"SELECT {COLUMNS} FROM "catalog" WHERE id = $1"#))
                .bind(id.uuid())
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.map(entry))
    }

    async fn update(
        &self,
        id: CatalogId,
        details: &CatalogDetails,
    ) -> Result<Option<Catalog>, StoreError> {
        let row: Option<Row> = sqlx::query_as(&format!(
            r#"UPDATE "catalog" SET name = $1, description = $2, price_cents = $3, updated_at = now()
               WHERE id = $4 RETURNING {COLUMNS}"#
        ))
        .bind(&details.name)
        .bind(&details.description)
        .bind(details.price_cents)
        .bind(id.uuid())
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(entry))
    }

    async fn delete(&self, id: CatalogId) -> Result<bool, StoreError> {
        let done = sqlx::query(r#"DELETE FROM "catalog" WHERE id = $1"#)
            .bind(id.uuid())
            .execute(&self.pool)
            .await?;
        Ok(done.rows_affected() > 0)
    }

    async fn list(&self, page: Page) -> Result<(Vec<Catalog>, i64), StoreError> {
        let rows: Vec<PageRow> = sqlx::query_as(&format!(
            r#"SELECT {COLUMNS}, count(*) OVER() AS total_count FROM "catalog"
                   ORDER BY created_at DESC, id DESC LIMIT $1 OFFSET $2"#
        ))
        .bind(page.limit)
        .bind(page.offset)
        .fetch_all(&self.pool)
        .await?;
        let Some(total) = rows.first().map(|row| row.7) else {
            let (total,): (i64,) = sqlx::query_as(r#"SELECT count(*) FROM "catalog""#)
                .fetch_one(&self.pool)
                .await?;
            return Ok((Vec::new(), total));
        };
        let entries = rows
            .into_iter()
            .map(|(a, b, c, d, e, f, g, _)| entry((a, b, c, d, e, f, g)))
            .collect();
        Ok((entries, total))
    }

    async fn ping(&self, within: Duration) -> bool {
        service_basics::postgres::ping(&self.pool, within)
            .await
            .is_ok()
    }
}
