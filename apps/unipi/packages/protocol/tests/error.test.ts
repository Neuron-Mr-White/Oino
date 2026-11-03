import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { z } from "zod";
import {
  APP_ERROR_INFO,
  AppErrorCodeSchema,
  AppErrorSchema,
  appError,
  describeAppError,
  defaultRetryableForCode,
  isRetryable,
  toAppError,
  type AppError,
} from "../src/error.js";

describe("toAppError", () => {
  it("returns an already-valid AppError as-is (normalized through the schema)", () => {
    const original: AppError = {
      code: "E_TIMEOUT",
      message: "attach timed out",
      retryable: true,
    };
    const result = toAppError(original);
    expect(result).toEqual(original);
  });

  it("strips unknown fields from an AppError-shaped object", () => {
    const result = toAppError({
      code: "E_TIMEOUT",
      message: "attach timed out",
      retryable: true,
      somethingElse: "ignored",
    });
    expect(result).toEqual({
      code: "E_TIMEOUT",
      message: "attach timed out",
      retryable: true,
    });
  });

  it("maps a ZodError to E_SCHEMA using the first issue's path and prettified detail", () => {
    const schema = z.object({ cols: z.number().int().min(1) });
    const parseResult = schema.safeParse({ cols: 0 });
    expect(parseResult.success).toBe(false);

    const appError = toAppError(parseResult.error);
    expect(appError.code).toBe("E_SCHEMA");
    expect(appError.path).toEqual(["cols"]);
    expect(appError.retryable).toBe(false);
    expect(typeof appError.detail).toBe("string");
    expect(appError.message.length).toBeGreaterThan(0);
  });

  it("maps an Error to E_INTERNAL", () => {
    const appError = toAppError(new Error("boom"));
    expect(appError).toMatchObject({
      code: "E_INTERNAL",
      message: "boom",
      retryable: false,
    });
  });

  it("maps a string to E_INTERNAL", () => {
    const appError = toAppError("something broke");
    expect(appError).toEqual({
      code: "E_INTERNAL",
      message: "something broke",
      retryable: false,
    });
  });

  it("maps an unknown value (object, number, etc.) to E_INTERNAL", () => {
    const appError = toAppError({ weird: true });
    expect(appError.code).toBe("E_INTERNAL");
    expect(appError.retryable).toBe(false);
  });

  it("maps null and undefined to E_INTERNAL", () => {
    expect(toAppError(null).code).toBe("E_INTERNAL");
    expect(toAppError(undefined).code).toBe("E_INTERNAL");
  });

  it("round-trips through AppErrorSchema", () => {
    const appError = toAppError(new Error("x"));
    expect(AppErrorSchema.safeParse(appError).success).toBe(true);
  });
});

describe("isRetryable", () => {
  it("is true when retryable flag is true", () => {
    expect(isRetryable({ code: "E_TRANSPORT", message: "m", retryable: true })).toBe(
      true,
    );
  });

  it("is false when retryable flag is false", () => {
    expect(
      isRetryable({ code: "E_TRANSPORT", message: "m", retryable: false }),
    ).toBe(false);
  });

  it("respects the explicit flag even for a normally non-retryable code", () => {
    expect(
      isRetryable({ code: "E_SCHEMA", message: "m", retryable: true }),
    ).toBe(true);
  });

  it("respects the explicit flag even for a normally retryable code", () => {
    expect(
      isRetryable({ code: "E_TIMEOUT", message: "m", retryable: false }),
    ).toBe(false);
  });
});

describe("defaultRetryableForCode", () => {
  it("marks transport/timeout/mux/board-unavailable as retryable by default", () => {
    expect(defaultRetryableForCode("E_TRANSPORT")).toBe(true);
    expect(defaultRetryableForCode("E_TIMEOUT")).toBe(true);
    expect(defaultRetryableForCode("E_MUX_UNAVAILABLE")).toBe(true);
    expect(defaultRetryableForCode("E_BOARD_UNAVAILABLE")).toBe(true);
  });

  it("marks schema/unauthorized/not-found/internal as non-retryable by default", () => {
    expect(defaultRetryableForCode("E_SCHEMA")).toBe(false);
    expect(defaultRetryableForCode("E_UNAUTHORIZED")).toBe(false);
    expect(defaultRetryableForCode("E_NOT_FOUND")).toBe(false);
    expect(defaultRetryableForCode("E_INTERNAL")).toBe(false);
    expect(defaultRetryableForCode("E_PAIRING_INVALID")).toBe(false);
    expect(defaultRetryableForCode("E_HOSTKEY_MISMATCH")).toBe(false);
    expect(defaultRetryableForCode("E_SETTINGS")).toBe(false);
    expect(defaultRetryableForCode("E_BOARD_REJECTED")).toBe(false);
  });
});

describe("AppErrorSchema host_version", () => {
  it("keeps the backend version a host stamps on the envelope", () => {
    const parsed = AppErrorSchema.parse({ code: "E_GIT", message: "push rejected", retryable: false, host_version: "1.0.0" });
    expect(parsed.host_version).toBe("1.0.0");
  });
  it("still accepts an envelope from an older host without it", () => {
    expect(AppErrorSchema.parse({ code: "E_GIT", message: "x", retryable: false }).host_version).toBeUndefined();
  });
});

describe("exhaustive error codes", () => {
  // The Rust enum is the source of truth (unipi-proto/src/error.rs); this
  // mirror, its info map and the retryable defaults must match it exactly.
  const rust = readFileSync(fileURLToPath(new URL("../../../crates/unipi-proto/src/error.rs", import.meta.url)), "utf8");
  const enumBody = /pub enum AppErrorCode \{([\s\S]*?)\n\}/.exec(rust)?.[1] ?? "";
  const rustCodes = [...enumBody.matchAll(/^\s*(E_[A-Z_]+),/gm)].map((m) => m[1]);
  const retryArm = /match self \{([\s\S]*?)=> true,/.exec(rust)?.[1] ?? "";
  const rustRetryable = new Set([...retryArm.matchAll(/E_[A-Z_]+/g)].map((m) => m[0]));

  it("mirrors the Rust AppErrorCode enum one-to-one", () => {
    expect(rustCodes.length).toBeGreaterThan(20);
    expect([...AppErrorCodeSchema.options]).toEqual(rustCodes);
    expect(Object.keys(APP_ERROR_INFO).sort()).toEqual([...rustCodes].sort());
  });

  it("agrees with AppErrorCode::default_retryable", () => {
    expect(rustRetryable.size).toBeGreaterThan(0);
    for (const code of AppErrorCodeSchema.options) {
      expect([code, defaultRetryableForCode(code)]).toEqual([code, rustRetryable.has(code)]);
    }
  });

  it("appError defaults retryable from the code and never leaves the message empty", () => {
    expect(appError("E_TIMEOUT", "slow")).toEqual({ code: "E_TIMEOUT", message: "slow", retryable: true });
    expect(appError("E_SCHEMA", "", { hint: "h" })).toEqual({ code: "E_SCHEMA", message: APP_ERROR_INFO.E_SCHEMA.title, hint: "h", retryable: false });
    expect(appError("E_TIMEOUT", "x", { retryable: false }).retryable).toBe(false);
    expect(AppErrorSchema.safeParse(appError("E_GIT", "")).success).toBe(true);
  });

  it("describeAppError shows the message with the code and backend version", () => {
    expect(describeAppError({ code: "E_GIT", message: "push rejected", retryable: false, host_version: "1.0.0" })).toEqual({
      text: "push rejected",
      reference: "E_GIT · host 1.0.0",
    });
    expect(describeAppError({ code: "E_GIT", message: " ", retryable: false })).toEqual({ text: APP_ERROR_INFO.E_GIT.title, reference: "E_GIT" });
  });

  it("describeAppError falls back to a generic line plus the code for anything unknown", () => {
    expect(describeAppError({ code: "backend_unavailable", message: "x" })).toEqual({ text: APP_ERROR_INFO.E_INTERNAL.title, reference: "backend_unavailable" });
    expect(describeAppError("boom")).toEqual({ text: APP_ERROR_INFO.E_INTERNAL.title, reference: "E_INTERNAL" });
  });
});
