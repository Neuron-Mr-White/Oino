import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";
import {
  ListDataSchema,
  PairDataSchema,
  AttachDataSchema,
} from "../src/control.js";

const fixturesDir = path.resolve(
  fileURLToPath(new URL("../../../protocol/fixtures", import.meta.url)),
);

interface ValidFixture {
  name: string;
  schema: string;
  value: unknown;
}

interface InvalidFixture {
  name: string;
  schema: string;
  value: unknown;
  reason: string;
}

const validFixtures: ValidFixture[] = JSON.parse(
  readFileSync(path.join(fixturesDir, "data.valid.json"), "utf8"),
);
const invalidFixtures: InvalidFixture[] = JSON.parse(
  readFileSync(path.join(fixturesDir, "data.invalid.json"), "utf8"),
);

const schemasByName = {
  ListData: ListDataSchema,
  PairData: PairDataSchema,
  AttachData: AttachDataSchema,
} as const;

function schemaFor(name: string) {
  const schema = (schemasByName as Record<string, (typeof schemasByName)[keyof typeof schemasByName]>)[name];
  if (!schema) {
    throw new Error(`unknown schema in fixture: ${name}`);
  }
  return schema;
}

describe("shared data fixtures", () => {
  it("has at least 2 valid fixtures per schema", () => {
    const counts = new Map<string, number>();
    for (const fixture of validFixtures) {
      counts.set(fixture.schema, (counts.get(fixture.schema) ?? 0) + 1);
    }
    for (const name of Object.keys(schemasByName)) {
      expect(counts.get(name) ?? 0).toBeGreaterThanOrEqual(2);
    }
  });

  it("has at least 2 invalid fixtures per schema", () => {
    const counts = new Map<string, number>();
    for (const fixture of invalidFixtures) {
      counts.set(fixture.schema, (counts.get(fixture.schema) ?? 0) + 1);
    }
    for (const name of Object.keys(schemasByName)) {
      expect(counts.get(name) ?? 0).toBeGreaterThanOrEqual(2);
    }
  });

  for (const fixture of validFixtures) {
    it(`accepts valid fixture: ${fixture.schema}/${fixture.name}`, () => {
      const result = schemaFor(fixture.schema).safeParse(fixture.value);
      expect(result.success, JSON.stringify(result.error?.issues)).toBe(true);
    });
  }

  for (const fixture of invalidFixtures) {
    it(`rejects invalid fixture: ${fixture.schema}/${fixture.name} (${fixture.reason})`, () => {
      const result = schemaFor(fixture.schema).safeParse(fixture.value);
      expect(result.success).toBe(false);
    });
  }
});
