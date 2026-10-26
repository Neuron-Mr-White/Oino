// Golden fixtures for oino-resources ResourceLoader: builds fixture trees, runs pi's
// DefaultResourceLoader (no extensions, no themes) and records skills, prompts, context files,
// SYSTEM/APPEND and diagnostics with paths normalised to $ROOT and `.pi` → `.oino`.
// Usage: node resources.mjs
import { mkdirSync, readFileSync, realpathSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { importPi, REPO, scratchDir } from "./lib.mjs";

const { DefaultResourceLoader } = await importPi("core/resource-loader.js");
const { SettingsManager } = await importPi("core/settings-manager.js");
const fixtures = join(REPO, "crates/oino-resources/tests/fixtures/pi");
const spec = JSON.parse(readFileSync(join(fixtures, "resources-cases.json"), "utf-8"));

function build(root, tree) {
  for (const [rel, content] of Object.entries(tree)) {
    const path = join(root, rel.replaceAll("{CONFIG}", ".pi"));
    mkdirSync(dirname(path), { recursive: true });
    writeFileSync(path, content);
  }
}

const out = [];
for (const c of spec.cases) {
  const root = realpathSync(scratchDir("resources"));
  build(root, spec.trees[c.tree]);
  const agentDir = join(root, "agent");
  const home = join(root, "home");
  mkdirSync(home, { recursive: true });
  process.env.HOME = home;
  const cwd = join(root, c.cwd ?? "work/proj");
  if (c.globalSettings) writeFileSync(join(agentDir, "settings.json"), JSON.stringify(c.globalSettings));
  if (c.projectSettings) {
    mkdirSync(join(cwd, ".pi"), { recursive: true });
    writeFileSync(join(cwd, ".pi", "settings.json"), JSON.stringify(c.projectSettings));
  }
  const prevCwd = process.cwd();
  process.chdir(cwd);
  const settingsManager = SettingsManager.create(cwd, agentDir, { projectTrusted: c.trusted });
  const loader = new DefaultResourceLoader({
    cwd,
    agentDir,
    settingsManager,
    noExtensions: true,
    noThemes: true,
    noSkills: c.noSkills,
    noPromptTemplates: c.noPromptTemplates,
    noContextFiles: c.noContextFiles,
    additionalSkillPaths: c.skillPaths,
    additionalPromptTemplatePaths: c.promptPaths,
    systemPrompt: c.systemPrompt,
    appendSystemPrompt: c.appendSystemPrompt,
  });
  await loader.reload();
  process.chdir(prevCwd);
  const result = {
    name: c.name,
    skills: loader.getSkills(),
    prompts: loader.getPrompts(),
    agentsFiles: loader.getAgentsFiles().agentsFiles,
    systemPrompt: loader.getSystemPrompt() ?? null,
    systemPromptSource: loader.getSystemPromptSource()?.path ?? null,
    appendSystemPrompt: loader.getAppendSystemPrompt(),
    appendSystemPromptSources: loader.getAppendSystemPromptSources().map((s) => s.path),
  };
  const text = JSON.stringify(result)
    .replaceAll(root, "$ROOT")
    .replaceAll("/.pi/", "/.oino/")
    .replaceAll('/.pi"', '/.oino"');
  out.push(JSON.parse(text));
}
writeFileSync(join(fixtures, "resources-expected.json"), `${JSON.stringify(out, null, 2)}\n`);
console.log(`wrote ${out.length} cases`);
