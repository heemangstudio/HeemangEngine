// Reference player for hm-wasm page packets: decodes a packet (see
// crates/hm-wasm/src/packet.rs) and paints it on a Canvas 2D context with
// glyph outlines as Path2D. Coordinates are page HU (7200 per inch).

const MAGIC = 0x4b504d48; // "HMPK"
export const PACKET_VERSION = 1;

class Reader {
  constructor(bytes, offset = 0, end = bytes.byteLength) {
    this.v = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
    this.at = offset;
    this.end = end;
  }
  need(n) {
    if (this.at + n > this.end) throw new Error(`packet truncated at ${this.at}`);
  }
  u8() { this.need(1); return this.v.getUint8(this.at++); }
  u16() { this.need(2); const x = this.v.getUint16(this.at, true); this.at += 2; return x; }
  u32() { this.need(4); const x = this.v.getUint32(this.at, true); this.at += 4; return x; }
  i32() { this.need(4); const x = this.v.getInt32(this.at, true); this.at += 4; return x; }
  f32() { this.need(4); const x = this.v.getFloat32(this.at, true); this.at += 4; return x; }
  count(size) {
    const n = this.u32();
    if (n * size > this.end - this.at) throw new Error(`count ${n} overruns the block`);
    return n;
  }
}

const tagName = (v, at) =>
  String.fromCharCode(v.getUint8(at), v.getUint8(at + 1), v.getUint8(at + 2), v.getUint8(at + 3));

function glyphs(r) {
  const run = {
    face: r.u32(), size: r.i32(), ratio: r.i32(), color: r.u32(), flags: r.u8(), baseline: r.i32(),
  };
  const n = r.count(6);
  run.glyphs = new Array(n);
  for (let k = 0; k < n; k++) run.glyphs[k] = [r.u16(), r.i32()];
  return run;
}

function paint(r) {
  const n = r.count(1);
  const ops = new Array(n);
  for (let k = 0; k < n; k++) {
    const tag = r.u8();
    switch (tag) {
      case 1: ops[k] = { op: 'glyphs', ...glyphs(r) }; break;
      case 2: ops[k] = { op: 'rect', x: r.i32(), y: r.i32(), w: r.i32(), h: r.i32(), color: r.u32() }; break;
      case 3: ops[k] = { op: 'line', x1: r.i32(), y1: r.i32(), x2: r.i32(), y2: r.i32(), width: r.i32(), color: r.u32(), dash: r.u8() }; break;
      case 4: {
        const p = { op: 'path', fill: r.u32(), width: r.i32(), stroke: r.u32(), dash: r.u8(), roundCap: r.u8() === 1, alpha: r.u8() };
        const m = r.count(1);
        const els = [];
        for (let j = 0; j < m; j++) {
          const verb = r.u8();
          const len = [2, 2, 6, 0][verb];
          if (len === undefined) throw new Error(`unknown path verb ${verb}`);
          const el = [verb];
          for (let q = 0; q < len; q++) el.push(r.i32());
          els.push(el);
        }
        p.els = els;
        ops[k] = p;
        break;
      }
      case 5: ops[k] = { op: 'patch', item: r.u32(), x: r.i32(), y: r.i32(), w: r.i32(), h: r.i32() }; break;
      case 6: {
        const run = glyphs(r);
        const width = r.i32();
        const m = r.count(4);
        const dash = [];
        for (let j = 0; j < m; j++) dash.push(r.i32());
        ops[k] = { op: 'outline', ...run, width, dash };
        break;
      }
      case 7: ops[k] = { op: 'clip', x: r.i32(), y: r.i32(), w: r.i32(), h: r.i32() }; break;
      case 8: ops[k] = { op: 'unclip' }; break;
      default: throw new Error(`unknown paint op ${tag} at ${r.at - 1}`);
    }
  }
  return ops;
}

function lines(r) {
  const lists = [];
  for (let n = r.count(8), k = 0; k < n; k++) {
    const section = r.u32();
    const steps = [];
    for (let m = r.count(12), j = 0; j < m; j++) steps.push({ para: r.u32(), control: r.u32(), index: r.u32() });
    lists.push({ section, steps });
  }
  const out = [];
  for (let n = r.count(28), k = 0; k < n; k++) {
    const list = r.u32();
    if (list >= lists.length) throw new Error(`line names list ${list}`);
    const line = { list: lists[list], para: r.u32(), line: r.u32(), y: r.i32(), height: r.i32(), baseline: r.i32() };
    const stops = [];
    for (let m = r.count(8), j = 0; j < m; j++) stops.push([r.u32(), r.i32()]);
    line.stops = stops;
    out.push(line);
  }
  return out;
}

function glyphList(r) {
  const n = r.count(8);
  const out = new Uint32Array(n * 2);
  for (let k = 0; k < n * 2; k++) out[k] = r.u32();
  return out;
}

/** Decodes and validates a page packet. Unknown blocks are kept raw. */
export function decodePacket(bytes) {
  const r = new Reader(bytes);
  if (r.u32() !== MAGIC) throw new Error('not a page packet');
  const version = r.u16();
  if (version !== PACKET_VERSION) throw new Error(`packet version ${version}, player reads ${PACKET_VERSION}`);
  r.u16();
  const header = {
    version, page: r.u32(), contentEpoch: r.u32(), layoutEpoch: r.u32(), fontGeneration: r.u32(),
    width: r.i32(), height: r.i32(),
  };
  const blocks = {};
  for (let n = r.count(12), k = 0; k < n; k++) {
    const tag = tagName(r.v, r.at);
    r.at += 4;
    const offset = r.u32();
    const length = r.u32();
    if (offset + length > bytes.byteLength) throw new Error(`block ${tag} overruns the packet`);
    blocks[tag] = { offset, length };
  }
  const read = (tag, f) => {
    const b = blocks[tag];
    if (!b) return undefined;
    const br = new Reader(bytes, b.offset, b.offset + b.length);
    const v = f(br);
    if (br.at !== br.end) throw new Error(`block ${tag} has ${br.end - br.at} bytes left over`);
    return v;
  };
  return {
    ...header,
    ops: read('PANT', paint) ?? [],
    lines: read('LINE', lines) ?? [],
    glyphs: read('GLYF', glyphList) ?? new Uint32Array(0),
  };
}

/** Decodes `Engine.glyphOutlines` output into `Map("face:glyph" -> {em, cmds})`. */
export function decodeOutlines(bytes, into = new Map()) {
  const r = new Reader(bytes);
  for (let n = r.count(16), k = 0; k < n; k++) {
    const face = r.u32();
    const glyph = r.u32();
    const em = r.f32();
    const m = r.count(4);
    const cmds = new Float32Array(m);
    for (let j = 0; j < m; j++) cmds[j] = r.f32();
    into.set(`${face}:${glyph}`, { em, cmds });
  }
  return into;
}

const DASHES = [null, [8, 4], [1, 2], [8, 3, 1, 3], [8, 3, 1, 3, 1, 3], [16, 6], [0, 2.5]];

export const css = colorref =>
  `rgb(${colorref & 255},${(colorref >> 8) & 255},${(colorref >> 16) & 255})`;

/**
 * Paints decoded packets. `engine` supplies glyph outlines; `patch(page,
 * item, dpi)` returns pixel patches (`Doc.patch`).
 */
export class Player {
  constructor(engine) {
    this.engine = engine;
    this.outlines = new Map();
    this.paths = new Map();
    this.generation = -1;
  }

  /** Fetches the outlines a packet needs that are not cached yet. */
  load(packet) {
    if (packet.fontGeneration !== this.generation) {
      this.outlines.clear();
      this.paths.clear();
      this.generation = packet.fontGeneration;
    }
    const g = packet.glyphs;
    const missing = [];
    for (let k = 0; k < g.length; k += 2) {
      if (!this.outlines.has(`${g[k]}:${g[k + 1]}`)) missing.push(g[k], g[k + 1]);
    }
    if (missing.length) decodeOutlines(this.engine.glyphOutlines(new Uint32Array(missing)), this.outlines);
  }

  glyphPath(face, glyph) {
    const key = `${face}:${glyph}`;
    let p = this.paths.get(key);
    if (p) return p;
    const o = this.outlines.get(key);
    p = { em: o?.em ?? 0, path: new Path2D() };
    const c = o?.cmds ?? [];
    for (let k = 0; k < c.length;) {
      switch (c[k++]) {
        case 0: p.path.moveTo(c[k], c[k + 1]); k += 2; break;
        case 1: p.path.lineTo(c[k], c[k + 1]); k += 2; break;
        case 2: p.path.quadraticCurveTo(c[k], c[k + 1], c[k + 2], c[k + 3]); k += 4; break;
        case 3: p.path.bezierCurveTo(c[k], c[k + 1], c[k + 2], c[k + 3], c[k + 4], c[k + 5]); k += 6; break;
        default: p.path.closePath();
      }
    }
    this.paths.set(key, p);
    return p;
  }

  /** The device path of a glyph run at `s` pixels per HU. */
  runPath(run, s) {
    const out = new Path2D();
    const skew = run.flags & 2 ? 0.2 : 0;
    for (const [glyph, x] of run.glyphs) {
      const { em, path } = this.glyphPath(run.face, glyph);
      if (!em) continue;
      const k = (run.size * s) / em;
      const m = new DOMMatrix([(k * run.ratio) / 100, 0, k * skew, -k, x * s, run.baseline * s]);
      out.addPath(path, m);
    }
    return out;
  }

  /**
   * Paints a packet on `ctx` at `dpi`, the canvas already sized and
   * cleared. `patch(item, dpi)` returns ImageData-ready bytes or null.
   */
  paint(ctx, packet, dpi, patch) {
    this.load(packet);
    const s = dpi / 7200;
    const px = v => v * s;
    const stroke = (width, color, dash, round) => {
      const w = Math.max(px(width), 0.5);
      ctx.lineWidth = w;
      ctx.strokeStyle = css(color);
      const d = DASHES[dash];
      ctx.setLineDash(d ? d.map(v => Math.max(v * w, 0.01)) : []);
      ctx.lineCap = dash === 6 || round ? 'round' : 'butt';
    };
    ctx.save();
    ctx.setTransform(1, 0, 0, 1, 0, 0);
    let depth = 0;
    for (const op of packet.ops) {
      switch (op.op) {
        case 'glyphs': {
          const path = this.runPath(op, s);
          ctx.fillStyle = css(op.color);
          ctx.fill(path, 'nonzero');
          if (op.flags & 1) {
            ctx.lineWidth = (op.size * s) / (op.flags & 4 ? 1000 : 40);
            ctx.strokeStyle = ctx.fillStyle;
            ctx.setLineDash([]);
            ctx.lineJoin = 'miter';
            ctx.lineCap = 'butt';
            ctx.stroke(path);
          }
          break;
        }
        case 'outline': {
          const path = this.runPath(op, s);
          ctx.lineWidth = Math.max(px(op.width), 0.5);
          ctx.strokeStyle = css(op.color);
          ctx.setLineDash(op.dash.map(v => Math.max(px(v), 0.01)));
          ctx.lineCap = 'round';
          ctx.lineJoin = 'round';
          ctx.stroke(path);
          break;
        }
        case 'rect':
          ctx.fillStyle = css(op.color);
          ctx.fillRect(px(op.x), px(op.y), Math.max(px(op.w), 0.5), Math.max(px(op.h), 0.5));
          break;
        case 'line':
          ctx.beginPath();
          ctx.moveTo(px(op.x1), px(op.y1));
          ctx.lineTo(px(op.x2), px(op.y2));
          stroke(op.width, op.color, op.dash, false);
          ctx.stroke();
          break;
        case 'path': {
          const p = new Path2D();
          for (const e of op.els) {
            if (e[0] === 0) p.moveTo(px(e[1]), px(e[2]));
            else if (e[0] === 1) p.lineTo(px(e[1]), px(e[2]));
            else if (e[0] === 2) p.bezierCurveTo(px(e[1]), px(e[2]), px(e[3]), px(e[4]), px(e[5]), px(e[6]));
            else p.closePath();
          }
          if (op.fill !== 0xff000000) {
            ctx.globalAlpha = (255 - op.alpha) / 255;
            ctx.fillStyle = css(op.fill);
            ctx.fill(p, 'evenodd');
            ctx.globalAlpha = 1;
          }
          if (op.width >= 0) {
            stroke(op.width, op.stroke, op.dash, op.roundCap);
            ctx.lineJoin = 'miter';
            ctx.stroke(p);
          }
          break;
        }
        case 'patch': {
          const img = patch && patch(op.item, dpi);
          if (img) ctx.drawImage(img, px(op.x), px(op.y), px(op.w), px(op.h));
          break;
        }
        case 'clip':
          ctx.save();
          depth++;
          ctx.beginPath();
          ctx.rect(px(op.x), px(op.y), px(op.w), px(op.h));
          ctx.clip();
          break;
        case 'unclip':
          if (depth > 0) {
            ctx.restore();
            depth--;
          }
          break;
      }
    }
    while (depth-- > 0) ctx.restore();
    ctx.restore();
  }
}

/** A canvas image from `Doc.patch` bytes, or null when empty. */
export function patchImage(bytes, makeCanvas) {
  if (!bytes || bytes.byteLength < 8) return null;
  const v = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const w = v.getUint32(0, true);
  const h = v.getUint32(4, true);
  if (!w || !h) return null;
  const canvas = makeCanvas(w, h);
  const ctx = canvas.getContext('2d');
  ctx.putImageData(new ImageData(new Uint8ClampedArray(bytes.buffer, bytes.byteOffset + 8, w * h * 4), w, h), 0, 0);
  return canvas;
}
