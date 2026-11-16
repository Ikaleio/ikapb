/**
 * Minimal ikapb painter: fills a rectangle with one color using a single paint token.
 *
 * Requires Node.js >= 24.6 (built-in WebSocket, zstd in node:zlib, type stripping). No dependencies.
 *
 *   node examples/ts/painter.ts --server http://localhost:8775 \
 *     --paint-token 01234567-89ab-cdef-0123-456789abcdef --rect 10,10,8,8 --color '#ff6600'
 *
 * Protocol: docs/protocol.md §3. Every WebSocket message is one zstd frame;
 * payload columns are little-endian and 4-byte aligned.
 */
import { setTimeout as sleep } from "node:timers/promises";
import { parseArgs } from "node:util";
import { zstdCompressSync, zstdDecompressSync } from "node:zlib";

const COOLING = 1;
const STATUS = ["OK", "COOLING", "NO_PAINT_TOKEN", "BAD_PAINT_TOKEN", "BANNED", "CLOSED", "IP_LIMIT"];

const pad4 = (n: number) => (n + 3) & ~3;

function encodeBatch(binds: [number, Uint8Array][], paints: [number, number, number][]): Buffer {
	const nb = binds.length;
	const np = paints.length;
	const bindPaintTokens = 16 + pad4(2 * nb);
	const paintSlots = bindPaintTokens + 16 * nb;
	const paintIdx = paintSlots + pad4(2 * np);
	const buf = Buffer.alloc(paintIdx + 8 * np);
	buf.writeUInt8(0x01, 0);
	buf.writeUInt8(1, 1);
	buf.writeUInt32LE(nb, 4);
	buf.writeUInt32LE(np, 8);
	binds.forEach(([slot, paintToken], i) => {
		buf.writeUInt16LE(slot, 16 + 2 * i);
		buf.set(paintToken, bindPaintTokens + 16 * i);
	});
	paints.forEach(([slot, idx, color], i) => {
		buf.writeUInt16LE(slot, paintSlots + 2 * i);
		buf.writeUInt32LE(idx, paintIdx + 4 * i);
		buf.writeUInt32LE(color, paintIdx + 4 * np + 4 * i);
	});
	return zstdCompressSync(buf);
}

interface Rules {
	w: number;
	h: number;
	c0: bigint;
	k: bigint;
}

class Painter {
	rules?: Rules;
	/** Pixel index -> color last reported by our paint sonars. */
	seen = new Map<number, number>();
	/**
	 * Brush from the server's brush entries (`t` in server µs); undefined = no brush.
	 * Any paint with this paint token, from this script or elsewhere, updates it.
	 */
	brush?: { idx: number; t: number };
	/** Server clock minus local clock, in µs. */
	private clockOffsetUs = 0;
	private results: [number, number][] = [];
	private waiter?: () => void;
	private ws: WebSocket;
	private paintToken: Uint8Array;
	private rect: [number, number, number, number];
	private color: number;

	constructor(ws: WebSocket, paintToken: Uint8Array, rect: [number, number, number, number], color: number) {
		this.ws = ws;
		this.paintToken = paintToken;
		this.rect = rect;
		this.color = color;
	}

	onMessage(frame: ArrayBuffer) {
		const p = zstdDecompressSync(Buffer.from(frame));
		const kind = p[0];
		if (kind === 0x84) {
			this.rules = { w: p.readUInt16LE(4), h: p.readUInt16LE(6), c0: p.readBigUInt64LE(16), k: p.readBigUInt64LE(24) };
			this.clockOffsetUs = Number(p.readBigUInt64LE(88)) - Date.now() * 1000;
			console.log(
				`rules: board ${this.rules.w}x${this.rules.h}, cooldown ${Number(this.rules.c0) / 1e6}s + ${Number(this.rules.k) / 1e6}s/px, sonar radius ${p.readUInt16LE(8)} px`,
			);
		} else if (kind === 0x81) {
			this.onTick(p);
		}
	}

	private onTick(p: Buffer) {
		const r = p.readUInt16LE(2);
		const [nr, nb, nk] = [4, 16, 20].map((o) => p.readUInt32LE(o));
		const a = 2 * r * r + 2 * r + 1; // cells per sonar
		// Copy into an aligned buffer so every u32 column can be viewed as a Uint32Array.
		const words = new Uint32Array(new Uint8Array(p).buffer, 0, p.length >> 2);
		const brushIdxAt = 32 + 8 * nb;
		const brushSlotAt = brushIdxAt + 4 * nb;
		const statusAt = brushSlotAt + pad4(2 * nb);
		let off = (statusAt + pad4(nr)) >> 2;
		const values = words.subarray(off, (off += nr));
		const sonarIdx = words.subarray(off, (off += nk));
		const sonarColor = words.subarray(off, (off += nk * a));

		sonarIdx.forEach((center, k) =>
			this.diamond(center, r).forEach((pix, j) => {
				if (pix !== null) this.seen.set(pix, sonarColor[k * a + j]);
			}),
		);
		for (let i = 0; i < nr; i++) this.results.push([p[statusAt + i], values[i]]);
		// Brush entries apply after the results. This script binds only slot 0 and
		// never guesses its brush, so the latest entry is the brush.
		for (let i = 0; i < nb; i++) {
			if (p.readUInt16LE(brushSlotAt + 2 * i) !== 0) continue;
			const t = Number(p.readBigUInt64LE(32 + 8 * i));
			this.brush = t === 0 ? undefined : { idx: p.readUInt32LE(brushIdxAt + 4 * i), t }; // t = 0: no brush
		}
		this.waiter?.();
	}

	/** Pixel index of each sonar cell in wire order, or null for cells off the board. */
	private diamond(center: number, r: number): (number | null)[] {
		const { w, h } = this.rules!;
		const x = center % w;
		const y = Math.floor(center / w);
		const out: (number | null)[] = [];
		for (let dy = -r; dy <= r; dy++) {
			const span = r - Math.abs(dy);
			for (let dx = -span; dx <= span; dx++) {
				const inside = x + dx >= 0 && x + dx < w && y + dy >= 0 && y + dy < h;
				out.push(inside ? (y + dy) * w + x + dx : null);
			}
		}
		return out;
	}

	private async nextResult(): Promise<[number, number]> {
		while (this.results.length === 0) {
			const { promise, resolve } = Promise.withResolvers<void>();
			this.waiter = resolve;
			await promise;
		}
		return this.results.shift()!;
	}

	private waitMs(idx: number): number {
		if (!this.brush) return 0;
		const { w, c0, k } = this.rules!;
		const d = Math.abs((idx % w) - (this.brush.idx % w)) + Math.abs(Math.floor(idx / w) - Math.floor(this.brush.idx / w));
		const ready = this.brush.t + Number(c0 + k * BigInt(d));
		return Math.max(0, (ready - (Date.now() * 1000 + this.clockOffsetUs)) / 1000);
	}

	*targets(): Generator<number> {
		const [x0, y0, w, h] = this.rect;
		for (let row = 0; row < h; row++) {
			for (let i = 0; i < w; i++) {
				const col = row % 2 === 0 ? i : w - 1 - i; // serpentine: every step has distance 1
				yield (y0 + row) * this.rules!.w + x0 + col;
			}
		}
	}

	async run() {
		this.ws.send(encodeBatch([[0, this.paintToken]], []));
		const [bindStatus] = await this.nextResult();
		if (bindStatus !== 0) throw new Error(`bind failed: ${STATUS[bindStatus]}`);
		for (const idx of this.targets()) {
			if (this.seen.get(idx) === this.color) continue;
			for (;;) {
				// Re-check after sleeping: a paint elsewhere may have moved the brush meanwhile.
				for (let ms = this.waitMs(idx); ms > 0; ms = this.waitMs(idx)) await sleep(ms);
				this.ws.send(encodeBatch([], [[0, idx, this.color]]));
				const [status, value] = await this.nextResult();
				if (status === COOLING) {
					await sleep(value / 1000);
					continue;
				}
				if (status !== 0) throw new Error(`paint rejected: ${STATUS[status]}`);
				console.log(`painted (${idx % this.rules!.w}, ${Math.floor(idx / this.rules!.w)})`);
				break;
			}
		}
	}
}

const { values } = parseArgs({
	options: {
		server: { type: "string", default: "http://localhost:8775" },
		"paint-token": { type: "string" },
		rect: { type: "string" },
		color: { type: "string", default: "#000000" },
	},
});
const rect = (values.rect ?? "").split(",").map(Number);
if (!values["paint-token"] || rect.length !== 4 || rect.some(Number.isNaN)) {
	console.error("usage: node painter.ts --paint-token <uuid> --rect X,Y,W,H [--color #rrggbb] [--server URL]");
	process.exit(1);
}

const paintToken = Uint8Array.from(Buffer.from(values["paint-token"].replaceAll("-", ""), "hex"));
const rgb = Number.parseInt(values.color!.slice(1), 16);
const color = ((rgb >> 16) | (rgb & 0xff00) | ((rgb & 0xff) << 16) | 0xff000000) >>> 0;

const ws = new WebSocket(`${values.server!.replace(/^http/, "ws")}/ws?public=0`);
ws.binaryType = "arraybuffer";
const painter = new Painter(ws, paintToken, rect as [number, number, number, number], color);
ws.onmessage = (e) => painter.onMessage(e.data as ArrayBuffer);
ws.onclose = (e) => console.log(`closed: ${e.code} ${e.reason}`);
ws.onopen = async () => {
	while (!painter.rules) await sleep(10);
	await painter.run();
	ws.close();
};
