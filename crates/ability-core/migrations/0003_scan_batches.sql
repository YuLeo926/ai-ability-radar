PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS scan_batches (
  id TEXT PRIMARY KEY,
  plan_json TEXT NOT NULL,
  mode_json TEXT NOT NULL,
  suite_id TEXT NOT NULL,
  suite_version TEXT NOT NULL,
  content_sha256 TEXT NOT NULL,
  scoring_rule_version TEXT NOT NULL,
  seed INTEGER NOT NULL CHECK (seed >= 0),
  status_json TEXT NOT NULL,
  acknowledgement_hash TEXT NOT NULL,
  acknowledgement_expires_at TEXT NOT NULL,
  planned_member_count INTEGER NOT NULL CHECK (planned_member_count BETWEEN 1 AND 25),
  terminal_member_count INTEGER NOT NULL DEFAULT 0
    CHECK (terminal_member_count >= 0 AND terminal_member_count <= planned_member_count),
  cancel_requested INTEGER NOT NULL DEFAULT 0 CHECK (cancel_requested IN (0,1)),
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  FOREIGN KEY (suite_id, suite_version)
    REFERENCES suite_versions(suite_id, suite_version)
);

CREATE TABLE IF NOT EXISTS scan_batch_targets (
  batch_id TEXT NOT NULL,
  position INTEGER NOT NULL CHECK (position >= 0),
  target_json TEXT NOT NULL,
  route_identity_json TEXT NOT NULL,
  adapter_identity_json TEXT NOT NULL,
  execution_surface_json TEXT NOT NULL,
  PRIMARY KEY (batch_id, position),
  FOREIGN KEY (batch_id) REFERENCES scan_batches(id) ON DELETE CASCADE,
  FOREIGN KEY (target_json) REFERENCES targets(target_json)
);

CREATE TABLE IF NOT EXISTS scan_batch_members (
  batch_id TEXT NOT NULL,
  ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
  target_position INTEGER NOT NULL CHECK (target_position >= 0),
  repetition_index INTEGER NOT NULL CHECK (repetition_index >= 0),
  run_id TEXT UNIQUE,
  status_json TEXT NOT NULL,
  failure_kind_json TEXT,
  attempt_number INTEGER NOT NULL DEFAULT 0 CHECK (attempt_number >= 0),
  updated_at TEXT NOT NULL,
  PRIMARY KEY (batch_id, ordinal),
  UNIQUE (batch_id, target_position, repetition_index),
  UNIQUE (batch_id, ordinal, run_id),
  FOREIGN KEY (batch_id, target_position)
    REFERENCES scan_batch_targets(batch_id, position),
  FOREIGN KEY (run_id) REFERENCES runs(id) ON DELETE RESTRICT
);

CREATE TABLE IF NOT EXISTS scan_execution_authorizations (
  batch_id TEXT NOT NULL,
  member_scope INTEGER NOT NULL CHECK (member_scope >= -1),
  attempt_number INTEGER NOT NULL CHECK (attempt_number > 0),
  max_provider_turns INTEGER NOT NULL CHECK (max_provider_turns > 0),
  max_task_budget_secs INTEGER NOT NULL CHECK (max_task_budget_secs > 0),
  acknowledgement_hash TEXT NOT NULL,
  allowed_failure_kind_json TEXT,
  expires_at TEXT NOT NULL,
  created_at TEXT NOT NULL,
  PRIMARY KEY (batch_id, member_scope, attempt_number),
  FOREIGN KEY (batch_id) REFERENCES scan_batches(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS scan_batch_task_isolation (
  batch_id TEXT NOT NULL,
  member_ordinal INTEGER NOT NULL,
  run_id TEXT NOT NULL,
  task_id TEXT NOT NULL,
  policy_version INTEGER NOT NULL CHECK (policy_version > 0),
  enforcement_json TEXT NOT NULL,
  user_attested INTEGER NOT NULL CHECK (user_attested IN (0,1)),
  recorded_at TEXT NOT NULL,
  PRIMARY KEY (run_id, task_id),
  FOREIGN KEY (batch_id, member_ordinal, run_id)
    REFERENCES scan_batch_members(batch_id, ordinal, run_id) ON DELETE CASCADE,
  FOREIGN KEY (run_id, task_id)
    REFERENCES task_results(run_id, task_id) ON DELETE CASCADE
    DEFERRABLE INITIALLY DEFERRED
);

CREATE TABLE IF NOT EXISTS baseline_snapshots (
  candidate_batch_id TEXT PRIMARY KEY,
  baseline_as_of TEXT NOT NULL,
  snapshot_json TEXT NOT NULL,
  content_sha256 TEXT NOT NULL,
  created_at TEXT NOT NULL,
  FOREIGN KEY (candidate_batch_id) REFERENCES scan_batches(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS scan_deletion_intents (
  id TEXT PRIMARY KEY,
  batch_id TEXT NOT NULL,
  run_id TEXT,
  quarantine_token TEXT,
  phase_json TEXT NOT NULL,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS scan_batches_status_created_idx
  ON scan_batches(status_json, created_at DESC, id ASC);
CREATE UNIQUE INDEX IF NOT EXISTS scan_batches_acknowledgement_once_idx
  ON scan_batches(acknowledgement_hash);
CREATE INDEX IF NOT EXISTS scan_batch_members_order_idx
  ON scan_batch_members(batch_id, ordinal ASC);
CREATE INDEX IF NOT EXISTS scan_batch_members_status_idx
  ON scan_batch_members(batch_id, status_json, ordinal ASC);
CREATE INDEX IF NOT EXISTS scan_execution_authorizations_expiry_idx
  ON scan_execution_authorizations(batch_id, member_scope, expires_at DESC);
CREATE UNIQUE INDEX IF NOT EXISTS scan_execution_authorizations_ack_once_idx
  ON scan_execution_authorizations(acknowledgement_hash);
CREATE INDEX IF NOT EXISTS scan_deletion_intents_phase_idx
  ON scan_deletion_intents(phase_json, created_at ASC);

INSERT OR IGNORE INTO schema_migrations(version) VALUES (3);
