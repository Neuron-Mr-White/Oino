// Golden fixtures for oino-resources pure functions: parseCommandArgs, substituteArgs,
// expandPromptTemplate, formatSkillsForPrompt, parseSkillBlock, key ids, keybinding migration,
// frontmatter parsing.
// Usage: node misc.mjs
import { writeFileSync } from "node:fs";
import { join } from "node:path";
import { importPi, PI_PKG, REPO } from "./lib.mjs";
import { pathToFileURL } from "node:url";

const pt = await importPi("core/prompt-templates.js");
const skills = await importPi("core/skills.js");
const session = await importPi("core/agent-session.js");
const kb = await importPi("core/keybindings.js");
const keyIds = await importPi("core/key-id-schema.js");
const fm = await importPi("utils/frontmatter.js");
const { Compile } = await import(pathToFileURL(join(PI_PKG, "node_modules/typebox/build/compile/index.mjs")).href);
const validKeyBinding = Compile(keyIds.KeybindingValueSchema);

const argStrings = [
  "", "   ", "a b  c", "\"a b\" c", "'a \"b\" c' d", "\"\" ''", "x\"y z\"w", "\"unterminated rest", "tab\tsep\nnewline",
  "a\u00a0b", "  lead trail  ", "'single' \"double\"", "emoji 😀 \"q 😀\"", "a'b'c", "--flag=\"v w\"",
];
const templates = [
  "$1 and $2", "$@", "$ARGUMENTS", "${1:-def}", "${3:-def}", "${@:-none}", "${ARGUMENTS:-none}", "${@:2}", "${@:2:1}",
  "${@:0}", "${@:1:0}", "${@:9}", "$0 $10 $1$2", "price $5 and $", "${unknown} ${@:x} ${1:-a}b}", "$$1", "\\$1",
  "${@:2:}", "${2:-}", "$ARGUMENTSX $@@", "${1:-${2}}", "a ${@:1:2} b",
];
const argSets = [[], ["one"], ["one", "two", "three"], ["", "b"], ["$2", "${1:-x}", "$@"]];

const parseCommandArgs = argStrings.map((input) => ({ input, output: pt.parseCommandArgs(input) }));
const substituteArgs = [];
for (const template of templates) for (const args of argSets) substituteArgs.push({ template, args, output: pt.substituteArgs(template, args) });

const tpl = (name, content) => ({ name, description: "", content, sourceInfo: {}, filePath: `/p/${name}.md` });
const promptTemplates = [tpl("review", "Review $1 with ${@:2}"), tpl("all", "All: $ARGUMENTS")];
const expandInputs = ["/review file.ts a b", "/review", "/all   spaced   args  ", "/missing x", "review x", "/", "/ review", "/all\tx\ny", "/review \"a b\" 'c'"];
const expandPromptTemplate = expandInputs.map((input) => ({ input, output: pt.expandPromptTemplate(input, promptTemplates) }));

const skill = (name, description, extra = {}) => ({ name, description, filePath: `/s/${name}/SKILL.md`, baseDir: `/s/${name}`, sourceInfo: {}, disableModelInvocation: false, ...extra });
const skillSets = [
  [],
  [skill("a", "desc a")],
  [skill("x&y", "<b> 'q' \"d\""), skill("h", "hidden", { disableModelInvocation: true })],
  [skill("h", "hidden", { disableModelInvocation: true })],
];
const formatSkills = [];
for (const set of skillSets) for (const reader of ["read", "bash", "indirect"]) formatSkills.push({ skills: set, reader, output: skills.formatSkillsForPrompt(set, reader) });

const blocks = [
  '<skill name="s" location="/a/SKILL.md">\nReferences are relative to /a.\n\nBody\n</skill>',
  '<skill name="s" location="/a/SKILL.md">\nReferences are relative to /a.\n\nBody\n</skill>\n\n  user text  ',
  '<skill name="s" location="/a/SKILL.md">\nA\n</skill>\n\nB\n</skill>\n\nC',
  '<skill name="s" location="/a/SKILL.md">\nBody\n</skill>\n\n   ',
  '<skill name="s" location="/a/SKILL.md">\nBody\n</skill>\n',
  '<skill name="" location="/a">\nB\n</skill>',
  "plain",
];
const parseSkillBlock = blocks.map((input) => ({ input, output: session.parseSkillBlock(input) }));

const keys = ["a", "z", "0", "A", "ctrl+a", "ctrl+shift+a", "shift+ctrl+o", "ctrl+ctrl+a", "ctrl+shift+alt+super+x", "ctrl+shift+alt+super+ctrl+x",
  "alt++", "ctrl+-", "pageUp", "pageup", "f12", "f13", "escape", "esc", "", "ctrl+", "ab", "ctrl+a+b", "super+`", "ctrl+\\", "shift+'"];
const keyIdChecks = keys.map((key) => ({ key, valid: validKeyBinding.Check(key) }));
const keyArrayChecks = [[], ["a", "ctrl+b"], ["a", "bad key"], "x"].map((value) => ({ value, valid: validKeyBinding.Check(value) }));

const migrations = [
  {},
  { cursorUp: "up", "app.interrupt": "escape", interrupt: "x", zeta: "a", alpha: "b", $schema: "s" },
  { submit: "enter", tab: ["tab"], "tui.input.submit": "ctrl+enter" },
].map((input) => ({ input, output: kb.migrateKeybindingsConfig(input) }));

const frontmatters = [
  "no frontmatter\n", "---\nname: x\ndescription: \"d: e\"\n---\nBody\n", "\ufeff---\r\nname: y\r\n---\r\n\r\n  Body  \r\n",
  "---\n---\nbody", "---\nname: x", "---\njust text\n---\nb", "---\nlist:\n  - 1\n  - two\nnested: {a: true, b: null}\n---\n",
  "---\nn: 1.5\nbig: 12345678901234\nneg: -3\nyes: yes\n---\n", "--- \nname: z\n---\nb",
];
const parseFrontmatter = frontmatters.map((input) => {
  try {
    return { input, output: fm.parseFrontmatter(input) };
  } catch (error) {
    return { input, error: true };
  }
});

const out = { parseCommandArgs, substituteArgs, expandPromptTemplate, formatSkills, parseSkillBlock, keyIdChecks, keyArrayChecks, migrations, parseFrontmatter };
writeFileSync(join(REPO, "crates/oino-resources/tests/fixtures/pi/misc-expected.json"), `${JSON.stringify(out, null, 2)}\n`);
console.log("wrote misc-expected.json");
