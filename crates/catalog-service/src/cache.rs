// SPDX-License-Identifier: MIT

//! The list cache, as the Scala `CatalogListCache`: each page at
//! `catalog-list:<limit>:<offset>` as `{"catalogs":[…],"total":N}`, for the
//! list TTL (60 s), with no invalidation on write (TTL-only freshness, US-1.2).
//! Either implementation reads the other's entries. Redis is never the
//! source of truth: down, slow, or holding what doesn't parse, it is a miss.

use std::sync::Arc;
use std::time::Duration;

use redis_cache::Cache;
use serde::{Deserialize, Serialize};

use crate::domain::Page;
use crate::routes::CatalogResponse;

/// How long a cache call may take before Postgres answers instead.
pub const TIMEOUT: Duration = Duration::from_millis(500);

/// The key a page is cached at.
#[must_use]
pub fn key(page: Page) -> String {
    format!("catalog-list:{}:{}", page.limit, page.offset)
}

/// A cached page (the Scala `CatalogListCacheEntry`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// The page's entries, as answered.
    pub catalogs: Vec<CatalogResponse>,
    /// The true total.
    pub total: i64,
}

/// The pages in Redis.
#[derive(Debug, Clone)]
pub struct ListCache {
    redis: Arc<Cache>,
    ttl: Duration,
}

impl ListCache {
    /// Over `redis`, each page standing for `ttl`.
    #[must_use]
    pub fn new(redis: Arc<Cache>, ttl: Duration) -> Self {
        Self { redis, ttl }
    }

    /// The cached page, or `None` on a miss — including Redis down or an
    /// entry that doesn't parse.
    pub async fn get(&self, page: Page) -> Option<Entry> {
        let cached = self.redis.get(&key(page)).await.ok()??;
        serde_json::from_str(&cached).ok()
    }

    /// Caches `entry` for `page`.
    ///
    /// # Errors
    /// The cache's (the caller logs it; the answer is unaffected).
    pub async fn set(&self, page: Page, entry: &Entry) -> Result<(), String> {
        let json = serde_json::to_string(entry).map_err(|error| error.to_string())?;
        self.redis
            .set(&key(page), &json, self.ttl)
            .await
            .map_err(|error| error.to_string())
    }
}
