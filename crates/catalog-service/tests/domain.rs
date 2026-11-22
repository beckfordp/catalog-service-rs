// SPDX-License-Identifier: MIT

//! The pure core, as the Scala `CatalogRules` and `Page`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use catalog_service::domain::{
    CatalogDetails, CatalogId, InvalidCatalog, InvalidPagination, NewCatalog, Page,
};

fn reason(error: &InvalidCatalog) -> String {
    error.to_string()
}

#[test]
fn an_entry_needs_a_name_a_price_of_at_least_0_and_a_sku() {
    let ok = NewCatalog::parse("Mug", "", 0, "SKU-1").unwrap();
    assert_eq!(
        (
            ok.details.name.as_str(),
            ok.details.price_cents,
            ok.sku.as_str()
        ),
        ("Mug", 0, "SKU-1")
    );
    assert_eq!(
        reason(&NewCatalog::parse(" \t", "d", 1, "S").unwrap_err()),
        "name must not be blank"
    );
    assert_eq!(
        reason(&NewCatalog::parse("n", "d", -1, "S").unwrap_err()),
        "priceCents must be at least 0, got -1"
    );
    assert_eq!(
        reason(&NewCatalog::parse("n", "d", 1, "  ").unwrap_err()),
        "sku must not be blank"
    );
    assert_eq!(
        reason(&NewCatalog::parse("", "d", -5, "").unwrap_err()),
        "name must not be blank",
        "name first"
    );
    assert!(CatalogDetails::parse("n", "", 5).is_ok());
}

#[test]
fn a_page_is_limit_1_to_100_and_offset_at_least_0() {
    assert_eq!(
        Page::from(None, None),
        Ok(Page {
            limit: 20,
            offset: 0
        })
    );
    assert_eq!(
        Page::from(Some(100), Some(7)),
        Ok(Page {
            limit: 100,
            offset: 7
        })
    );
    for (limit, offset) in [(Some(0), None), (Some(101), None), (None, Some(-1))] {
        assert_eq!(Page::from(limit, offset), Err(InvalidPagination));
    }
    assert_eq!(
        InvalidPagination.to_string(),
        "Invalid pagination parameters"
    );
}

#[test]
fn an_id_is_a_uuid() {
    let id: CatalogId = "CC3349FB-88E9-497D-A1B4-0982031A9162".parse().unwrap();
    assert_eq!(id.to_string(), "cc3349fb-88e9-497d-a1b4-0982031a9162");
    assert!("nope".parse::<CatalogId>().is_err());
}
