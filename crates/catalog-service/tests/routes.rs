// SPDX-License-Identifier: MIT

//! The routes over a store in memory and a list cache on a throwaway Redis:
//! the order of judgement (decode, rules, lookup), the error bodies, the list
//! cache-aside (TTL-only, the Scala keys and format), Redis down a miss, a
//! store failure a 500, and the docs.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use catalog_service::cache::ListCache;
use catalog_service::server::{AppState, app};
use redis_cache::Cache;
use serde_json::{Value, json};
use service_testing::Redis;
use support::Memory;
use telemetry::logging::Logger;
use tower::ServiceExt;

const MISSING: &str = "/catalogs/00000000-0000-4000-8000-000000000000";

fn service(store: Memory, redis_url: &str) -> Router {
    let cache = Arc::new(Cache::new(redis_url, Duration::from_millis(500)).unwrap());
    app(AppState {
        store,
        cache: ListCache::new(cache, Duration::from_mins(1)),
        logger: Logger::stdout("catalog-service-test"),
    })
}

/// Nothing listens there: every cache call is a miss.
const NO_REDIS: &str = "redis://127.0.0.1:1";

struct Answer {
    status: StatusCode,
    total: Option<String>,
    body: String,
}

impl Answer {
    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap()
    }
}

async fn call(app: &Router, method: &str, path: &str, body: Option<Value>) -> Answer {
    let request = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json")
        .body(body.map_or_else(Body::empty, |b| Body::from(b.to_string())))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let total = response
        .headers()
        .get("x-total-count")
        .map(|v| v.to_str().unwrap().to_owned());
    let bytes = to_bytes(response.into_body(), 1 << 20).await.unwrap();
    Answer {
        status,
        total,
        body: String::from_utf8(bytes.to_vec()).unwrap(),
    }
}

fn entry(name: &str) -> Value {
    json!({"name": name, "description": "d", "priceCents": 5, "sku": "SKU-1"})
}

#[tokio::test]
async fn an_entry_is_created_read_updated_and_deleted() {
    let app = service(Memory::default(), NO_REDIS);
    let created = call(&app, "POST", "/catalogs", Some(entry("Mug"))).await;
    assert_eq!(created.status, StatusCode::CREATED);
    let body = created.json();
    assert_eq!(
        (body["name"].clone(), body["priceCents"].clone()),
        (json!("Mug"), json!(5))
    );
    assert_eq!(body["createdAt"], body["updatedAt"]);
    let path = format!("/catalogs/{}", body["id"].as_str().unwrap());
    assert_eq!(call(&app, "GET", &path, None).await.json(), body);
    for method in ["PATCH", "PUT"] {
        let update = json!({"name": method, "description": "", "priceCents": 0});
        let updated = call(&app, method, &path, Some(update)).await;
        assert_eq!(updated.status, StatusCode::OK, "{method}");
        assert_eq!(
            (
                updated.json()["name"].clone(),
                updated.json()["sku"].clone()
            ),
            (json!(method), json!("SKU-1"))
        );
    }
    assert_eq!(
        call(&app, "DELETE", &path, None).await.status,
        StatusCode::NO_CONTENT
    );
    let gone = call(&app, "GET", &path, None).await;
    assert_eq!(
        (gone.status, gone.json()),
        (StatusCode::NOT_FOUND, json!({"error": "Catalog not found"}))
    );
    assert_eq!(
        call(&app, "DELETE", &path, None).await.status,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn decoding_then_the_rules_then_the_lookup_is_judged() {
    let app = service(Memory::default(), NO_REDIS);
    let text = |answer: &Answer| (answer.status, answer.body.clone());
    let id = (
        StatusCode::BAD_REQUEST,
        "Invalid value for: path parameter id".to_owned(),
    );
    for method in ["GET", "DELETE", "PATCH", "PUT"] {
        assert_eq!(
            text(&call(&app, method, "/catalogs/nope", Some(json!({}))).await),
            id,
            "{method}"
        );
    }
    let undecodable = call(&app, "POST", "/catalogs", Some(json!({"name": "n"}))).await;
    assert_eq!(undecodable.status, StatusCode::BAD_REQUEST);
    assert!(
        undecodable.body.starts_with("Invalid value for: body ("),
        "{}",
        undecodable.body
    );
    assert_eq!(
        call(&app, "PATCH", MISSING, Some(json!({"name": "n"})))
            .await
            .status,
        StatusCode::BAD_REQUEST
    );
    let refused = call(
        &app,
        "POST",
        "/catalogs",
        Some(json!({"name": "n", "description": "d", "priceCents": -1, "sku": "S"})),
    )
    .await;
    assert_eq!(
        (refused.status, refused.json()),
        (
            StatusCode::BAD_REQUEST,
            json!({"error": "priceCents must be at least 0, got -1"})
        )
    );
    let invalid_first = call(
        &app,
        "PUT",
        MISSING,
        Some(json!({"name": "", "description": "d", "priceCents": 1})),
    )
    .await;
    assert_eq!(
        invalid_first.json(),
        json!({"error": "name must not be blank"})
    );
    let missing = call(
        &app,
        "PATCH",
        MISSING,
        Some(json!({"name": "n", "description": "d", "priceCents": 1})),
    )
    .await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_list_pages_newest_first_with_the_total() {
    let app = service(Memory::default(), NO_REDIS);
    let empty = call(&app, "GET", "/catalogs", None).await;
    assert_eq!(
        (empty.status, empty.total.as_deref(), empty.json()),
        (StatusCode::OK, Some("0"), json!([]))
    );
    let mut ids = Vec::new();
    for n in 0..3 {
        ids.push(
            call(&app, "POST", "/catalogs", Some(entry(&format!("E{n}"))))
                .await
                .json()["id"]
                .clone(),
        );
    }
    let all = call(&app, "GET", "/catalogs", None).await;
    assert_eq!(all.total.as_deref(), Some("3"));
    assert_eq!(
        all.json()
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["id"].clone())
            .collect::<Vec<_>>(),
        [ids[2].clone(), ids[1].clone(), ids[0].clone()]
    );
    let page = call(&app, "GET", "/catalogs?limit=1&offset=1", None).await;
    assert_eq!(
        (page.total.as_deref(), page.json()[0]["id"].clone()),
        (Some("3"), ids[1].clone())
    );
    for query in ["limit=0", "limit=101", "offset=-1"] {
        let bad = call(&app, "GET", &format!("/catalogs?{query}"), None).await;
        assert_eq!(
            (bad.status, bad.json()),
            (
                StatusCode::BAD_REQUEST,
                json!({"error": "Invalid pagination parameters"})
            ),
            "{query}"
        );
    }
    for query in ["limit=lots", "offset=x"] {
        let bad = call(&app, "GET", &format!("/catalogs?{query}"), None).await;
        assert_eq!(bad.status, StatusCode::BAD_REQUEST, "{query}");
        assert!(
            bad.body.starts_with("Invalid value for: query parameter "),
            "{}",
            bad.body
        );
    }
}

#[tokio::test]
async fn a_page_is_cached_until_its_entry_goes_and_reads_the_scala_format() {
    let redis = Redis::start();
    let app = service(Memory::default(), &redis.url());
    let raw = Cache::new(&redis.url(), Duration::from_millis(500)).unwrap();
    call(&app, "POST", "/catalogs", Some(entry("First"))).await;
    let first = call(&app, "GET", "/catalogs", None).await;
    let cached: Value =
        serde_json::from_str(&raw.get("catalog-list:20:0").await.unwrap().unwrap()).unwrap();
    assert_eq!(cached, json!({"catalogs": first.json(), "total": 1}));
    call(&app, "POST", "/catalogs", Some(entry("Second"))).await;
    let stale = call(&app, "GET", "/catalogs", None).await;
    assert_eq!(
        (stale.json(), stale.total.as_deref()),
        (first.json(), Some("1")),
        "TTL-only freshness"
    );
    raw.delete(&["catalog-list:20:0"]).await.unwrap();
    let fresh = call(&app, "GET", "/catalogs", None).await;
    assert_eq!(
        (fresh.json()[0]["name"].clone(), fresh.total.as_deref()),
        (json!("Second"), Some("2"))
    );
    let scala = r#"{"catalogs":[{"id":"x","name":"Scala","description":"","priceCents":0,"sku":"S","createdAt":"2026-11-22T21:42:51.470557Z","updatedAt":"2026-11-22T21:42:51.470557Z"}],"total":42}"#;
    raw.set("catalog-list:3:9", scala, Duration::from_mins(1))
        .await
        .unwrap();
    let served = call(&app, "GET", "/catalogs?limit=3&offset=9", None).await;
    assert_eq!(
        (served.json()[0]["name"].clone(), served.total.as_deref()),
        (json!("Scala"), Some("42"))
    );
    raw.set("catalog-list:4:0", "not json", Duration::from_mins(1))
        .await
        .unwrap();
    let unreadable = call(&app, "GET", "/catalogs?limit=4", None).await;
    assert_eq!(
        unreadable.total.as_deref(),
        Some("2"),
        "an unreadable entry is a miss"
    );
    redis.stop();
    let down = call(&app, "GET", "/catalogs?limit=5", None).await;
    assert_eq!(
        (down.status, down.total.as_deref()),
        (StatusCode::OK, Some("2")),
        "Redis down falls back to the store"
    );
}

#[tokio::test]
async fn a_store_failure_is_a_500_and_readiness_follows_the_store_only() {
    let store = Memory::default();
    let app = service(store.clone(), NO_REDIS);
    assert_eq!(
        call(&app, "GET", "/health/ready", None).await.status,
        StatusCode::OK,
        "Redis is not checked"
    );
    store.failing.store(true, Ordering::SeqCst);
    let update = Some(json!({"name": "n", "description": "d", "priceCents": 1}));
    for (method, path, body) in [
        ("POST", "/catalogs", Some(entry("n"))),
        ("GET", MISSING, None),
        ("PATCH", MISSING, update),
        ("DELETE", MISSING, None),
        ("GET", "/catalogs", None),
    ] {
        let failed = call(&app, method, path, body).await;
        assert_eq!(
            (failed.status, failed.body),
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error".to_owned()
            ),
            "{method} {path}"
        );
    }
    store.down.store(true, Ordering::SeqCst);
    assert_eq!(
        call(&app, "GET", "/health/ready", None).await.status,
        StatusCode::SERVICE_UNAVAILABLE
    );
}

#[tokio::test]
async fn the_docs_and_the_edge() {
    let app = service(Memory::default(), NO_REDIS);
    let yaml = call(&app, "GET", "/docs/docs.yaml", None).await;
    assert_eq!(yaml.status, StatusCode::OK);
    for operation in [
        "getCatalogs",
        "postCatalogs",
        "getCatalogsId",
        "patchCatalogsId",
        "putCatalogsId",
        "deleteCatalogsId",
        "getHealthReady",
    ] {
        assert!(yaml.body.contains(operation), "{operation}");
    }
    assert_eq!(
        call(&app, "GET", "/docs/", None).await.status,
        StatusCode::OK
    );
    assert_eq!(
        call(&app, "GET", "/docs/asyncapi.json", None).await.json()["channels"],
        json!({})
    );
    assert!(
        call(&app, "GET", "/docs/asyncapi/", None)
            .await
            .body
            .contains("publishes and consumes nothing")
    );
    assert_eq!(
        call(&app, "GET", "/nope", None).await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(&app, "POST", MISSING, None).await.status,
        StatusCode::METHOD_NOT_ALLOWED
    );
    let request = Request::builder()
        .method("GET")
        .uri("/catalogs")
        .header("origin", "http://localhost:5173")
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(
        response.headers()["access-control-expose-headers"],
        "X-Total-Count"
    );
}
