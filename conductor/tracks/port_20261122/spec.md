# Spec — port catalog-service (gluon all-Rust Stage A, GREEN)

## Overview
The Scala catalog-service in idiomatic Rust on the template's shape: the `catalog`
table (its Flyway V1, verbatim), `POST`/`GET /catalogs`, `GET`/`PATCH`/`PUT`/`DELETE
/catalogs/{id}`, the list cached per page in Redis as the Scala service caches it
(`catalog-list:<limit>:<offset>`, `{"catalogs","total"}`, 60 s, no write
invalidation), readiness on Postgres only, CORS exposing `X-Total-Count`, the docs.

## Functional requirements
CATALOG-F1–F10, CATALOG-D1–D2 (tests/contract `docs/behaviours.md`). Redis down: the
list falls back to Postgres within the cache timeout (decision A10).

## Acceptance criteria
`scripts/ci.sh` green, ≥90% lines; `bin/verify-port catalog-service` all MET; the
cache check seen red against a build that doesn't cache.

## Out of scope
Wire rows: metrics (`catalog.cache.requests`, db durations), log messages.
