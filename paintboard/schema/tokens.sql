PRAGMA journal_mode=WAL;

CREATE TABLE IF NOT EXISTS tokens(
  uid INTEGER PRIMARY KEY,
  token TEXT NOT NULL,
  issued_at_ms INTEGER NOT NULL,
  cooldown_ms INTEGER NOT NULL,
  ip TEXT,
  last_paint_at_ms INTEGER
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_tokens_token_unique ON tokens(token);

CREATE TABLE IF NOT EXISTS bans(
  uid INTEGER PRIMARY KEY,
  reason TEXT,
  created_at_ms INTEGER NOT NULL,
  expire_at_ms INTEGER
);
