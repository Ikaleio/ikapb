# /// script
# requires-python = ">=3.14"
# dependencies = ["websockets>=15", "numpy>=2"]
# ///
"""Minimal ikapb painter: fills a rectangle with one color using a single paint token.

Usage:
    uv run examples/python/painter.py --server http://localhost:8775 \
        --paint-token 01234567-89ab-cdef-0123-456789abcdef --rect 10 10 8 8 --color '#ff6600'

Protocol: docs/protocol.md §3. Every WebSocket message is one zstd frame;
payload columns are little-endian and 4-byte aligned.
"""

import argparse
import asyncio
import struct
import time
import uuid
from compression import zstd

import numpy as np
import websockets

COOLING = 1
STATUS = {0: "OK", 1: "COOLING", 2: "NO_PAINT_TOKEN", 3: "BAD_PAINT_TOKEN", 4: "BANNED", 5: "CLOSED", 6: "IP_LIMIT"}


def pad4(n: int) -> int:
    return (n + 3) & ~3


def encode_batch(binds: list[tuple[int, bytes]], paints: list[tuple[int, int, int]]) -> bytes:
    nb, np_ = len(binds), len(paints)
    out = bytearray(struct.pack("<BBHIII", 0x01, 1, 0, nb, np_, 0))
    out += struct.pack(f"<{nb}H", *(s for s, _ in binds)).ljust(pad4(2 * nb), b"\0")
    out += b"".join(t for _, t in binds)
    out += struct.pack(f"<{np_}H", *(s for s, _, _ in paints)).ljust(pad4(2 * np_), b"\0")
    out += struct.pack(f"<{np_}I", *(i for _, i, _ in paints))
    out += struct.pack(f"<{np_}I", *(c for _, _, c in paints))
    return zstd.compress(bytes(out))


def u32(buf: bytes, off: int, n: int) -> np.ndarray:
    return np.frombuffer(buf, dtype="<u4", count=n, offset=off)


class Painter:
    def __init__(self, paint_token: bytes, rect: tuple[int, int, int, int], color: int):
        self.paint_token, self.rect, self.color = paint_token, rect, color
        self.rules: dict = {}
        self.seen: dict[int, int] = {}  # pixel index -> color last reported by our paint sonars
        # (pixel index, server time in µs) from the server's brush entries; None = no brush.
        # Any paint with this paint token, from this script or elsewhere, updates it.
        self.brush: tuple[int, int] | None = None
        self.clock_offset = 0.0  # server clock minus local clock, in seconds
        self.results: asyncio.Queue[tuple[int, int]] = asyncio.Queue()

    def on_message(self, frame: bytes) -> None:
        p = zstd.decompress(frame)
        kind = p[0]
        if kind == 0x84:  # RULES
            w, h, r = struct.unpack_from("<HHH", p, 4)
            c0, k = struct.unpack_from("<QQ", p, 16)
            (server_us,) = struct.unpack_from("<Q", p, 88)
            self.clock_offset = server_us / 1e6 - time.time()
            self.rules = {"w": w, "h": h, "c0": c0, "k": k}
            print(f"rules: board {w}x{h}, cooldown {c0 / 1e6:.3f}s + {k / 1e6:.3f}s/px, sonar radius {r} px")
        elif kind == 0x81:  # TICK
            r, nr = struct.unpack_from("<HI", p, 2)
            nb, nk = struct.unpack_from("<II", p, 16)
            a = 2 * r * r + 2 * r + 1  # cells per sonar
            brush_t = np.frombuffer(p, dtype="<u8", count=nb, offset=32)
            off = 32 + 8 * nb
            brush_idx = u32(p, off, nb)
            off += 4 * nb
            brush_slot = np.frombuffer(p, dtype="<u2", count=nb, offset=off)
            off += pad4(2 * nb)
            status = np.frombuffer(p, dtype=np.uint8, count=nr, offset=off)
            off += pad4(nr)
            value = u32(p, off, nr)
            off += 4 * nr
            sonar_idx = u32(p, off, nk)
            off += 4 * nk
            sonar_color = u32(p, off, nk * a).reshape(nk, a)
            for center, block in zip(sonar_idx.tolist(), sonar_color.tolist()):
                for pix, c in zip(self.diamond(center, r), block):
                    if pix is not None:
                        self.seen[pix] = c
            for st, val in zip(status.tolist(), value.tolist()):
                self.results.put_nowait((st, val))
            # Brush entries apply after the results. This script binds only slot 0 and
            # never guesses its brush, so the latest entry is the brush.
            for slot, t, i in zip(brush_slot.tolist(), brush_t.tolist(), brush_idx.tolist()):
                if slot == 0:
                    self.brush = (i, t) if t else None  # t = 0: no brush

    def diamond(self, center: int, r: int):
        """Pixel index of each sonar cell in wire order, or None for cells off the board."""
        w, h = self.rules["w"], self.rules["h"]
        x, y = center % w, center // w
        for dy in range(-r, r + 1):
            span = r - abs(dy)
            for dx in range(-span, span + 1):
                inside = 0 <= x + dx < w and 0 <= y + dy < h
                yield (y + dy) * w + x + dx if inside else None

    def targets(self):
        x0, y0, w, h = self.rect
        for row in range(h):  # serpentine order keeps every step at distance 1
            cols = range(w) if row % 2 == 0 else range(w - 1, -1, -1)
            for col in cols:
                yield (y0 + row) * self.rules["w"] + x0 + col

    def wait_seconds(self, idx: int) -> float:
        if self.brush is None:
            return 0.0
        p, t = self.brush
        w = self.rules["w"]
        d = abs(idx % w - p % w) + abs(idx // w - p // w)
        ready = (t + self.rules["c0"] + self.rules["k"] * d) / 1e6
        return max(0.0, ready - (time.time() + self.clock_offset))

    async def run(self, ws) -> None:
        await ws.send(encode_batch([(0, self.paint_token)], []))
        status, _ = await self.results.get()
        if status != 0:
            raise SystemExit(f"bind failed: {STATUS[status]}")
        for idx in self.targets():
            if self.seen.get(idx) == self.color:
                continue
            while True:
                # Re-check after sleeping: a paint elsewhere may have moved the brush meanwhile.
                while (wait := self.wait_seconds(idx)) > 0:
                    await asyncio.sleep(wait)
                await ws.send(encode_batch([], [(0, idx, self.color)]))
                status, value = await self.results.get()
                if status == COOLING:
                    await asyncio.sleep(value / 1e6)
                    continue
                if status != 0:
                    raise SystemExit(f"paint rejected: {STATUS[status]}")
                print(f"painted ({idx % self.rules['w']}, {idx // self.rules['w']})")
                break


async def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--server", default="http://localhost:8775")
    ap.add_argument("--paint-token", required=True)
    ap.add_argument("--rect", nargs=4, type=int, metavar=("X", "Y", "W", "H"), required=True)
    ap.add_argument("--color", default="#000000")
    args = ap.parse_args()

    rgb = int(args.color.lstrip("#"), 16)
    color = (rgb >> 16) | (rgb & 0xFF00) | ((rgb & 0xFF) << 16) | 0xFF000000
    painter = Painter(uuid.UUID(args.paint_token).bytes, tuple(args.rect), color)
    url = args.server.replace("http", "ws", 1) + "/ws?public=0"

    async with websockets.connect(url, max_size=None) as ws:
        reader = asyncio.create_task(read_loop(ws, painter))
        while not painter.rules:
            await asyncio.sleep(0.01)
        try:
            await painter.run(ws)
        finally:
            reader.cancel()


async def read_loop(ws, painter: Painter) -> None:
    async for frame in ws:
        painter.on_message(frame)


if __name__ == "__main__":
    asyncio.run(main())
