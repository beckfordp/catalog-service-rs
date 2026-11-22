// SPDX-License-Identifier: MIT

//! The pure core, as the Scala domain (`CatalogId`, `CatalogRules`, `Page`):
//! values parsed at the edge, so the rest of the service holds only valid ones.

use std::fmt;
use std::str::FromStr;

use time::OffsetDateTime;
use uuid::Uuid;

/// A catalogue entry's id: a UUID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CatalogId(Uuid);

impl CatalogId {
    /// The UUID.
    #[must_use]
    pub fn uuid(self) -> Uuid {
        self.0
    }
}

impl From<Uuid> for CatalogId {
    fn from(id: Uuid) -> Self {
        Self(id)
    }
}

impl FromStr for CatalogId {
    type Err = uuid::Error;

    fn from_str(text: &str) -> Result<Self, uuid::Error> {
        Uuid::parse_str(text).map(Self)
    }
}

impl fmt::Display for CatalogId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// A catalogue entry, as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Catalog {
    /// Its id.
    pub id: CatalogId,
    /// Display name.
    pub name: String,
    /// Description (may be empty).
    pub description: String,
    /// Price in cents, at least 0.
    pub price_cents: i32,
    /// Stock-keeping unit (not unique).
    pub sku: String,
    /// When it was created (Postgres `now()`).
    pub created_at: OffsetDateTime,
    /// When it last changed.
    pub updated_at: OffsetDateTime,
}

/// Why an entry was refused; the reason is the error body's.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct InvalidCatalog(pub String);

/// The details an update sets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogDetails {
    /// Non-blank.
    pub name: String,
    /// Any text.
    pub description: String,
    /// At least 0.
    pub price_cents: i32,
}

/// A new entry: its details and a non-blank sku.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewCatalog {
    /// Name, description, price.
    pub details: CatalogDetails,
    /// Non-blank.
    pub sku: String,
}

/// Java's `String.isBlank`: empty or only whitespace.
fn blank(text: &str) -> bool {
    text.trim().is_empty()
}

impl CatalogDetails {
    /// The details, when the name isn't blank and the price isn't negative.
    ///
    /// # Errors
    /// [`InvalidCatalog`] with the Scala service's reason.
    pub fn parse(name: &str, description: &str, price_cents: i32) -> Result<Self, InvalidCatalog> {
        if blank(name) {
            return Err(InvalidCatalog("name must not be blank".to_owned()));
        }
        if price_cents < 0 {
            return Err(InvalidCatalog(format!(
                "priceCents must be at least 0, got {price_cents}"
            )));
        }
        Ok(Self {
            name: name.to_owned(),
            description: description.to_owned(),
            price_cents,
        })
    }
}

impl NewCatalog {
    /// The entry, when its details are valid and the sku isn't blank.
    ///
    /// # Errors
    /// [`InvalidCatalog`]: the details' reason first, then the sku's.
    pub fn parse(
        name: &str,
        description: &str,
        price_cents: i32,
        sku: &str,
    ) -> Result<Self, InvalidCatalog> {
        let details = CatalogDetails::parse(name, description, price_cents)?;
        if blank(sku) {
            return Err(InvalidCatalog("sku must not be blank".to_owned()));
        }
        Ok(Self {
            details,
            sku: sku.to_owned(),
        })
    }
}

/// One page of the list: limit 1–100 (default 20), offset ≥ 0 (default 0).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Page {
    /// Entries to return.
    pub limit: i32,
    /// Entries to skip.
    pub offset: i32,
}

/// A limit or offset outside its bounds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("Invalid pagination parameters")]
pub struct InvalidPagination;

impl Page {
    /// The default limit.
    pub const DEFAULT_LIMIT: i32 = 20;
    /// The largest limit.
    pub const MAX_LIMIT: i32 = 100;

    /// The page `limit`/`offset` ask for, defaults filled in.
    ///
    /// # Errors
    /// [`InvalidPagination`].
    pub fn from(limit: Option<i32>, offset: Option<i32>) -> Result<Self, InvalidPagination> {
        let limit = limit.unwrap_or(Self::DEFAULT_LIMIT);
        let offset = offset.unwrap_or(0);
        if (1..=Self::MAX_LIMIT).contains(&limit) && offset >= 0 {
            Ok(Self { limit, offset })
        } else {
            Err(InvalidPagination)
        }
    }
}
