// SPDX-License-Identifier: MIT

//! The catalogs routes, as the Scala `CatalogRoutes`: the path, query and body
//! decoded first (tapir's plain-text 400), then the rules (400 with the JSON
//! reason), then the entry (404). The list is cache-aside over [`ListCache`];
//! a single entry is read from Postgres. A store failure is a 500.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderName, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use service_basics::jvm::java_instant;
use service_basics::text_error::TextError;
use uuid::Uuid;

use crate::cache::Entry;
use crate::domain::{Catalog, CatalogDetails, CatalogId, InvalidCatalog, NewCatalog, Page};
use crate::server::AppState;
use crate::store::{Catalogs, StoreError};

/// A catalog entry as stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CatalogResponse {
    /// Catalog id (UUID).
    pub id: String,
    /// Product name.
    pub name: String,
    /// What the product is.
    pub description: String,
    /// Price in cents.
    pub price_cents: i32,
    /// Stock-keeping unit.
    pub sku: String,
    /// When the entry was created (UTC).
    pub created_at: String,
    /// When the entry last changed (UTC).
    pub updated_at: String,
}

impl From<Catalog> for CatalogResponse {
    fn from(entry: Catalog) -> Self {
        Self {
            id: entry.id.to_string(),
            name: entry.name,
            description: entry.description,
            price_cents: entry.price_cents,
            sku: entry.sku,
            created_at: java_instant(entry.created_at),
            updated_at: java_instant(entry.updated_at),
        }
    }
}

/// A new catalog entry.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateCatalogRequest {
    /// Product name.
    pub name: String,
    /// What the product is.
    pub description: String,
    /// Price in cents, at least 0.
    pub price_cents: i32,
    /// Stock-keeping unit.
    pub sku: String,
}

/// Fields to update; `sku` is immutable.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCatalogRequest {
    /// Product name.
    pub name: String,
    /// What the product is.
    pub description: String,
    /// Price in cents, at least 0.
    pub price_cents: i32,
}

/// `GET /catalogs`'s query, decoded as numbers by [`list`] (tapir's 400
/// names the parameter that isn't one).
#[derive(Debug, Default, Deserialize)]
pub struct ListQuery {
    /// Maximum entries to return.
    pub limit: Option<String>,
    /// Entries to skip.
    pub offset: Option<String>,
}

/// Error body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub struct ErrorResponse {
    /// Human-readable reason.
    pub error: String,
}

/// Why a route didn't answer its success.
#[derive(Debug)]
pub enum RouteError {
    /// tapir's text 400: what didn't decode (`path parameter id`, `body (…)`).
    Decode(String),
    /// A rule's 400, or 404, with its JSON reason.
    Refused(StatusCode, String),
    /// Postgres failed.
    Store(StoreError),
}

impl From<StoreError> for RouteError {
    fn from(error: StoreError) -> Self {
        Self::Store(error)
    }
}

impl From<InvalidCatalog> for RouteError {
    fn from(error: InvalidCatalog) -> Self {
        Self::Refused(StatusCode::BAD_REQUEST, error.0)
    }
}

impl IntoResponse for RouteError {
    fn into_response(self) -> Response {
        match self {
            Self::Decode(what) => text(
                StatusCode::BAD_REQUEST,
                format!("Invalid value for: {what}"),
            ),
            Self::Refused(status, error) => (status, Json(ErrorResponse { error })).into_response(),
            Self::Store(_) => text(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error".to_owned(),
            ),
        }
    }
}

fn text(status: StatusCode, message: String) -> Response {
    TextError { status, message }.into_response()
}

fn not_found() -> RouteError {
    RouteError::Refused(StatusCode::NOT_FOUND, "Catalog not found".to_owned())
}

fn catalog_id(raw: &str) -> Result<CatalogId, RouteError> {
    raw.parse()
        .map_err(|_| RouteError::Decode("path parameter id".to_owned()))
}

fn body<T: serde::de::DeserializeOwned>(bytes: &Bytes) -> Result<T, RouteError> {
    serde_json::from_slice(bytes).map_err(|error| RouteError::Decode(format!("body ({error})")))
}

fn query_int(raw: Option<&str>, name: &str) -> Result<Option<i32>, RouteError> {
    raw.map(|raw| {
        raw.parse::<i32>()
            .map_err(|_| RouteError::Decode(format!("query parameter {name}")))
    })
    .transpose()
}

/// Logs a failure the way the Scala routes do, and passes it on.
fn logged<T, S>(state: &AppState<S>, result: Result<T, RouteError>) -> Result<T, RouteError> {
    match &result {
        Err(RouteError::Store(error)) => state
            .logger
            .error(&[("error", &error.to_string())], "Request failed"),
        Err(RouteError::Refused(_, reason)) => {
            state.logger.warn(&[("reason", reason)], "Request refused");
        }
        _ => {}
    }
    result
}

/// `POST /catalogs` (201).
///
/// # Errors
/// An undecodable body or a broken rule (400); a store failure (500).
pub async fn create<S: Catalogs>(
    State(state): State<AppState<S>>,
    bytes: Bytes,
) -> Result<(StatusCode, Json<CatalogResponse>), RouteError> {
    let request: CreateCatalogRequest = body(&bytes)?;
    let result = async {
        let new = NewCatalog::parse(
            &request.name,
            &request.description,
            request.price_cents,
            &request.sku,
        )?;
        Ok(state
            .store
            .create(CatalogId::from(Uuid::new_v4()), &new)
            .await?)
    }
    .await;
    let entry = logged(&state, result)?;
    Ok((StatusCode::CREATED, Json(entry.into())))
}

/// `GET /catalogs/{id}`, from Postgres.
///
/// # Errors
/// A malformed id (400), no such entry (404), a store failure (500).
pub async fn get<S: Catalogs>(
    State(state): State<AppState<S>>,
    Path(raw): Path<String>,
) -> Result<Json<CatalogResponse>, RouteError> {
    let id = catalog_id(&raw)?;
    let result = state
        .store
        .get(id)
        .await
        .map_err(RouteError::from)
        .and_then(|e| e.ok_or_else(not_found));
    Ok(Json(logged(&state, result)?.into()))
}

/// `PATCH` and `PUT /catalogs/{id}`: the body judged before the lookup.
///
/// # Errors
/// A malformed id or body, a broken rule (400), no such entry (404), a store
/// failure (500).
pub async fn update<S: Catalogs>(
    State(state): State<AppState<S>>,
    Path(raw): Path<String>,
    bytes: Bytes,
) -> Result<Json<CatalogResponse>, RouteError> {
    let id = catalog_id(&raw)?;
    let request: UpdateCatalogRequest = body(&bytes)?;
    let result = async {
        let details =
            CatalogDetails::parse(&request.name, &request.description, request.price_cents)?;
        state
            .store
            .update(id, &details)
            .await?
            .ok_or_else(not_found)
    }
    .await;
    Ok(Json(logged(&state, result)?.into()))
}

/// `DELETE /catalogs/{id}` (204).
///
/// # Errors
/// A malformed id (400), no such entry (404), a store failure (500).
pub async fn delete<S: Catalogs>(
    State(state): State<AppState<S>>,
    Path(raw): Path<String>,
) -> Result<StatusCode, RouteError> {
    let id = catalog_id(&raw)?;
    let result = match state.store.delete(id).await {
        Ok(true) => Ok(StatusCode::NO_CONTENT),
        Ok(false) => Err(not_found()),
        Err(error) => Err(error.into()),
    };
    logged(&state, result)
}

/// `GET /catalogs?limit&offset`: one page and `X-Total-Count`, cache-aside.
///
/// # Errors
/// A non-numeric limit or offset, or one out of bounds (400); a store failure
/// on a miss (500).
pub async fn list<S: Catalogs>(
    State(state): State<AppState<S>>,
    Query(query): Query<ListQuery>,
) -> Result<Response, RouteError> {
    let limit = query_int(query.limit.as_deref(), "limit")?;
    let offset = query_int(query.offset.as_deref(), "offset")?;
    let page = Page::from(limit, offset)
        .map_err(|error| RouteError::Refused(StatusCode::BAD_REQUEST, error.to_string()));
    let page = logged(&state, page)?;
    let entry = if let Some(cached) = state.cache.get(page).await {
        cached
    } else {
        let (entries, total) = logged(
            &state,
            state.store.list(page).await.map_err(RouteError::from),
        )?;
        let fresh = Entry {
            catalogs: entries.into_iter().map(CatalogResponse::from).collect(),
            total,
        };
        if let Err(error) = state.cache.set(page, &fresh).await {
            state.logger.warn(
                &[("error", &error)],
                "Catalog-list cache write failed - response unaffected",
            );
        }
        fresh
    };
    let total = HeaderName::from_static("x-total-count");
    Ok(([(total, entry.total.to_string())], Json(entry.catalogs)).into_response())
}
