// SPDX-License-Identifier: MIT

//! The service's API description: what tapir generated from the Scala
//! endpoints (capture 20261122 openapi.yaml), stated once here next to the
//! DTO types it derives the schemas from, served with Swagger UI
//! (CATALOG-D1); and an AsyncAPI document with no channels, since the service
//! publishes and consumes nothing (CATALOG-D2; the Scala service serves none,
//! decision A6).

use axum::Router;
use serde_json::{Value, json};
use telemetry::docs::openapi::{Api, Operation};
use telemetry::docs::{asyncapi, swagger};

use crate::routes::{CatalogResponse, CreateCatalogRequest, ErrorResponse, UpdateCatalogRequest};

const ID: &str = "3f2b8c1e-7a4d-4e9b-9c2a-1d5e6f7a8b90";
const ID_DESCRIPTION: &str = "Catalog id (UUID). A malformed id returns 400.";
const RULE: &str = "The body breaks a catalog rule: blank `name`/`sku` or `priceCents` < 0.";

fn example() -> Value {
    json!({
        "id": ID,
        "name": "Rolex Submariner",
        "description": "Iconic diver's watch, 41mm stainless steel case",
        "priceCents": 950_000,
        "sku": "watch-rolex-submariner",
        "createdAt": "2026-11-01T09:30:00Z",
        "updatedAt": "2026-11-01T09:45:00Z",
    })
}

/// A JSON body: its schema component and example. (Always `Some`: the
/// builder takes `None` for a response without one.)
#[allow(clippy::unnecessary_wraps)]
fn entry() -> Option<(&'static str, Value)> {
    Some(("CatalogResponse", example()))
}

#[allow(clippy::unnecessary_wraps)]
fn error(message: &str) -> Option<(&'static str, Value)> {
    Some(("ErrorResponse", json!({ "error": message })))
}

fn update_body() -> Value {
    json!({
        "name": "Rolex Submariner",
        "description": "Iconic diver's watch, 41mm stainless steel case",
        "priceCents": 950_000,
    })
}

fn by_id(operation: Operation) -> Operation {
    operation
        .tag("catalogs")
        .path_parameter("id", ID_DESCRIPTION, json!(ID))
}

fn not_found(operation: Operation) -> Operation {
    operation.response(404, "No catalog with this id.", error("Catalog not found"))
}

fn update(operation: Operation, summary: &str, description: &str) -> Operation {
    not_found(
        by_id(operation)
            .summary(summary)
            .description(description)
            .request_body("UpdateCatalogRequest", update_body())
            .response(200, "", entry())
            .response(400, RULE, error("priceCents must be at least 0, got -1")),
    )
}

/// The OpenAPI document: the six catalogs operations, then the two health checks.
#[must_use]
pub fn api() -> Api {
    Api::new("Catalog Service", "1.0")
        .schema::<CatalogResponse>()
        .schema::<CreateCatalogRequest>()
        .schema::<ErrorResponse>()
        .schema::<UpdateCatalogRequest>()
        .operation(
            Operation::post("/catalogs")
                .tag("catalogs")
                .summary("Create a catalog entry")
                .description(
                    "Adds a product. Blank `name`/`sku` or `priceCents` < 0 → 400. Not idempotent \
                     (new id each call; SKU uniqueness not enforced). Cached list pages are not \
                     invalidated.",
                )
                .request_body(
                    "CreateCatalogRequest",
                    json!({
                        "name": "Rolex Submariner",
                        "description": "Iconic diver's watch, 41mm stainless steel case",
                        "priceCents": 950_000,
                        "sku": "watch-rolex-submariner",
                    }),
                )
                .response(201, "", entry())
                .response(400, RULE, error("priceCents must be at least 0, got -1")),
        )
        .operation(not_found(
            by_id(Operation::get("/catalogs/{id}"))
                .summary("Get a catalog entry")
                .description("Read from Postgres (not cached). 404 unknown id; 400 malformed id.")
                .response(200, "", entry()),
        ))
        .operation(update(
            Operation::patch("/catalogs/{id}"),
            "Update a catalog entry",
            "Sets name/description/priceCents (all required; `sku` immutable). Safe to repeat \
             (only `updatedAt` advances). Body validated before lookup: 400, then 404.",
        ))
        .operation(update(
            Operation::put("/catalogs/{id}"),
            "Update a catalog entry (replace)",
            "Same as PATCH.",
        ))
        .operation(not_found(
            by_id(Operation::delete("/catalogs/{id}"))
                .summary("Delete a catalog entry")
                .description("Hard delete; repeating returns 404.")
                .response(204, "", None),
        ))
        .operation(
            Operation::get("/catalogs")
                .tag("catalogs")
                .summary("List catalog entries")
                .description(
                    "One page, newest first (ties by id); the `X-Total-Count` header is the true \
                     total. Redis-cached per (limit, offset) with a 60s default TTL and no write \
                     invalidation; falls back to Postgres if Redis fails. Bad limit/offset → 400.",
                )
                .query_parameter(
                    "limit",
                    "Maximum entries to return (1–100, default 20).",
                    json!(20),
                )
                .query_parameter("offset", "Entries to skip (default 0).", json!(0))
                .response(200, "", Some(("CatalogResponse[]", json!([example()]))))
                .response(
                    400,
                    "`limit` outside 1–100 or `offset` < 0.",
                    error("Invalid pagination parameters"),
                ),
        )
        .operation(
            Operation::get("/health")
                .tag("health")
                .summary("Liveness check")
                .description("200 while the process is up. Checks no dependencies.")
                .response(200, "", None),
        )
        .operation(
            Operation::get("/health/ready")
                .tag("health")
                .summary("Readiness check")
                .description("200 when Postgres answers a ping, else 503 (Redis is not checked).")
                .response(200, "", None)
                .response(503, "Postgres didn't answer a ping.", None),
        )
}

/// The AsyncAPI document: no channels.
#[must_use]
pub fn asyncapi() -> Value {
    asyncapi::document("Catalog Service", "1.0", &[])
}

/// Swagger UI at `/docs/` (`/docs/docs.yaml`), and the AsyncAPI document and page.
pub fn router() -> Router {
    swagger::router(api().to_yaml()).merge(asyncapi::router_with_page(&asyncapi()))
}
