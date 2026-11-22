// SPDX-License-Identifier: MIT

//! The catalog-service binary: the Rust port of gluon's Scala
//! catalog-service (ADR 0029). Generated from `tools/rust-service-template`:
//! the catalogue in Postgres, its list pages cached in Redis, six routes,
//! health, readiness on Postgres, and the docs.
pub mod cache;
pub mod config;
pub mod db;
pub mod docs;
pub mod domain;
pub mod routes;
pub mod server;
pub mod store;

/// The package version, as built.
#[must_use]
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
