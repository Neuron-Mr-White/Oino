// Golden fixtures for oino-settings: runs pi's SettingsManager over cases.json and records the
// merged/global/project settings, getter results, written files and errors.
// Usage: node settings.mjs   (writes crates/oino-settings/tests/fixtures/pi/settings-expected.json)
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import { importPi, REPO, scratchDir } from "./lib.mjs";

const { SettingsManager } = await importPi("core/settings-manager.js");
const fixtures = join(REPO, "crates/oino-settings/tests/fixtures/pi");
const cases = JSON.parse(readFileSync(join(fixtures, "settings-cases.json"), "utf-8"));

const GETTERS = {
  getDefaultProvider: (sm) => sm.getDefaultProvider(),
  getDefaultModel: (sm) => sm.getDefaultModel(),
  getDefaultThinkingLevel: (sm) => sm.getDefaultThinkingLevel(),
  getAllModelThinkingLevels: (sm) => sm.getAllModelThinkingLevels(),
  getSteeringMode: (sm) => sm.getSteeringMode(),
  getFollowUpMode: (sm) => sm.getFollowUpMode(),
  getDefaultTools: (sm) => sm.getDefaultTools(),
  getEnabledModels: (sm) => sm.getEnabledModels(),
  getThinkingBudgets: (sm) => sm.getThinkingBudgets(),
  getSessionDir: (sm) => sm.getSessionDir(),
  getCompactionSettings: (sm) => sm.getCompactionSettings(),
  getCompactionSettingsForModel: (sm) => sm.getCompactionSettings({ provider: "p", id: "m" }),
  getBranchSummarySettings: (sm) => sm.getBranchSummarySettings(),
  getRetrySettings: (sm) => sm.getRetrySettings(),
  getProviderRetrySettings: (sm) => sm.getProviderRetrySettings(),
  getTransport: (sm) => sm.getTransport(),
  getHttpIdleTimeoutMs: (sm) => sm.getHttpIdleTimeoutMs(),
  getWebSocketConnectTimeoutMs: (sm) => sm.getWebSocketConnectTimeoutMs(),
  getCacheWarmingMode: (sm) => sm.getCacheWarmingMode(),
  getShowCacheMissNotices: (sm) => sm.getShowCacheMissNotices(),
  getShellPath: (sm) => sm.getShellPath(),
  getShellCommandPrefix: (sm) => sm.getShellCommandPrefix(),
  getImageAutoResize: (sm) => sm.getImageAutoResize(),
  getBlockImages: (sm) => sm.getBlockImages(),
  getEnableSkillCommands: (sm) => sm.getEnableSkillCommands(),
  getDefaultProjectTrust: (sm) => sm.getDefaultProjectTrust(),
  getPackages: (sm) => sm.getPackages(),
  getExtensionPaths: (sm) => sm.getExtensionPaths(),
  getSkillPaths: (sm) => sm.getSkillPaths(),
  getPromptTemplatePaths: (sm) => sm.getPromptTemplatePaths(),
  getThemePaths: (sm) => sm.getThemePaths(),
  httpProxy: (sm) => sm.getGlobalSettings().httpProxy,
};

function run(fn) {
  try {
    const value = fn();
    return value === undefined ? { undefined: true } : { value };
  } catch (error) {
    return { error: error.message };
  }
}

const out = [];
for (const c of cases) {
  const root = scratchDir("settings");
  const agentDir = join(root, "agent");
  const cwd = join(root, "cwd");
  mkdirSync(agentDir, { recursive: true });
  mkdirSync(join(cwd, ".pi"), { recursive: true });
  const write = (path, value) => {
    if (value === undefined) return;
    writeFileSync(path, typeof value === "string" ? value : JSON.stringify(value, null, 2));
  };
  write(join(agentDir, "settings.json"), c.global);
  write(join(cwd, ".pi", "settings.json"), c.project);
  const sm = SettingsManager.create(cwd, agentDir, { projectTrusted: c.trusted ?? true });
  const opErrors = [];
  for (const [method, ...args] of c.ops ?? []) {
    try {
      sm[method](...args);
    } catch (error) {
      opErrors.push(`${method}: ${error.message}`);
    }
  }
  await sm.flush();
  const getters = {};
  for (const [name, fn] of Object.entries(GETTERS)) getters[name] = run(() => fn(sm));
  const read = (path) => (existsSync(path) ? readFileSync(path, "utf-8") : null);
  out.push({
    name: c.name,
    settings: sm.getSettings(),
    global: sm.getGlobalSettings(),
    project: sm.getProjectSettings(),
    getters,
    files: { global: read(join(agentDir, "settings.json")), project: read(join(cwd, ".pi", "settings.json")) },
    opErrors,
    errors: sm.drainErrors().map((e) => e.scope),
  });
}

const text = JSON.stringify(out, null, 2).replaceAll(homedir(), "$HOME");
writeFileSync(join(fixtures, "settings-expected.json"), `${text}\n`);
console.log(`wrote ${out.length} cases`);
