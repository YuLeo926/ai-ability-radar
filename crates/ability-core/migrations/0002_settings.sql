CREATE TABLE IF NOT EXISTS settings (
  key TEXT PRIMARY KEY,
  value_json TEXT NOT NULL
);

INSERT OR IGNORE INTO settings(key,value_json)
VALUES ('raw_retention_days', 'null');

CREATE TABLE IF NOT EXISTS publications (
  report_id TEXT PRIMARY KEY,
  run_id TEXT NOT NULL,
  exported_at TEXT NOT NULL,
  report_sha256 TEXT NOT NULL,
  destination_kind TEXT NOT NULL,
  FOREIGN KEY (run_id) REFERENCES runs(id) ON DELETE CASCADE
);

INSERT OR IGNORE INTO schema_migrations(version) VALUES (2);
