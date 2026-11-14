//! Stripe Checkout: the card form lives on Stripe, so this server never sees a card number.

use axum::extract::State;
use axum::response::Redirect;

use crate::{ApiError, AppState};

#[derive(serde::Deserialize)]
struct CheckoutSession {
    url: String,
}

/// `POST /api/checkout` (a plain form post): opens a Checkout session for the configured
/// price and sends the browser to Stripe, which brings it back to `/?paid=1`.
pub async fn checkout(State(state): State<AppState>) -> Result<Redirect, ApiError> {
    let config = &state.config;
    let key = config.stripe_api_key.as_deref().ok_or(ApiError::NotFound)?;
    let success_url = format!("{}/?paid=1&session_id={{CHECKOUT_SESSION_ID}}", config.public_web_url);
    let cancel_url = format!("{}/?paid=0", config.public_web_url);
    let response = reqwest::Client::new()
        .post("https://api.stripe.com/v1/checkout/sessions")
        .bearer_auth(key)
        .form(&[
            ("mode", "subscription"),
            ("line_items[0][price]", config.stripe_price_id.as_str()),
            ("line_items[0][quantity]", "1"),
            ("success_url", success_url.as_str()),
            ("cancel_url", cancel_url.as_str()),
        ])
        .send()
        .await
        .map_err(|e| ApiError::WorksOnMyMachine(format!("stripe: {e}")))?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(ApiError::WorksOnMyMachine(format!("stripe {status}: {body}")));
    }
    let session: CheckoutSession =
        response.json().await.map_err(|e| ApiError::WorksOnMyMachine(format!("stripe: {e}")))?;
    tracing::info!("checkout session opened");
    Ok(Redirect::to(&session.url))
}
