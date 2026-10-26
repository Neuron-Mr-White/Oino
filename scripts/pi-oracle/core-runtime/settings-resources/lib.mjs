// Shared helpers for the core-runtime settings/resources pi oracle (pi-coding-agent 1.4.0).
// PI_CODING_AGENT_PKG overrides the package root; default is the mise global install.
import { existsSync, mkdirSync, mkdtempSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const defaultPkg = join(
  homedir(),
  ".local/share/mise/installs/node/24.16.0/lib/node_modules/@earendil-works/pi-coding-agent",
);

export const PI_PKG = process.env.PI_CODING_AGENT_PKG ?? defaultPkg;
export const REPO = resolve(dirname(fileURLToPath(import.meta.url)), "../../../..");

/** Import a module of pi's dist tree, e.g. `core/settings-manager.js`. */
export async function importPi(relative) {
  const file = join(PI_PKG, "dist", relative);
  if (!existsSync(file)) throw new Error(`pi module not found: ${file}`);
  return import(pathToFileURL(file).href);
}

/** A fresh scratch dir under the repo's target/ (never /tmp: user quota). */
export function scratchDir(prefix) {
  const root = join(REPO, "target/tmp/pi-oracle");
  mkdirSync(root, { recursive: true });
  return mkdtempSync(join(root, `${prefix}-`));
}
