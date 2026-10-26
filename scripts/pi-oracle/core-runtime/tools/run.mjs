// Golden generator: runs pi's real built-in tool definitions over cases.json.
// Usage (from this directory):
//   node run.mjs > ../../../../crates/oino-tools/tests/fixtures/tools.json
// Needs `rg` and `fd` where pi finds them (PATH or ~/.pi/agent/bin).
import fs from "node:fs";
import path from "node:path";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";
import { PI, importPi } from "./lib.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const repo = path.resolve(here, "../../../..");
const root = path.join(repo, "target/oino-tools/oracle");
const tmp = path.join(root, "tmp");
fs.rmSync(root, { recursive: true, force: true });
fs.mkdirSync(tmp, { recursive: true });
process.env.TMPDIR = tmp;

const tools = await importPi("core/tools/index.js");
const require = createRequire(`${PI}/package.json`);
const photon = require("@silvia-odwyer/photon-node");

const cases = JSON.parse(fs.readFileSync(path.join(here, "cases.json"), "utf8"));

function pixels(width, height) {
  const raw = new Uint8Array(width * height * 4);
  for (let y = 0; y < height; y++) {
    for (let x = 0; x < width; x++) {
      const i = (y * width + x) * 4;
      raw[i] = (x * 7) % 256;
      raw[i + 1] = (y * 13) % 256;
      raw[i + 2] = (x + y) % 256;
      raw[i + 3] = 255;
    }
  }
  return raw;
}

function bmp(width, height) {
  const rowSize = Math.ceil((width * 3) / 4) * 4;
  const size = 54 + rowSize * height;
  const buf = Buffer.alloc(size);
  buf.write("BM", 0);
  buf.writeUInt32LE(size, 2);
  buf.writeUInt32LE(54, 10);
  buf.writeUInt32LE(40, 14);
  buf.writeInt32LE(width, 18);
  buf.writeInt32LE(height, 22);
  buf.writeUInt16LE(1, 26);
  buf.writeUInt16LE(24, 28);
  buf.writeUInt32LE(rowSize * height, 34);
  const raw = pixels(width, height);
  for (let y = 0; y < height; y++) {
    for (let x = 0; x < width; x++) {
      const src = ((height - 1 - y) * width + x) * 4;
      const dst = 54 + y * rowSize + x * 3;
      buf[dst] = raw[src + 2];
      buf[dst + 1] = raw[src + 1];
      buf[dst + 2] = raw[src];
    }
  }
  return buf;
}

function image(spec) {
  if (spec.image === "bmp") return bmp(spec.width, spec.height);
  const img = new photon.PhotonImage(pixels(spec.width, spec.height), spec.width, spec.height);
  const bytes = spec.image === "jpeg" ? img.get_bytes_jpeg(80) : img.get_bytes();
  img.free();
  return Buffer.from(bytes);
}

/** FNV-1a 64 of the UTF-8 bytes, as 16 hex digits. Must match `fnv1a` in the Rust test. */
function fnv1a(text) {
  let hash = 0xcbf29ce484222325n;
  for (const byte of Buffer.from(text, "utf8")) {
    hash ^= BigInt(byte);
    hash = (hash * 0x100000001b3n) & 0xffffffffffffffffn;
  }
  return hash.toString(16).padStart(16, "0");
}

/** Replaces strings over 4096 chars by a digest. Must match `compact()` in the Rust test. */
function compact(value) {
  if (typeof value === "string") {
    if (value.length <= 4096) return value;
    return { $long: Buffer.byteLength(value, "utf8"), head: value.slice(0, 200), tail: value.slice(-200), fnv: fnv1a(value) };
  }
  if (Array.isArray(value)) return value.map(compact);
  if (value && typeof value === "object") return Object.fromEntries(Object.entries(value).map(([k, v]) => [k, compact(v)]));
  return value;
}

/** Must match `content()` in crates/oino-tools/tests/golden_tools.rs. */
function content(spec) {
  if (typeof spec === "string") return { bytes: Buffer.from(spec, "utf8") };
  if (spec.b64 !== undefined) return { bytes: Buffer.from(spec.b64, "base64") };
  if (spec.lines !== undefined) {
    const lines = [];
    for (let i = 1; i <= spec.lines; i++) lines.push(`${spec.prefix ?? ""}${spec.pad ? String(i).padStart(spec.pad, "0") : i}`);
    return { bytes: Buffer.from(`${lines.join("\n")}\n`, "utf8") };
  }
  if (spec.repeat !== undefined) return { bytes: Buffer.from(spec.repeat.repeat(spec.count), "utf8") };
  if (spec.concat !== undefined) return { bytes: Buffer.concat(spec.concat.map((part) => content(part).bytes)) };
  if (spec.image !== undefined) return { bytes: image(spec), materialized: true };
  throw new Error(`bad file spec ${JSON.stringify(spec)}`);
}

function imageDims(data) {
  try {
    const img = photon.PhotonImage.new_from_byteslice(Buffer.from(data, "base64"));
    const dims = { width: img.get_width(), height: img.get_height() };
    img.free();
    return dims;
  } catch {
    return null;
  }
}

function model(spec) {
  if (!spec) return undefined;
  return { provider: "prov", id: "mod-1", api: "faux", input: spec.input, inputLimits: spec.inputLimits };
}

const out = [];
for (const c of cases) {
  const dir = path.join(root, "cases", c.name);
  fs.mkdirSync(dir, { recursive: true });
  const materialized = {};
  for (const d of c.dirs ?? []) fs.mkdirSync(path.join(dir, d), { recursive: true });
  for (const [rel, spec] of Object.entries(c.files ?? {})) {
    const file = path.join(dir, rel);
    fs.mkdirSync(path.dirname(file), { recursive: true });
    const { bytes, materialized: isImage } = content(spec);
    fs.writeFileSync(file, bytes);
    // Small images are stored so Rust reads pi's exact bytes; larger ones are regenerated.
    if (isImage && bytes.length < 16384) materialized[rel] = bytes.toString("base64");
  }
  for (const [rel, target] of Object.entries(c.symlinks ?? {})) fs.symlinkSync(target, path.join(dir, rel));

  const cwd = c.ctx?.cwd ? path.join(dir, c.ctx.cwd) : dir;
  const ctx = {
    cwd,
    model: model(c.ctx?.model),
    sessionManager: { getSessionId: () => "sess-1", getSessionFile: () => undefined },
  };
  if (c.ctx?.session) {
    ctx.model = { provider: "prov", id: "mod-1", input: ["text"] };
    ctx.thinkingLevel = "high";
    ctx.sessionManager = { getSessionId: () => "sess-1", getSessionFile: () => "/s/file.jsonl" };
  }
  const definition = tools.createToolDefinition(c.tool, dir, { [c.tool]: c.options ?? {} });
  const rawArgs = JSON.parse(JSON.stringify(c.args).replaceAll("$CWD", dir));
  const args = definition.prepareArguments ? definition.prepareArguments(rawArgs) : rawArgs;
  const controller = new AbortController();
  if (c.abort === "before") controller.abort();
  if (c.abortAfterMs) setTimeout(() => controller.abort(), c.abortAfterMs);
  const saved = {};
  for (const [key, value] of Object.entries(c.env ?? {})) {
    saved[key] = process.env[key];
    process.env[key] = value;
  }
  let record;
  // pi runs rg/fd from the process cwd, which is normally the session cwd.
  process.chdir(fs.existsSync(cwd) ? cwd : dir);
  try {
    const result = await definition.execute("call-1", args, controller.signal, () => {}, ctx);
    record = {
      content: result.content,
      details: result.details ?? null,
      structuredContent: result.structuredContent ?? null,
      isError: result.isError ?? false,
    };
    const dims = result.content.filter((block) => block.type === "image").map((block) => imageDims(block.data));
    if (dims.length > 0) record.imageDims = dims;
  } catch (error) {
    record = { error: error.message };
  }
  for (const [key, value] of Object.entries(saved)) {
    if (value === undefined) delete process.env[key];
    else process.env[key] = value;
  }
  // Image data: `$SOURCE` when it is the file's own bytes, else `$ENCODED` (re-encoded bytes
  // differ between Photon and the image crate; `imageDims` carries the comparison).
  if (record.content?.some((block) => block.type === "image")) {
    const source = fs.readFileSync(path.resolve(cwd, args.path.replace(/^@/, ""))).toString("base64");
    const marker = (data) => (data === source ? "$SOURCE" : "$ENCODED");
    for (const block of record.content) if (block.type === "image") block.data = marker(block.data);
    if (record.structuredContent?.type === "image") record.structuredContent.data = marker(record.structuredContent.data);
  }
  if (record.structuredContent && typeof record.structuredContent.wall_time_seconds === "number") {
    record.structuredContent.wall_time_seconds = 0;
  }
  const files = {};
  for (const rel of c.check ?? []) {
    const file = path.join(dir, rel);
    files[rel] = fs.existsSync(file) ? fs.readFileSync(file).toString("base64") : null;
  }
  const text = JSON.stringify({ name: c.name, materialized, result: record, files })
    .replaceAll(JSON.stringify(dir).slice(1, -1), "$CWD")
    .replaceAll(JSON.stringify(tmp).slice(1, -1), "$TMP")
    .replace(/\$TMP\/pi-bash-[0-9a-f]{16}\.log/g, "$TMPLOG");
  const parsed = JSON.parse(text);
  out.push({ ...parsed, result: compact(parsed.result), files: compact(parsed.files) });
}
console.log(JSON.stringify(out, null, 1));
