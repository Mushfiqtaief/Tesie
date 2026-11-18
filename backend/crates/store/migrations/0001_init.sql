CREATE TABLE IF NOT EXISTS proxy_nodes (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    proxy_type TEXT NOT NULL DEFAULT 'socks5',
    proxy_url TEXT,
    raw_outbound_json TEXT,
    is_direct INTEGER NOT NULL DEFAULT 0,
    enabled INTEGER NOT NULL DEFAULT 1,
    alive INTEGER NOT NULL DEFAULT 1,
    egress_ip TEXT,
    last_delay_ms INTEGER,
    last_probed_at INTEGER,
    consecutive_failures INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_proxy_nodes_egress_ip ON proxy_nodes(egress_ip);

CREATE TABLE IF NOT EXISTS ip_quotas (
    ip_key TEXT PRIMARY KEY,
    provider_id TEXT NOT NULL DEFAULT 'opencode',
    window_started_at INTEGER NOT NULL,
    window_duration_ms INTEGER NOT NULL DEFAULT 3600000,
    requests_in_window INTEGER NOT NULL DEFAULT 0,
    max_requests_per_window INTEGER,
    tokens_in_window INTEGER NOT NULL DEFAULT 0,
    max_tokens_per_window INTEGER,
    total_requests INTEGER NOT NULL DEFAULT 0,
    total_prompt_tokens INTEGER NOT NULL DEFAULT 0,
    total_completion_tokens INTEGER NOT NULL DEFAULT 0,
    total_rate_limits INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL DEFAULT 'active',
    cooldown_until INTEGER,
    backoff_level INTEGER NOT NULL DEFAULT 0,
    canonical_session_id TEXT,
    session_created_at INTEGER,
    last_error TEXT,
    last_used_at INTEGER,
    updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_ip_quotas_status ON ip_quotas(status, cooldown_until);

CREATE TABLE IF NOT EXISTS provider_connections (
    id TEXT PRIMARY KEY,
    provider_id TEXT NOT NULL,
    name TEXT NOT NULL,
    api_key TEXT,
    proxy_node_id TEXT REFERENCES proxy_nodes(id) ON DELETE SET NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    cooldown_until INTEGER,
    backoff_level INTEGER NOT NULL DEFAULT 0,
    last_error TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS api_keys (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    key_hash TEXT NOT NULL UNIQUE,
    key_prefix TEXT NOT NULL,
    raw_key TEXT,
    quota_tokens_limit INTEGER,
    quota_tokens_used INTEGER NOT NULL DEFAULT 0,
    quota_requests_limit INTEGER,
    quota_requests_used INTEGER NOT NULL DEFAULT 0,
    enabled INTEGER NOT NULL DEFAULT 1,
    created_at INTEGER NOT NULL,
    last_used_at INTEGER
);

CREATE TABLE IF NOT EXISTS provider_configs (
    provider_id TEXT PRIMARY KEY,
    enabled INTEGER NOT NULL DEFAULT 1,
    proxy_mode TEXT NOT NULL DEFAULT 'all',
    selected_proxy_ids_json TEXT NOT NULL DEFAULT '[]',
    window_duration_ms INTEGER NOT NULL DEFAULT 3600000,
    max_requests_per_window INTEGER,
    max_tokens_per_window INTEGER,
    session_ttl_ms INTEGER NOT NULL DEFAULT 14400000,
    thinking_level TEXT NOT NULL DEFAULT 'auto',
    probe_enabled INTEGER NOT NULL DEFAULT 1,
    probe_interval_sec INTEGER NOT NULL DEFAULT 30,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS combos (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    strategy TEXT NOT NULL DEFAULT 'fallback',
    models_json TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS usage_logs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    request_id TEXT NOT NULL,
    api_key_id TEXT,
    provider_id TEXT NOT NULL,
    model TEXT NOT NULL,
    proxy_name TEXT NOT NULL,
    egress_ip TEXT,
    status_code INTEGER NOT NULL,
    prompt_tokens INTEGER NOT NULL DEFAULT 0,
    cached_tokens INTEGER NOT NULL DEFAULT 0,
    completion_tokens INTEGER NOT NULL DEFAULT 0,
    total_tokens INTEGER NOT NULL DEFAULT 0,
    latency_ms INTEGER NOT NULL,
    ttft_ms INTEGER,
    tokens_per_sec REAL,
    error_kind TEXT,
    error_detail TEXT,
    created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_usage_logs_created_at ON usage_logs(created_at DESC);
CREATE INDEX IF NOT EXISTS idx_usage_logs_egress_ip ON usage_logs(egress_ip);

CREATE TABLE IF NOT EXISTS settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    updated_at INTEGER NOT NULL
);

INSERT OR IGNORE INTO proxy_nodes (
    id, name, proxy_type, proxy_url, raw_outbound_json, is_direct, enabled, alive,
    consecutive_failures, created_at, updated_at
) VALUES (
    'direct', 'DIRECT', 'direct', NULL, NULL, 1, 1, 1,
    0, CAST(strftime('%s','now') AS INTEGER) * 1000, CAST(strftime('%s','now') AS INTEGER) * 1000
);

INSERT OR IGNORE INTO settings (key, value, updated_at) VALUES
    ('strict_proxy_mode', 'false', CAST(strftime('%s','now') AS INTEGER) * 1000),
    ('default_free_ip_window_ms', '3600000', CAST(strftime('%s','now') AS INTEGER) * 1000),
    ('default_free_ip_max_requests', '', CAST(strftime('%s','now') AS INTEGER) * 1000),
    ('default_free_ip_max_tokens', '', CAST(strftime('%s','now') AS INTEGER) * 1000),
    ('session_ttl_ms', '14400000', CAST(strftime('%s','now') AS INTEGER) * 1000),
    ('proxy_test_interval_sec', '15', CAST(strftime('%s','now') AS INTEGER) * 1000);
