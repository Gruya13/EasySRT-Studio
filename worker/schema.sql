-- EasySRT Studio Telemetry & K-Anonymous Learning Schema (Cloudflare D1 / SQLite)
-- Zero-PII: Never stores raw IP addresses, device IDs, filenames, or API keys.

CREATE TABLE IF NOT EXISTS daily_update_checks (
  utc_date TEXT NOT NULL,
  target TEXT NOT NULL,
  arch TEXT NOT NULL,
  current_version TEXT NOT NULL,
  check_count INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (utc_date, target, arch, current_version)
);

CREATE TABLE IF NOT EXISTS job_and_edit_metrics (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  created_at TEXT NOT NULL,
  utc_date TEXT NOT NULL,
  event_kind TEXT NOT NULL,
  app_version TEXT NOT NULL,
  os TEXT NOT NULL,
  source_lang TEXT,
  target_lang TEXT,
  translation_style TEXT,
  address_form TEXT,

  -- Level 1: Hardware, Runtime & Pipeline RTF Performance
  gpu_tier TEXT,
  whisper_backend TEXT,
  whisper_model_id TEXT,
  llm_provider TEXT,
  llm_model_id TEXT,
  audio_duration_ms INTEGER,
  whisper_elapsed_ms INTEGER,
  diarizer_elapsed_ms INTEGER,
  llm_elapsed_ms INTEGER,
  asr_rtf REAL,
  llm_rtf REAL,
  oom_cpu_fallback INTEGER,
  batch_subdivision_retries INTEGER,
  cue_count INTEGER,
  preset TEXT,

  -- Level 2A: Automated Structural, Grammatical & Timing Edit Classifiers (Zero Text)
  total_baseline_cues INTEGER,
  total_final_cues INTEGER,
  unchanged_cues_count INTEGER,
  text_edited_cues_count INTEGER,
  edit_rate_pct REAL,
  initial_high_cps_count INTEGER,
  final_high_cps_count INTEGER,
  initial_line_too_long_count INTEGER,
  final_line_too_long_count INTEGER,
  avg_cps_before REAL,
  avg_cps_after REAL,
  condense_cps_used INTEGER,
  enclitic_reordered_count INTEGER,
  redundant_possessive_removed_count INTEGER,
  passive_calque_removed_count INTEGER,
  conjunction_comma_fixed_count INTEGER,
  missed_filler_deleted_count INTEGER,
  inflection_suffix_fix_count INTEGER,
  boundary_orphan_shifted_count INTEGER,
  split_count INTEGER,
  split_after_comma_count INTEGER,
  split_after_conjunction_count INTEGER,
  avg_pre_split_chars REAL,
  avg_pre_split_duration_ms REAL,
  merge_count INTEGER,
  avg_merged_short_cue_duration_ms REAL,
  avg_merged_short_cue_chars REAL,
  avg_merged_gap_ms REAL,
  start_trim_count INTEGER,
  avg_start_delta_ms REAL,
  end_trim_count INTEGER,
  avg_end_delta_ms REAL
);

CREATE INDEX IF NOT EXISTS idx_job_and_edit_metrics_created_at
  ON job_and_edit_metrics (created_at, event_kind);

CREATE TABLE IF NOT EXISTS ngram_candidates (
  pair_hash TEXT PRIMARY KEY,
  lang_pair TEXT NOT NULL,
  old_span TEXT NOT NULL,
  new_span TEXT NOT NULL,
  cohort_count INTEGER NOT NULL DEFAULT 0,
  subnet_count INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_ngram_candidates_promoted
  ON ngram_candidates (cohort_count, subnet_count, updated_at);

CREATE TABLE IF NOT EXISTS ngram_cohorts (
  pair_hash TEXT NOT NULL,
  cohort_proof TEXT NOT NULL,
  subnet_hash TEXT NOT NULL,
  created_at TEXT NOT NULL,
  PRIMARY KEY (pair_hash, cohort_proof)
);

CREATE INDEX IF NOT EXISTS idx_ngram_cohorts_pair_subnet
  ON ngram_cohorts (pair_hash, subnet_hash);

CREATE TABLE IF NOT EXISTS training_edit_pairs (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  created_at TEXT NOT NULL,
  app_version TEXT NOT NULL,
  source_lang TEXT NOT NULL,
  target_lang TEXT NOT NULL,
  style TEXT NOT NULL,
  address_form TEXT NOT NULL,
  source_text TEXT NOT NULL,
  initial_ai_text TEXT NOT NULL,
  final_human_text TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_training_edit_pairs_created_at
  ON training_edit_pairs (created_at);
