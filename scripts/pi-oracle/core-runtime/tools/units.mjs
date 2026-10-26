// Golden generator: pure helpers (truncate, formatSize, path resolution, diffs, fuzzy
// normalisation). Usage: node units.mjs > ../../../../crates/oino-tools/tests/fixtures/units.json
import { importPi } from "./lib.mjs";

const truncate = await importPi("core/tools/truncate.js");
const paths = await importPi("core/tools/path-utils.js");
const diff = await importPi("core/tools/edit-diff.js");

// Deterministic PRNG (mulberry32); must match nothing on the Rust side: inputs are recorded.
function rng(seed) {
  return () => {
    seed |= 0;
    seed = (seed + 0x6d2b79f5) | 0;
    let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

const sizes = [0, 1, 1023, 1024, 1280, 1536, 10239, 51200, 52224, 1048575, 1048576, 1572864, 5 * 1024 * 1024 + 1];
const formatSize = sizes.map((bytes) => ({ bytes, text: truncate.formatSize(bytes) }));

const texts = [
  "", "a", "a\n", "a\nb", "a\nb\n", "\n\n\n", "x".repeat(100), "é".repeat(30) + "\nz",
  Array.from({ length: 30 }, (_, i) => `line ${i}`).join("\n"),
  Array.from({ length: 30 }, (_, i) => `line ${i}`).join("\n") + "\n",
  "short\n" + "y".repeat(80) + "\nend", "😀😀😀\nab😀",
];
const limits = [{}, { maxLines: 5 }, { maxBytes: 20 }, { maxLines: 3, maxBytes: 50 }, { maxBytes: 7 }, { maxLines: 1, maxBytes: 1 }];
const truncation = [];
for (const text of texts) {
  for (const options of limits) {
    truncation.push({ text, options, head: truncate.truncateHead(text, options), tail: truncate.truncateTail(text, options) });
  }
}
const lines = ["", "abc", "x".repeat(500), "x".repeat(501), "é".repeat(600), "😀".repeat(260)].map((line) => ({
  line, result: truncate.truncateLine(line),
}));

const cwd = "/work/project";
const pathInputs = ["a.txt", "./a/../b", "/abs//x/", "@rel", "@/abs", "a\u00a0b", "a\u2003b", "~", "~/notes.md",
  "file:///tmp/a%20b", "../..", "../../../../..", "", ".", "dir/", "x/./y/../z"];
const resolve = pathInputs.map((input) => ({ input, cwd, resolved: paths.resolveToCwd(input, cwd), expanded: paths.expandPath(input) }));

const fuzzy = ["plain", "trail   \nx\t\n", "\u201cq\u201d \u2018s\u2019", "a\u2014b\u2013c\u2212d", "nb\u00a0sp\u3000x", "\uff21\uff42\u2460", "\ufb01ne"]
  .map((text) => ({ text, normalized: diff.normalizeForFuzzyMatch(text) }));

const random = rng(42);
const words = ["alpha", "beta", "gamma", "delta", "", "eps", "zeta"];
const diffs = [];
for (let i = 0; i < 160; i++) {
  const length = Math.floor(random() * 30);
  const old = Array.from({ length }, () => words[Math.floor(random() * words.length)]);
  const next = [];
  for (const line of old) {
    const roll = random();
    if (roll < 0.15) continue;
    if (roll < 0.3) next.push(words[Math.floor(random() * words.length)]);
    else next.push(line);
    if (random() < 0.1) next.push(`new${i}`);
  }
  const oldText = old.join("\n") + (random() < 0.5 ? "\n" : "");
  const newText = next.join("\n") + (random() < 0.5 ? "\n" : "");
  if (oldText === newText) continue;
  diffs.push({
    old: oldText,
    new: newText,
    display: diff.generateDiffString(oldText, newText),
    patch: diff.generateUnifiedPatch("f.txt", oldText, newText),
  });
}
const home = JSON.stringify(process.env.HOME).slice(1, -1);
console.log(JSON.stringify({ formatSize, truncation, lines, resolve, fuzzy, diffs }, null, 1).replaceAll(`"${home}`, '"$HOME'));
