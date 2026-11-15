# Paintboard FE

Vite + React Router SPA for the Paintboard frontend.

## Development

```bash
bun install
bun dev
```

### Environment variables

- `VITE_PAINTBOARD_BASE_URL` (optional): default backend base URL shown in the UI

## Build / Preview

```bash
bun run build
bun start
```

## Routes

- `/` - main paintboard canvas
- `/admin` - admin console

## Deployment notes

- This is a client-side SPA; your hosting must rewrite all routes to `index.html`.
- The `/admin` page calls backend admin endpoints from the browser. Ensure your backend CORS allows `X-Admin-Key` (and `Content-Type`).
