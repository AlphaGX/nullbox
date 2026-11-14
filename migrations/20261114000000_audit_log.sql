-- Who did what, and who was refused. Append-only.
CREATE TABLE audit_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    at TEXT NOT NULL,
    action TEXT NOT NULL,
    actor TEXT,
    detail TEXT NOT NULL
);
CREATE INDEX audit_log_at ON audit_log (at);
