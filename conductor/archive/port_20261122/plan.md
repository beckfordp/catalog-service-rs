# Plan — port catalog-service (gluon all-Rust Stage A, GREEN)

Spec: [spec.md](./spec.md).

## Phase 1: Port
- [x] Task 1.1: domain (id, rules, page) — tests first `8924fad`
- [x] Task 1.2: Postgres store (the Scala SQL) — tests first (throwaway Postgres) `8924fad`
- [x] Task 1.3: list cache (the Scala keys and format) — tests first (throwaway Redis) `8924fad`
- [x] Task 1.4: routes and errors; docs — tests first (in-memory store) `8924fad`
- [x] Task: Conductor - User Manual Verification 'Phase 1' (Protocol in workflow.md): gluon `bin/verify-port catalog-service` all MET (evidence `docs/ports/evidence/catalog-service/green.txt`); CI 96.89% lines; cache proof: with `ListCache::get` never hitting, both CATALOG-F8 checks red ("a cached page stands until it goes", `cache-broken.txt`), restored

## Archive (2026-11-22)

Reviewed: no High/Medium. Deliberate differences: Redis down, the list falls back to
Postgres within 500 ms where Scala hangs (A10); AsyncAPI served (A6). Ids are
`Uuid::new_v4` at the route (the template's `IdGen`/`Clock` dropped: Postgres stamps
the times, as in Scala).
