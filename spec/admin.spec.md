# Admin API Specification

## 1. Authentication and Scope

- All `/admin/*` endpoints require header `X-Admin-Key` to match `admin_manage_key` from configuration.
- If `admin_manage_key` is empty or missing, every admin request returns HTTP 401.
- Rate limiting is enforced externally by Caddy; the server performs no additional throttling or audit logging.

- CORS: `/admin/*` endpoints allow cross-origin requests that include the `X-Admin-Key` header. Preflight (`OPTIONS`) responses for `/admin/*` include `Access-Control-Allow-Headers` permitting `X-Admin-Key` (alongside common headers such as `Content-Type` and `Authorization`).

## 2. Configuration Parameters

| Parameter              | Type   | Default   | Constraint                                                                                            |
| ---------------------- | ------ | --------- | ----------------------------------------------------------------------------------------------------- |
| `admin_manage_key`     | string | `""`      | Non-empty enables admin endpoints; empty disables them.                                               |
| `admin_max_blob_bytes` | u64    | 5_242_880 | Must be `> 0`. Used as the maximum accepted size for uploaded or downloaded images in `/admin/fills`. |

## 3. `/admin/fills`

### 3.1 Request

`POST /admin/fills` with `Content-Type: application/json`.

```json
{
  "area": {"x": <u32>, "y": <u32>, "width": <u32>, "height": <u32>},
  "source": {
    "kind": "color" | "blob" | "url",
    "value": "#RRGGBB" | "<base64>" | "https://...",
    "format": "png" | "jpg" | "jpeg" | "webp"   // required for kind!=color
  },
  "filters": {
    "scale": {"mode": "contain"|"cover"|"fill", "width": <u32>, "height": <u32>},
    "grayscale": <bool>,
    "crop": {"x": <u32>, "y": <u32>, "width": <u32>, "height": <u32>}
  }
}
```

### 3.2 Validation

- Area bounds: `x < 1000`, `y < 600`, `width >= 1`, `height >= 1`, and `x + width <= 1000`, `y + height <= 600`.
- `kind=color`: `value` must be `#RRGGBB` (exactly 6 hex digits).
- `kind=blob`: `value` must be base64; decoded length `<= admin_max_blob_bytes`; `format` must be one of the allowed formats.
- `kind=url`: `value` must start with `https://`; server enforces `Content-Length <= admin_max_blob_bytes` when provided and rejects bodies exceeding the limit; `format` must be allowed.
- Supported image formats: `png`, `jpg`, `jpeg`, `webp`.
- Filter defaults: `grayscale=false`; `scale` and `crop` are optional. When `scale` is absent, target width/height default to `area.width/area.height` with `fill` semantics.
- `scale` target dimensions must be `>=1`. Behaviors:
  - `fill`: resize to exactly `width × height`, ignoring aspect ratio.
  - `contain`: resize to fit within `width × height` while keeping aspect ratio, then place the resized image at the top-left of a `width × height` black canvas (RGB all zeros).
  - `cover`: scale up/down to cover the full `width × height` (based on the larger scaling factor) and then center-crop to `width × height`.
- `crop`: applied **after** scaling. Rectangle must be fully inside the current image.
- After all filters, the image dimensions must exactly equal `area.width × area.height`; otherwise the request fails with HTTP 400.

### 3.3 Effects

- For `color` sources, every pixel in the target area is set to the parsed RGB value (`0x00RRGGBB`).
- For image sources, bytes are decoded, filters are applied in order `scale -> crop -> grayscale`, alpha is discarded, and the resulting RGB values overwrite the target area.
- Writes are applied atomically over the specified rectangle: all pixels in the area are updated within the same admin operation.
- A manual tick is emitted so `GET /api/paintboard/getboard` ETag changes and WebSocket subscribers receive a broadcast of the overwritten pixels.
- When WAL is enabled, the overwrite is appended to WAL with `uid=0`.

### 3.4 Response

- Success: HTTP 200 with body `{ "code": 200, "data": { "written": <u32>, "format": "rgb24" } }`, where `written = area.width × area.height`.
- Errors:
  - HTTP 400 for validation/decoding failures.
  - HTTP 401 when admin key is missing/invalid.
  - HTTP 413 when the decoded/downloaded image exceeds `admin_max_blob_bytes`.
  - HTTP 415 when `format` is not supported.

## 4. `/admin/tokens`

### 4.1 Data Model

Fields mirror the `tokens` table and omit legacy revoke/expire/scope flags:

```
{ uid: i64 (primary key), token: UUID, cooldown_ms: i64, issued_at_ms: i64,
  ip: string|null, last_paint_at_ms: i64|null }
```

Constraints:

- `uid` uniquely identifies each record; `token` is unique per row but is not used as the identifier for admin CRUD.
- `token` must be a valid UUID string when returned or persisted.
- `cooldown_ms >= -1`.
- `issued_at_ms` defaults to `now_ms` when omitted.
- `last_paint_at_ms` is nullable; setting it to `null` clears any cached cooldown state for the token.
- Banned `uid` values cannot receive new tokens (HTTP 403).

### 4.2 Endpoints

- `GET /admin/tokens?uid=&page=&page_size=`
  - Loads all tokens from SQLite, filters by optional `uid` (exact match), applies zero-based pagination.
  - Response: `{code:200,data:{items:[Token],total:u64}}`.
- `POST /admin/tokens`
  - Body: `{uid: i64, cooldown_ms: i64, issued_at_ms?: i64, ip?: string|null, last_paint_at_ms?: i64|null}`.
  - Generates UUIDv4, rejects banned `uid`, and replaces any previous token for the same `uid` (removing its cooldown entry).
  - Persists to SQLite, updates `TokenManager`, and seeds/clears the cooldown cache based on `last_paint_at_ms` (null clears).
- `GET /admin/tokens/{uid}`
  - Returns the token record or HTTP 404 if not found.
- `PUT /admin/tokens/{uid}`
  - Replaces the record identified by `uid`. Body uses the same shape as `POST` but omits `uid`; missing optional fields become `null`.
  - Path `uid` is authoritative and cannot be changed via the body.
  - Resets the cooldown cache according to the provided `last_paint_at_ms` value.
- `PATCH /admin/tokens/{uid}`
  - Partially updates provided fields; omitted fields remain unchanged. The `uid` cannot be changed.
  - If `last_paint_at_ms` is supplied, the cooldown cache is updated (set to the provided value or cleared when `null`).
- `DELETE /admin/tokens/{uid}`
  - Removes the token from SQLite, cooldown cache, and `TokenManager`; responds with the deleted record or `null` when absent.

### 4.3 Cache Consistency

- All write operations (`POST/PUT/PATCH/DELETE`) must update `TokenManager` immediately after successful SQLite persistence.
- Whenever a token is replaced, deleted, or has `last_paint_at_ms` set to `null`, its cooldown entry is evicted; when a non-null `last_paint_at_ms` is supplied, the cooldown cache seeds that timestamp for the token.

## 5. `/admin/bans`

### 5.1 Data Model

`bans` table (SQLite):

```
bans(
  uid INTEGER PRIMARY KEY,
  reason TEXT,
  created_at_ms INTEGER NOT NULL,
  expire_at_ms INTEGER NULL
)
```

Runtime representation matches the table schema. A ban is **active** when `expire_at_ms` is `NULL` or `expire_at_ms > now_ms`.

### 5.2 Endpoints

- `POST /admin/bans`
  - Body: `{uid: i64, reason?: string, expire_at_ms?: i64}`. `created_at_ms` is set to current time. If a ban already exists, it is replaced with the new window/reason.
  - Response: `{code:200,data:BanRecord}`.
- `GET /admin/bans?uid=&active_only=`
  - Returns all bans from the in-memory store, optionally filtering by `uid` and/or `active_only=true`.
- `DELETE /admin/bans/{uid}`
  - Removes the ban (idempotent) and returns the last stored record or `null` when none existed.

### 5.3 Enforcement

- During WebSocket paint validation, the server rejects packets whose `uid` is currently banned, returning ACK_FORBIDDEN.
- `/api/auth/gettoken` and `/admin/tokens` create/update operations refuse to issue tokens for banned `uid` values (HTTP 403).

## 6. Error Envelope

Admin handlers use JSON envelopes `{code: <u16>, data: <object>}`.

- Success uses `code=200`.
- Validation failures return HTTP 400 with `code=400` and `data.error` explaining the reason.
- Missing/invalid admin key returns HTTP 401.
- Not found returns HTTP 404.
- Conflicts (UID uniqueness) return HTTP 409.

## 7. `/admin/ws/connections`

### 7.1 Authentication

- Requires `X-Admin-Key` header matching `admin_manage_key`.
- Returns HTTP 401 when admin access is disabled or the key is invalid.

### 7.2 Response

- Method: `GET`
- Body: `{code:200,data:{items:[{ip:string, connections:u64}]}}`
  - `ip` uses canonical string formatting from `IpAddr`.
  - `connections` equals the number of active WebSocket sessions currently tracked for the IP.
- Semantics: Snapshot of the in-memory IP connection limiter at request time; counts may change after the response is sent. Tracking occurs even when `network.max_connections_per_ip` is unset or `0` (enforcement disabled).
