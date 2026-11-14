//! Routes and the middleware stack.

mod admin;
mod customer;
pub mod extract;
mod mcp;
mod payments;
mod utility;
mod worker;

use axum::Router;
use axum::http::{HeaderName, HeaderValue, Method, header};
use axum::routing::{get, post};
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::trace::TraceLayer;

use crate::{AppState, Config};
use extract::{ACTOR_HEADER, TICKET_TOKEN_HEADER};

use axum::{
    extract::{Request, State},
    http::{StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};

async fn require_bearer(
    State(expected_token): State<String>,
    request: Request,
    next: Next,
) -> Response {
    // Public API routes.
    if !request.uri().path().eq("/api/ticket") {
        return next.run(request).await;
    }

    let token = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));

    let authorized = token
        .map(|token| {
            bool::from(token.as_bytes().eq(expected_token.as_bytes()))
        })
        .unwrap_or(false);

    if !authorized {
        return (
            StatusCode::UNAUTHORIZED,
            [(header::WWW_AUTHENTICATE, "Bearer")],
            "Unauthorized",
        )
            .into_response();
    }

    next.run(request).await
}

pub fn router(state: AppState) -> Router {
    let api = Router::new()
        // customers
        .route("/tickets", post(customer::create_ticket).get(worker::list_tickets))
        .route("/public/tickets/{id}", get(customer::get_ticket))
        .route("/public/tickets/{id}/messages", post(customer::reply))
        // workers
        .route("/tickets/{id}", get(worker::get_ticket).patch(worker::update_ticket))
        .route("/tickets/{id}/messages", post(worker::add_message))
        .route("/tickets/{id}/yeet", post(worker::yeet_ticket))
        .route("/stats/backlog", get(worker::backlog_stats))
        // admin
        .route("/admin/export", get(admin::export))
        // payments
        .route("/checkout", post(payments::checkout))
        // utility
        .route("/healthz", get(utility::healthz))
        .route("/teapot", get(utility::teapot));

    let config = state.config.clone();
    let app = Router::new().nest("/api", api).route("/mcp", post(mcp::post).get(mcp::get));
    middleware(app.fallback(utility::not_found), &config).with_state(state)
}

/// The one place for cross-cutting layers. Tracing exporters, rate limiting,
/// auth middleware and CORS hardening go here. Layers run bottom to top.
///
/// Routes outside `/api` (an MCP server at `/mcp`, say) go next to the `nest` in
/// `router`, so they pass through here too.
pub fn middleware<S: Clone + Send + Sync + 'static>(router: Router<S>, config: &Config) -> Router<S> {
    let trace = TraceLayer::new_for_http().make_span_with(crate::telemetry::request_span).on_response(
        |response: &axum::http::Response<_>, latency: std::time::Duration, span: &tracing::Span| {
            span.record("http.response.status_code", response.status().as_u16());
            tracing::debug!(status = response.status().as_u16(), ?latency, "response");
        },
    );
    router.layer(cors(config)).layer(trace)
    .layer(
        axum::middleware::from_fn_with_state(
            config.admin_token.clone(),
            require_bearer,
        ))
}

/// CORS for the web app on another origin (Vite in dev, Tauri). `Authorization` is
/// allowed, so a bearer token works as it is. Session cookies need more: add
/// `.allow_credentials(true)` here and `credentials: "include"` to the `fetch` in
/// `apps/web/src/api/client.ts`. Behind one origin in production, none of this applies.
fn cors(config: &Config) -> CorsLayer {
    let origins: Vec<HeaderValue> = config
        .cors_allowed_origins
        .iter()
        .filter_map(|origin| match origin.parse() {
            Ok(value) => Some(value),
            Err(_) => {
                tracing::warn!(%origin, "ignoring invalid CORS origin");
                None
            }
        })
        .collect();
    CorsLayer::new()
        .allow_origin(AllowOrigin::list(origins))
        .allow_methods([Method::GET, Method::POST, Method::PATCH])
        .allow_headers([
            header::CONTENT_TYPE,
            header::AUTHORIZATION,
            HeaderName::from_static(ACTOR_HEADER),
            HeaderName::from_static(TICKET_TOKEN_HEADER),
        ])
}
