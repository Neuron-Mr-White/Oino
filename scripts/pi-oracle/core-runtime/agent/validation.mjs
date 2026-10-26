// Golden generator: pi-ai validateToolArguments over a fixed case list.
// Usage: node validation.mjs > ../../../../crates/oino-agent/tests/fixtures/validation.json
import { pathToFileURL } from "node:url";
const PI = process.env.PI_CODING_AGENT_PKG ?? `${process.env.HOME}/.local/share/mise/installs/node/24.16.0/lib/node_modules/@earendil-works/pi-coding-agent`;
const { validateToolArguments } = await import(pathToFileURL(`${PI}/node_modules/@earendil-works/pi-ai/dist/index.js`).href);
const obj = (properties, required = [], extra = {}) => ({ type: "object", properties, required, ...extra });
const schemas = {
  read: obj({ path: { type: "string" }, offset: { type: "number" }, limit: { type: "integer" } }, ["path"]),
  strict: obj({ path: { type: "string" } }, ["path"], { additionalProperties: false }),
  nested: obj({ edits: { type: "array", items: obj({ oldText: { type: "string" }, newText: { type: "string" } }, ["oldText", "newText"]) } }, ["edits"]),
  enums: obj({ mode: { enum: ["a", "b"] }, kind: { type: "string", enum: ["x", "y"] }, c: { const: 3 } }),
  bools: obj({ flag: { type: "boolean" }, n: { type: "null" } }),
  limits: obj({ s: { type: "string", minLength: 2, maxLength: 4 }, n: { type: "number", minimum: 1, maximum: 10 }, e: { type: "number", exclusiveMinimum: 0, exclusiveMaximum: 5 }, a: { type: "array", minItems: 1, maxItems: 2 }, i: { type: "integer", multipleOf: 3 } }),
  pattern: obj({ p: { type: "string", pattern: "^[a-z]+$" } }),
  unions: obj({ u: { anyOf: [{ type: "string" }, { type: "number" }] }, o: { oneOf: [{ type: "string" }, { type: "boolean" }] }, t: { type: ["string", "null"] } }),
  addl: obj({ a: { type: "string" } }, [], { additionalProperties: { type: "number" } }),
  arr: obj({ xs: { type: "array", items: { type: "number" } }, uniq: { type: "array", uniqueItems: true } }),
  minprops: obj({}, [], { minProperties: 1 }),
  anyofLimits: obj({ u: { anyOf: [{ type: "string", minLength: 5 }, { type: "number", minimum: 10 }] } }),
  oneofDup: obj({ u: { oneOf: [{ type: "string" }, { type: "string" }] } }),
  arrObj: obj({ e: { type: "array", items: { type: "object", properties: { a: { type: "string" } } } } }),
  twoReq: obj({ a: { type: "string" }, b: { type: "number" } }, ["a", "b"]),
  nestedStrict: obj({ x: { type: "object", properties: { y: { type: "string" } }, required: ["y"], additionalProperties: false } }),
  allOf: obj({ x: { allOf: [{ type: "string" }, { minLength: 3 }] } }),
  not: obj({ x: { not: { type: "string" } } }),
  tuple: obj({ x: { type: "array", items: [{ type: "string" }] } }),
  contains: obj({ x: { type: "array", contains: { type: "string" } } }),
  rootObj: { type: "object", properties: { a: { type: "string" } }, required: ["a"] },
  objStr: obj({ x: { type: "object" }, y: { type: "array" } }),
  ref: { type: "object", properties: { p: { $ref: "#/$defs/P" } }, required: ["p"], $defs: { P: { type: "object", properties: { n: { type: "integer" } }, required: ["n"] } } },
  optnull: obj({ path: { type: "string" }, opt: { type: "number" }, optstr: { type: "string" } }, ["path"]),
};
const cases = [
  ["read", {}], ["read", { path: 1 }], ["read", { path: "x", offset: "abc" }], ["read", { path: "x", offset: "5" }],
  ["read", { path: "x", limit: 2.5 }], ["read", { path: "x", limit: "7" }], ["read", { path: true }], ["read", { path: null }],
  ["read", { path: "x", offset: true }], ["read", { path: "x", offset: null }], ["read", { path: ["a"] }], ["read", { path: { a: 1 } }],
  ["strict", { path: "x", extra: 1 }], ["strict", { path: "x" }], ["strict", { extra: 1 }],
  ["nested", { edits: [{}] }], ["nested", { edits: [{ oldText: "a", newText: "b" }, { oldText: 1 }] }], ["nested", { edits: "nope" }], ["nested", {}],
  ["enums", { mode: "c" }], ["enums", { kind: "z" }], ["enums", { c: 4 }], ["enums", { mode: "a", kind: "x", c: 3 }],
  ["bools", { flag: "true" }], ["bools", { flag: "false" }], ["bools", { flag: 1 }], ["bools", { flag: "yes" }], ["bools", { n: "" }], ["bools", { n: 1 }],
  ["limits", { s: "a" }], ["limits", { s: "abcde" }], ["limits", { n: 0 }], ["limits", { n: 11 }], ["limits", { e: 0 }], ["limits", { e: 5 }], ["limits", { a: [] }], ["limits", { a: [1, 2, 3] }], ["limits", { i: 4 }],
  ["pattern", { p: "ABC" }], ["pattern", { p: "abc" }],
  ["unions", { u: true }], ["unions", { u: 1 }], ["unions", { o: 3 }], ["unions", { t: 5 }], ["unions", { t: null }],
  ["addl", { a: "x", b: "y" }], ["addl", { a: "x", b: "2" }],
  ["arr", { xs: ["1", "x"] }], ["arr", { uniq: [1, 1] }],
  ["minprops", {}],
  ["optnull", { path: "x", opt: null, optstr: null }],
  ["anyofLimits", { u: {} }], ["anyofLimits", { u: "ab" }], ["oneofDup", { u: "ab" }], ["arrObj", { e: [1] }],
  ["twoReq", { b: "x" }], ["twoReq", { b: "x", a: {} }], ["read", { path: "x", limit: "7.5" }], ["read", { path: "x", offset: "" }],
  ["read", { path: "x", offset: " 3 " }], ["read", { path: 1.5 }], ["read", { path: 1e21 }], ["nestedStrict", { x: { z: 1 } }],
  ["allOf", { x: "ab" }], ["not", { x: "ab" }], ["tuple", { x: [1] }], ["contains", { x: [1] }], ["bools", { flag: 0 }],
  ["bools", { flag: null }], ["bools", { flag: 2 }], ["strict", { path: "a", x: 1, y: 2 }], ["read", { path: "x", offset: "0x10" }],
  ["read", { path: "x", offset: "Infinity" }], ["read", { path: "x", limit: true }], ["objStr", { x: "{}" }], ["objStr", { y: "[]" }],
  ["ref", { p: { n: "4" } }], ["ref", { p: {} }], ["ref", {}],
];
const out = cases.map(([schemaName, args]) => {
  try {
    return { schema: schemaName, args, ok: validateToolArguments({ name: "t", parameters: schemas[schemaName] }, { name: "t", arguments: args }) };
  } catch (error) {
    return { schema: schemaName, args, error: error.message };
  }
});
out.push((() => { try { validateToolArguments({ name: "t", parameters: schemas.read }, { name: "t", arguments: {}, argumentsError: "Unexpected end of JSON input" }); return {}; } catch (e) { return { schema: "read", args: {}, argumentsError: "Unexpected end of JSON input", error: e.message }; } })());
console.log(JSON.stringify({ schemas, cases: out }, null, 2));
