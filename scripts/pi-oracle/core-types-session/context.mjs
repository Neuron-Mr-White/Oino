// Writes <name>.expected.json (pi buildSessionContext/projection/tree per leaf) for every
// <name>.jsonl in a fixture dir. Files are opened from temp copies (pi migrates in place).
// Usage: node context.mjs <fixture-dir>
import { copyFileSync, mkdtempSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { loadSessionManager, snapshot } from "./lib.mjs";

const { SessionManager } = await loadSessionManager();
const dir = process.argv[2];
const work = mkdtempSync(join(dir, ".ctx-"));
for (const file of readdirSync(dir).filter((f) => f.endsWith(".jsonl")).sort()) {
  const copy = join(work, file);
  copyFileSync(join(dir, file), copy);
  const sm = SessionManager.open(copy, work);
  const name = file.replace(/\.jsonl$/, "");
  writeFileSync(join(dir, `${name}.expected.json`), `${JSON.stringify(snapshot(sm))}\n`);
}
rmSync(work, { recursive: true, force: true });
