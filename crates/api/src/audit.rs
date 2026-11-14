//! Audit log: domain events, created tickets and refused requests, in SQLite.
//! Writes never block or fail a request; they also go to `tracing` (and so to OTLP).

use chrono::Utc;
use serde_json::{Value, json};
use sqlx::{Row, SqlitePool};

use crate::events::EventSink;
use nullbox_domain::DomainEvent;

#[derive(Clone)]
pub struct AuditLog {
    pool: SqlitePool,
}

impl AuditLog {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub fn record(&self, action: &str, actor: Option<&str>, detail: Value) {
        tracing::info!(target: "nullbox::audit", action, actor = actor.unwrap_or("-"), %detail, "audit");
        let pool = self.pool.clone();
        let (action, actor, detail) = (action.to_owned(), actor.map(str::to_owned), detail.to_string());
        tokio::spawn(async move {
            let result = sqlx::query("INSERT INTO audit_log (at, action, actor, detail) VALUES (?, ?, ?, ?)")
                .bind(Utc::now().to_rfc3339())
                .bind(&action)
                .bind(&actor)
                .bind(&detail)
                .execute(&pool)
                .await;
            if let Err(e) = result {
                tracing::error!(error = %e, "audit write failed");
            }
        });
    }

    pub async fn recent(&self, limit: i64) -> Result<Vec<Value>, sqlx::Error> {
        let rows = sqlx::query("SELECT at, action, actor, detail FROM audit_log ORDER BY id DESC LIMIT ?")
            .bind(limit)
            .fetch_all(&self.pool)
            .await?;
        Ok(rows
            .iter()
            .map(|row| {
                let detail: String = row.get("detail");
                json!({
                    "at": row.get::<String, _>("at"),
                    "action": row.get::<String, _>("action"),
                    "actor": row.get::<Option<String>, _>("actor"),
                    "detail": serde_json::from_str::<Value>(&detail).unwrap_or(Value::String(detail)),
                })
            })
            .collect())
    }
}

/// Domain events go to the audit log as well as to the log output.
pub struct AuditEventSink(pub AuditLog);

impl EventSink for AuditEventSink {
    fn emit(&self, event: DomainEvent) {
        let payload = serde_json::to_value(&event).unwrap_or_default();
        self.0.record("domain_event", None, payload);
    }
}
