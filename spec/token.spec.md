# Token Subsystem Specification

## 1. Overview

The token subsystem issues authentication tokens, validates them for paint
requests, and persists the latest paint timestamp across graceful shutdowns.

## 2. Token Formats

- External representation: UUID string with hyphens
  (`xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx`).
- Binary representation (over the paint protocol): 16 raw bytes of the UUID.
- Storage representation (SQLite): UUID string with hyphens.

## 3. Storage Schema

All token data is stored in a single SQLite table.

```sql
tokens (
    uid INTEGER PRIMARY KEY,       -- User identifier
    token TEXT NOT NULL,           -- UUID string, unique per user
    issued_at_ms INTEGER NOT NULL, -- Token creation timestamp (ms)
    cooldown_ms INTEGER NOT NULL,  -- Paint cooldown in milliseconds
    ip TEXT,                       -- Optional last-known client IP
    last_paint_at_ms INTEGER       -- Nullable; persisted cooldown snapshot
)
```

- `uid` uniquely identifies the row and enforces one token per user.
- `token` is unique across all users.
- `last_paint_at_ms` stores the latest paint time from the previous runtime and
  is cleared to `NULL` immediately after startup load.
- Expiration, revocation flags, and `token_state` are removed.

## 4. Token Validation

**Function:** `validate_token(token: [u8; 16], uid: u32) -> Result<TokenInfo, Error>`

**Procedure:**
1. Parse the 16-byte payload as a UUID.
2. Look up a row whose `token` matches the UUID string.
3. If no row exists: return `INVALID_TOKEN`.
4. If `uid` differs from the row’s `uid`: return `UID_MISMATCH`.
5. If the `uid` is present in the ban list: return `INVALID_TOKEN`.
6. Otherwise return `TokenInfo`.

**TokenInfo fields:**

```rust
TokenInfo {
    uid: u32,
    token: [u8; 16],
    issued_at_ms: u64,
    cooldown_ms: i64,
    ip: Option<String>,
}
```

`cooldown_ms = -1` means “use the global cooldown value”.

## 5. Cooldown Lifecycle

### 5.1 Startup

1. Load every row from `tokens` into memory.
2. For each row with `last_paint_at_ms` not `NULL`, seed the in-memory cooldown
   cache using that value.
3. After the load completes, execute `UPDATE tokens SET last_paint_at_ms = NULL`
   so that stale cooldowns are not reused on the next run.

### 5.1.1 Legacy migration

During startup schema initialization, the process checks for the legacy layout
(`tokens` table with `expire_at_ms`/`revoked` columns or a `token_state`
table). If found, it performs a one-time migration:

- Read all rows from the legacy `tokens` table and discard tokens that are
  revoked or already expired at migration time.
- Parse token strings as UUIDs; rows with invalid UUIDs are dropped.
- Load `last_paint_at_ms` from `token_state` when it matches a kept token;
  unmatched `token_state` rows are ignored.
- Deduplicate by `uid`, keeping the token with the greatest
  `issued_at_ms`. The newer token is kept even if only the older one has
  persisted cooldown data.
- Drop legacy indexes and the `token_state`/legacy `tokens` tables, create the
  new `tokens` schema, and insert the migrated rows with `ip` set to `NULL`.
- Log the migration outcome (migrated rows, dropped expired/revoked rows,
  invalid tokens, stale state rows, and dedup count).

### 5.1.2 Removing redundant `ban_reason`

During startup schema initialization, if the `tokens` table contains a
`ban_reason` column, the server performs a one-time migration:

- Read all rows from the existing `tokens` table.
- Create the new `tokens` schema (without `ban_reason`).
- Insert each row into the new table, preserving `uid`, `token`, `issued_at_ms`,
  `cooldown_ms`, `ip`, and `last_paint_at_ms`.
- Discard the `ban_reason` column values.

### 5.2 Runtime Checks

Let `cooldown_ms` be the effective cooldown (global value when
`cooldown_ms = -1`). Let `last_paint_ms` be the in-memory cache entry for the
token, defaulting to `0` when absent. Let `now_ms` be the current UNIX time in
milliseconds.

```
IF cooldown_ms <= 0:
    RETURN true
IF last_paint_ms = 0:
    RETURN true
IF now_ms - last_paint_ms >= cooldown_ms:
    RETURN true
ELSE:
    RETURN false
```

Whenever a paint is accepted, the cache entry for that token is updated to the
current `now_ms`.

### 5.3 Graceful Shutdown

On graceful shutdown, take a snapshot of the in-memory cooldown cache and write
`last_paint_at_ms` for each token back to the `tokens` table in a single batch
transaction. If the snapshot is empty, no write occurs.

## 6. HTTP Admin API (Token routes)

- Issuing a token replaces any previous token for the same `uid`.
- Admin CRUD endpoints expose `uid`, `token`, `issued_at_ms`, `cooldown_ms`,
  `ip`, and `last_paint_at_ms`; fields not supplied when creating
  a token default to `None` except for `issued_at_ms` (current time) and
  `cooldown_ms` (required).
- Expiration and revocation are not part of the model and must not be exposed
  or enforced.

## 7. Public HTTP Token Issuance (`/api/*/gettoken`)

When a token is issued via `/api/auth/gettoken` or `/api/paintboard/gettoken`,
the persisted `tokens.ip` field is set to the resolved `client_ip` for the HTTP
request, formatted using `IpAddr` canonical string formatting. Client IP
resolution follows the procedure in `spec/config.spec.md` §3.2.2.
