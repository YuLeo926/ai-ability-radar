PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS task_agent_evidence (
  run_id TEXT NOT NULL,
  task_id TEXT NOT NULL,
  contract_version TEXT NOT NULL,
  status_json TEXT NOT NULL,
  command_succeeded INTEGER CHECK (command_succeeded IS NULL OR command_succeeded >= 0),
  command_failed INTEGER CHECK (command_failed IS NULL OR command_failed >= 0),
  command_unknown INTEGER CHECK (command_unknown IS NULL OR command_unknown >= 0),
  exit_codes_json TEXT NOT NULL,
  tool_error_count INTEGER CHECK (tool_error_count IS NULL OR tool_error_count >= 0),
  file_change_count INTEGER CHECK (file_change_count IS NULL OR file_change_count >= 0),
  session_present INTEGER CHECK (session_present IS NULL OR session_present IN (0,1)),
  tokens_json TEXT NOT NULL,
  model_json TEXT,
  provider_unknown_field_count INTEGER
    CHECK (provider_unknown_field_count IS NULL OR provider_unknown_field_count >= 0),
  agent_duration_ms INTEGER CHECK (agent_duration_ms IS NULL OR agent_duration_ms >= 0),
  evidence_rel_path TEXT,
  PRIMARY KEY (run_id, task_id),
  FOREIGN KEY (run_id, task_id)
    REFERENCES task_results(run_id, task_id) ON DELETE CASCADE
    DEFERRABLE INITIALLY DEFERRED
);

CREATE INDEX IF NOT EXISTS task_agent_evidence_run_idx
  ON task_agent_evidence(run_id, task_id);

INSERT OR IGNORE INTO schema_migrations(version) VALUES (4);
