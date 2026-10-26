// Golden generator: metadata of every pi built-in tool definition.
// Usage: node meta.mjs > ../../../../crates/oino-tools/tests/fixtures/meta.json
import { importPi } from "./lib.mjs";

const { createAllToolDefinitions } = await importPi("core/tools/index.js");
const defs = createAllToolDefinitions("/cwd");
const out = {};
for (const [name, d] of Object.entries(defs)) {
  out[name] = {
    name: d.name,
    label: d.label,
    description: d.description,
    promptSnippet: d.promptSnippet ?? null,
    promptGuidelines: d.promptGuidelines ?? null,
    parameters: JSON.parse(JSON.stringify(d.parameters)),
    outputSchema: d.outputSchema ? JSON.parse(JSON.stringify(d.outputSchema)) : null,
    constrainedSampling: d.constrainedSampling ?? null,
    executionMode: d.executionMode ?? null,
    hasPrepareArguments: typeof d.prepareArguments === "function",
  };
}
console.log(JSON.stringify(out, null, 2));
