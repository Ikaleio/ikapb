# AGENTS.md - Paintboard FE Frontend

## Project Overview

Real-time collaborative pixel art canvas application built as a **Vite + React Router SPA**, React 19, TypeScript, and shadcn/ui. The app connects to a paintboard backend directly via HTTP and WebSocket for live pixel updates.

## Architecture & Data Flow

### Core Components Structure

- **Single-page app**: `src/routes/home.tsx` renders `<PaintboardCanvas />` which is the main UI
- **Routing**: `src/router.tsx` defines `/` and `/admin`
- **App wrapper**: `src/root-layout.tsx` wraps the app with `ThemeProvider` (+ analytics)
- **Component organization**: `components/paintboard-canvas.tsx` (800+ lines) contains all canvas logic, WebSocket handling, UI panels, and state management
- **No external state management**: All state lives in React hooks within `PaintboardCanvas`

### Backend Integration Pattern

The app directly connects to the backend server configured via `baseUrl`:

- `GET /api/paintboard/getboard` - Fetches initial board state (600K RGB pixel data)
- `POST /api/auth/gettoken` - Authenticates user with UID + access_key to get paint token
- WebSocket connection to `ws://<baseUrl>/api/paintboard/ws` for real-time pixel updates

### Critical Data Flow

1. User sets `baseUrl` → localStorage → triggers board fetch from backend
2. Board data (Uint8Array: 1000×600×3 RGB bytes) → `boardDataRef.current` → rendered to canvas
3. WebSocket opens → receives binary messages (3 message types: 0xfa pixel update, 0xfc heartbeat, 0xff paint response)
4. User clicks → constructs 31-byte binary paint message with token → WebSocket send → await 0xff response with status code

## Development Workflows

### Running the App

```bash
bun install    # Install dependencies
bun dev        # Start dev server on port 3000
bun run build  # Production build
bun start      # Preview production build (serves `dist/`)
```

### Key Configuration

- **CSP**: `index.html` includes a CSP meta tag allowing all WebSocket connections (`ws:` and `wss:`). For production, prefer setting CSP headers in your hosting/CDN.
- **Path aliases**: `@/*` maps to project root (defined in `tsconfig.json` and `components.json`)

## Project-Specific Patterns

### Binary Protocol Implementation

Located in `components/paintboard-canvas.tsx`:

```typescript
// Paint message structure (31 bytes):
// [0]: 0xfe opcode
// [1-2]: x coordinate (uint16 little-endian)
// [3-4]: y coordinate (uint16 little-endian)
// [5-7]: RGB color bytes
// [8-10]: uid (uint24 little-endian)
// [11-26]: token UUID bytes (16 bytes, hex string converted)
// [27-30]: message ID (uint32 little-endian)
```

**Critical**: Token must be converted from UUID string format (with dashes removed) to 16 raw bytes.

### State Persistence Strategy

Three values saved to localStorage with `paintboard_` prefix:

- `paintboard_baseUrl` - Backend server URL
- `paintboard_token` - Authentication token (UUID format)
- `paintboard_uid` - User ID string

Loaded on mount, no expiration logic.

### shadcn/ui Integration

- Configuration in `components.json` defines aliases and style="new-york"
- UI components in `components/ui/` use Radix UI primitives + Tailwind
- Utility function `cn()` in `lib/utils.ts` combines clsx + tailwind-merge for conditional classes

### Theme System

- Uses `next-themes` via `components/theme-provider.tsx` wrapper
- System theme detection enabled by default
- Theme toggle in `components/theme-toggle.tsx` uses dropdown with sun/moon icons

## Critical Gotchas

### WebSocket Retry Logic

Max 3 retries with exponential backoff (2^n seconds, capped at 10s). Connection state in `wsStatus` state. **Important**: Retry counter resets on successful connection.

### Canvas Rendering Optimization

- Canvas uses `imageRendering: 'pixelated'` CSS for sharp pixel art
- Full board re-renders only on initial load (`renderBoard()`)
- Individual pixel updates use `fillRect(x, y, 1, 1)` for efficiency
- Board data stored in ref (`boardDataRef`) to avoid re-renders

### Paint Callback System

Uses `Map<number, callback>` keyed by random message IDs to match async paint responses. Callbacks auto-delete after invocation. Status codes:

- `0xef` (239): Success
- `0xee` (238): Cooldown active
- `0xed` (237): Invalid token
- `0xec` (236): Malformed request

## External Dependencies

### Key Libraries

- **UI Framework**: shadcn/ui (30+ Radix UI components pre-installed)
- **Form Handling**: react-hook-form + zod (installed but unused in current code)
- **Toast Notifications**: Custom implementation in `components/ui/toaster.tsx` using Radix Toast
- **Icons**: lucide-react

### Backend API Expectations

Backend must provide:

- `GET /api/paintboard/getboard` - Returns raw RGB binary (1,800,000 bytes)
- `POST /api/auth/gettoken` - Accepts `{uid: number, access_key: string}`, returns `{code: 200, data: {token: string}}` on success or `{code: 403, data: {errorType: string}}` on failure
- `WS /api/paintboard/ws` - Binary WebSocket protocol (see Binary Protocol section)

## Adding Features

### To Add New UI Components

```bash
npx shadcn@latest add <component-name>
```

Components auto-install to `components/ui/` with proper aliases.

### To Modify Canvas Behavior

All canvas interaction logic is in `paintboard-canvas.tsx`:

- Click handling: `handleCanvasClick` (line ~380)
- Pan/zoom: `handleMouseDown`, `handleMouseMove`, `handleMouseUp` (line ~500+)
- WebSocket messages: `ws.onmessage` switch statement (line ~200+)

## Admin Console

- Route: `/admin` (via React Router)
- Calls backend admin endpoints from the browser, so the backend must allow CORS for `X-Admin-Key` (and `Content-Type`).
