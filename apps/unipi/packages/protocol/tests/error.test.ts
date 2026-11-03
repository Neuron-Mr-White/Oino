import { describe, expect, it } from "vitest";
import { z } from "zod";
import {
  AppErrorSchema,
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
