// SPDX-License-Identifier: MIT

//! `catalog` in a throwaway Postgres, migrated as the service migrates it,
//! with the Scala `CatalogStore`'s SQL.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::time::Duration;

use catalog_service::domain::{CatalogDetails, CatalogId, NewCatalog, Page};
use catalog_service::store::Catalogs;
use support::Postgres;
use uuid::Uuid;

fn id(n: u128) -> CatalogId {
    Uuid::from_u128(n).into()
}

fn new(name: &str, sku: &str) -> NewCatalog {
    NewCatalog::parse(name, "a description", 1299, sku).unwrap()
}

#[tokio::test]
async fn an_entry_is_created_read_updated_and_deleted() {
    let postgres = Postgres::start();
    let store = support::store(&postgres).await;
    let created = store.create(id(1), &new("Mug", "SKU-1")).await.unwrap();
    assert_eq!(
        (
            created.name.as_str(),
            created.sku.as_str(),
            created.price_cents
        ),
        ("Mug", "SKU-1", 1299)
    );
    assert_eq!(
        created.created_at, created.updated_at,
        "Postgres stamps both"
    );
    assert_eq!(store.get(id(1)).await.unwrap(), Some(created.clone()));
    let details = CatalogDetails::parse("Mug v2", "", 0).unwrap();
    let updated = store.update(id(1), &details).await.unwrap().unwrap();
    assert_eq!(
        (
            updated.name.as_str(),
            updated.description.as_str(),
            updated.price_cents
        ),
        ("Mug v2", "", 0)
    );
    assert_eq!(
        (updated.sku.as_str(), updated.created_at),
        ("SKU-1", created.created_at)
    );
    assert!(updated.updated_at > created.updated_at);
    assert_eq!(store.update(id(2), &details).await.unwrap(), None);
    assert!(store.delete(id(1)).await.unwrap());
    assert!(!store.delete(id(1)).await.unwrap());
    assert_eq!(store.get(id(1)).await.unwrap(), None);
}

#[tokio::test]
async fn the_list_is_newest_first_with_the_total_even_past_the_end() {
    let postgres = Postgres::start();
    let store = support::store(&postgres).await;
    assert_eq!(
        store.list(Page::from(None, None).unwrap()).await.unwrap(),
        (Vec::new(), 0)
    );
    for n in 1..=3 {
        store
            .create(id(n), &new(&format!("Entry {n}"), "SKU"))
            .await
            .unwrap();
    }
    let (all, total) = store.list(Page::from(None, None).unwrap()).await.unwrap();
    assert_eq!(total, 3);
    assert_eq!(
        all.iter().map(|e| e.id).collect::<Vec<_>>(),
        [id(3), id(2), id(1)]
    );
    let (second, total) = store
        .list(Page::from(Some(1), Some(1)).unwrap())
        .await
        .unwrap();
    assert_eq!(
        (second.iter().map(|e| e.id).collect::<Vec<_>>(), total),
        (vec![id(2)], 3)
    );
    let (past, total) = store
        .list(Page::from(Some(5), Some(10)).unwrap())
        .await
        .unwrap();
    assert_eq!((past.len(), total), (0, 3));
}

#[tokio::test]
async fn the_ping_follows_postgres() {
    let postgres = Postgres::start();
    let store = support::store(&postgres).await;
    assert!(store.ping(Duration::from_secs(2)).await);
    postgres.stop();
    assert!(!store.ping(Duration::from_secs(2)).await);
    assert!(store.get(id(1)).await.is_err());
}
