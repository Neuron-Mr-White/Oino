// Golden generator: pi AgentSession (createAgentSession) with pi-ai's faux provider over
// scripted scenarios; records every session event (JSON) after normalisation.
// Usage: node session-events.mjs > ../../../../crates/oino-core/tests/fixtures/session/events.json
import { mkdtempSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
const PI = process.env.PI_CODING_AGENT_PKG ?? `${process.env.HOME}/.local/share/mise/installs/node/24.16.0/lib/node_modules/@earendil-works/pi-coding-agent`;
const pi = await import(pathToFileURL(`${PI}/dist/index.js`).href);
const ai = await import(pathToFileURL(`${PI}/node_modules/@earendil-works/pi-ai/dist/index.js`).href);
const scenarios = JSON.parse(readFileSync(new URL("./scenarios.json", import.meta.url), "utf8"));

function block(spec) {
  if (spec.type === "text") return ai.fauxText(spec.text);
  if (spec.type === "thinking") return ai.fauxThinking(spec.thinking);
  return ai.fauxToolCall(spec.name, spec.arguments, { id: spec.id });
}

const out = [];
for (const scenario of scenarios) {
  const cwd = mkdtempSync(join(tmpdir(), "oino-oracle-"));
  const agentDir = join(cwd, "agent");
  const faux = ai.fauxProvider({ provider: "faux", api: "faux", models: [{ id: "faux-1", reasoning: true, contextWindow: 128000, maxTokens: 16384 }], tokensPerSecond: 0 });
  faux.setResponses(scenario.responses.map((r) => ai.fauxAssistantMessage(r.content.map(block), { stopReason: r.stopReason, errorMessage: r.errorMessage })));
  const modelRuntime = await pi.ModelRuntime.create({ authPath: join(agentDir, "auth.json"), modelsPath: join(agentDir, "models.json") });
  modelRuntime.registerNativeProvider(faux.provider);
  const settingsManager = pi.SettingsManager.inMemory(scenario.settings ?? {});
  const resourceLoader = new pi.DefaultResourceLoader({ cwd, agentDir, settingsManager, noSkills: true, noPromptTemplates: true, noThemes: true, noContextFiles: true, noExtensions: true });
  await resourceLoader.reload();
  const { session } = await pi.createAgentSession({
    cwd, agentDir, modelRuntime, settingsManager, resourceLoader,
    sessionManager: pi.SessionManager.inMemory(cwd),
    model: faux.getModel(), thinkingLevel: "off", tools: scenario.tools ?? [],
  });
  const events = [];
  session.subscribe((event) => events.push(JSON.parse(JSON.stringify(event))));
  for (const prompt of scenario.prompts) {
    await session.prompt(prompt);
    await session.waitForIdle();
  }
  out.push({ name: scenario.name, events, entries: session.sessionManager.getEntries() });
  session.dispose();
}
console.log(JSON.stringify(out, null, 2));
