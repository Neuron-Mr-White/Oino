import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";
import { ControlSchema } from "../src/control.js";

const fixturesDir = path.resolve(
  fileURLToPath(new URL("../../../protocol/fixtures", import.meta.url)),
);

interface ValidFixture {
  name: string;
  value: unknown;
}

interface InvalidFixture {
  name: string;
  value: unknown;
  reason: string;
}

const validFixtures: ValidFixture[] = JSON.parse(
  readFileSync(path.join(fixturesDir, "control.valid.json"), "utf8"),
);
const invalidFixtures: InvalidFixture[] = JSON.parse(
  readFileSync(path.join(fixturesDir, "control.invalid.json"), "utf8"),
);

describe("shared fixtures", () => {
  it("has at least 12 valid fixtures", () => {
    expect(validFixtures.length).toBeGreaterThanOrEqual(12);
  });

  it("has at least 15 invalid fixtures", () => {
    expect(invalidFixtures.length).toBeGreaterThanOrEqual(15);
  });

  for (const fixture of validFixtures) {
    it(`accepts valid fixture: ${fixture.name}`, () => {
      const result = ControlSchema.safeParse(fixture.value);
      expect(result.success, JSON.stringify(result.error?.issues)).toBe(true);
    });
  }

  for (const fixture of invalidFixtures) {
    it(`rejects invalid fixture: ${fixture.name} (${fixture.reason})`, () => {
      const result = ControlSchema.safeParse(fixture.value);
      expect(result.success).toBe(false);
    });
  }
});
