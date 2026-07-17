PRAGMA foreign_keys = ON;
PRAGMA journal_mode = WAL;
PRAGMA synchronous = FULL;
PRAGMA secure_delete = ON;

CREATE TABLE IF NOT EXISTS schema_migrations (
  version INTEGER PRIMARY KEY
);

CREATE TABLE IF NOT EXISTS targets (
  target_json TEXT PRIMARY KEY
);

CREATE TABLE IF NOT EXISTS suite_versions (
  suite_id TEXT NOT NULL,
  suite_version TEXT NOT NULL,
  content_sha256 TEXT NOT NULL,
  scoring_rule_version TEXT NOT NULL,
  PRIMARY KEY (suite_id, suite_version)
);

CREATE TABLE IF NOT EXISTS runs (
  id TEXT PRIMARY KEY,
  target_json TEXT NOT NULL,
  mode_json TEXT NOT NULL,
  suite_id TEXT NOT NULL,
  suite_version TEXT NOT NULL,
  status_json TEXT NOT NULL,
  started_at TEXT NOT NULL,
  finished_at TEXT,
  total_tasks INTEGER NOT NULL,
  completed_tasks INTEGER NOT NULL,
  environment_json TEXT NOT NULL,
  score_json TEXT,
  FOREIGN KEY (target_json) REFERENCES targets(target_json),
  FOREIGN KEY (suite_id, suite_version)
    REFERENCES suite_versions(suite_id, suite_version)
);

CREATE TABLE IF NOT EXISTS task_results (
  run_id TEXT NOT NULL,
  task_id TEXT NOT NULL,
  category_json TEXT NOT NULL,
  outcome_json TEXT NOT NULL,
  score REAL,
  failure_kind_json TEXT,
  duration_ms INTEGER NOT NULL,
  answer_rel_path TEXT,
  detail TEXT NOT NULL,
  PRIMARY KEY (run_id, task_id),
  FOREIGN KEY (run_id) REFERENCES runs(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS runs_started_at_idx ON runs(started_at DESC);
CREATE INDEX IF NOT EXISTS task_results_run_idx ON task_results(run_id);

INSERT OR IGNORE INTO schema_migrations(version) VALUES (1);
