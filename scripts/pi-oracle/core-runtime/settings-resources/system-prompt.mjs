// Golden fixtures for oino-resources system prompt: pi buildSystemPromptSections/State/Text with
// the D13 substitutions (runtime §9.4) applied to pi's output.
// Usage: node system-prompt.mjs
import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { importPi, REPO } from "./lib.mjs";

const sp = await importPi("core/system-prompt.js");
const config = await importPi("config.js");
const fixtures = join(REPO, "crates/oino-resources/tests/fixtures/pi");
const cases = JSON.parse(readFileSync(join(fixtures, "system-prompt-cases.json"), "utf-8"));
const DOCS_ROOT = "/DOCS";

const piPreamble =
  "You are an expert coding assistant operating inside pi, a coding agent harness. You help users by reading files, executing commands, editing code, and writing new files.";
const oinoPreamble =
  "You are an expert coding assistant operating inside Oino, a coding agent harness. You help users by reading files, executing commands, editing code, and writing new files.";
const piDocs = `Pi documentation (read only when the user asks about pi itself, its SDK, extensions, themes, skills, or TUI):
- Main documentation: ${config.getReadmePath()}
- Additional docs: ${config.getDocsPath()}
- Examples: ${config.getExamplesPath()} (extensions, custom tools, SDK)
- When reading pi docs or examples, resolve docs/... under Additional docs and examples/... under Examples, not the current working directory
- When asked about: extensions (docs/extensions.md, examples/extensions/), themes (docs/themes.md), skills (docs/skills.md), prompt templates (docs/prompt-templates.md), TUI components (docs/tui.md), keybindings (docs/keybindings.md), SDK integrations (docs/sdk.md), custom providers (docs/custom-provider.md), adding models (docs/models.md), pi packages (docs/packages.md), environment variables (docs/environment-variables.md), MCP servers (docs/mcp.md), codemode scripts and non-LLM models such as classifiers and image models (docs/codemode.md)
- When working on pi topics, read the docs and examples, and follow .md cross-references before implementing
- Always read pi .md files completely and follow links to related docs (e.g., tui.md for TUI API details)`;
// runtime.md §9.4, verbatim, with {docs_root} = DOCS_ROOT.
const oinoDocs = `Oino documentation (read only when the user asks about Oino itself, its SDK, extensions, themes, skills, or TUI):
- Main documentation: ${DOCS_ROOT}/README.md
- Additional docs: ${DOCS_ROOT}/docs
- Examples: ${DOCS_ROOT}/examples (extensions, custom tools, SDK)
- When reading Oino docs or examples, resolve docs/... under Additional docs and examples/... under Examples, not the current working directory
- When asked about: extensions (docs/extensions.md, examples/extensions/), extension API and WIT (docs/extension-api.md), themes (docs/themes.md), skills (docs/skills.md), prompt templates (docs/prompt-templates.md), TUI (docs/tui.md), keybindings (docs/keybindings.md), SDK integrations (docs/sdk.md), providers and models (docs/providers.md, docs/models.md), Oino packages (docs/packages.md), environment variables (docs/environment-variables.md), RPC and ACP (docs/rpc.md, docs/acp.md)
- When working on Oino topics, read the docs and examples, and follow .md cross-references before implementing
- Always read Oino .md files completely and follow links to related docs (e.g., tui.md for TUI details)`;
const piEnv = "You can inspect PI_* environment variables for current model and session details.";
const oinoEnv = "You can inspect OINO_* environment variables (also exported as PI_*) for current model and session details.";

const substitute = (text) =>
  text.replaceAll(piPreamble, oinoPreamble).replaceAll(piDocs, oinoDocs).replaceAll(piEnv, oinoEnv);
const substituteInput = (value) => JSON.parse(JSON.stringify(value).replaceAll(JSON.stringify(piEnv).slice(1, -1), JSON.stringify(oinoEnv).slice(1, -1)));

const out = [];
for (const c of cases) {
  let result;
  try {
    const sections = sp.buildSystemPromptSections(c.options);
    const state = sp.buildSystemPromptState(c.options);
    const text = sp.buildSystemPrompt(c.options);
    const sub = (obj) => obj && Object.fromEntries(Object.entries(obj).map(([k, v]) => [k, substitute(v)]));
    result = {
      sections: sub(sections),
      state: { content: substitute(state.content), ...(state.sections ? { sections: sub(state.sections) } : {}) },
      text: substitute(text),
      normalized: substituteInput(sp.normalizeBuildSystemPromptOptions(c.options)),
    };
  } catch (error) {
    result = { error: error.message };
  }
  out.push({ name: c.name, options: substituteInput(c.options), ...result });
}
writeFileSync(join(fixtures, "system-prompt-expected.json"), `${JSON.stringify(out, null, 2)}\n`);
console.log(`wrote ${out.length} cases`);
