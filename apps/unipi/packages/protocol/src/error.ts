import { z } from "zod";

/** Stable machine-readable error codes shared across host, core and UI. */
export const AppErrorCodeSchema = z.enum([
  "E_PROTO_VERSION",
  "E_SCHEMA",
  "E_UNAUTHORIZED",
  "E_NOT_FOUND",
  "E_UNSUPPORTED",
  "E_TRANSPORT",
  "E_TIMEOUT",
  "E_MUX_UNAVAILABLE",
  "E_BOARD_UNAVAILABLE",
  "E_BOARD_REJECTED",
  "E_CHAT_UNAVAILABLE",
  "E_BLOB_REJECTED",
  "E_BLOB_UNAVAILABLE",
  "E_PAIRING_INVALID",
  "E_HOSTKEY_MISMATCH",
  "E_SETTINGS",
  "E_INTERNAL",
  "E_CONFLICT",
  "E_BINARY",
  "E_SCOPE",
  "E_FORWARD_DISABLED",
  "E_BROWSER_DISABLED",
  "E_BROWSER_UNAVAILABLE",
  "E_DESKTOP_DISABLED",
  "E_DESKTOP_UNAVAILABLE",
  "E_GIT",
  "E_LSP_TAKEN",
]);

export type AppErrorCode = z.infer<typeof AppErrorCodeSchema>;

/** `path` segments identify where in a payload the error occurred. */
const PathSchema = z.array(z.union([z.string(), z.number()]));

/**
 * The one error envelope used everywhere (host -> core -> UI). See
 * unipi-app/DESIGN.md §5.5.
 */
export const AppErrorSchema = z.object({
  code: AppErrorCodeSchema,
  message: z.string().min(1, "message must not be empty"),
  hint: z.string().optional(),
  path: PathSchema.optional(),
  retryable: z.boolean(),
  detail: z.unknown().optional(),
  /** The backend (host) version that produced the error; older hosts omit it. */
  host_version: z.string().optional(),
});

export type AppError = z.infer<typeof AppErrorSchema>;

/** What the UI knows about one code: a friendly fallback line and its default retryability. */
export interface AppErrorInfo {
  /** Shown when an error carries no usable message of its own. */
  readonly title: string;
  /** Mirrors `AppErrorCode::default_retryable` in unipi-proto (parity-tested). */
  readonly retryable: boolean;
}

/**
 * Every code, handled once (app-convention: exhaustive error handling). A
 * total `Record` over the enum: adding a code to `AppErrorCodeSchema` fails
 * to type-check until it gets an entry here, and `tests/error.test.ts` fails
 * until the Rust enum (`unipi-proto/src/error.rs`) and this list agree.
 */
export const APP_ERROR_INFO: Readonly<Record<AppErrorCode, AppErrorInfo>> = {
  E_PROTO_VERSION: { title: "This computer runs an incompatible version.", retryable: false },
  E_SCHEMA: { title: "The request was not valid.", retryable: false },
  E_UNAUTHORIZED: { title: "This device is not allowed to do that.", retryable: false },
  E_NOT_FOUND: { title: "It no longer exists.", retryable: false },
  E_UNSUPPORTED: { title: "This computer does not support that yet.", retryable: false },
  E_TRANSPORT: { title: "The connection to this computer dropped.", retryable: true },
  E_TIMEOUT: { title: "This computer did not answer in time.", retryable: true },
  E_MUX_UNAVAILABLE: { title: "The terminal multiplexer is not available.", retryable: true },
  E_BOARD_UNAVAILABLE: { title: "The board is not available right now.", retryable: true },
  E_BOARD_REJECTED: { title: "The board rejected the change.", retryable: false },
  E_CHAT_UNAVAILABLE: { title: "The chat is not available right now.", retryable: true },
  E_BLOB_REJECTED: { title: "The file was rejected.", retryable: false },
  E_BLOB_UNAVAILABLE: { title: "The file is not available right now.", retryable: true },
  E_PAIRING_INVALID: { title: "The pairing code is not valid.", retryable: false },
  E_HOSTKEY_MISMATCH: { title: "This computer's identity changed. Pair it again.", retryable: false },
  E_SETTINGS: { title: "The setting could not be saved.", retryable: false },
  E_INTERNAL: { title: "Something went wrong.", retryable: false },
  E_CONFLICT: { title: "The file changed on disk.", retryable: false },
  E_BINARY: { title: "This file is not text.", retryable: false },
  E_SCOPE: { title: "That path is outside the allowed folders.", retryable: false },
  E_FORWARD_DISABLED: { title: "Port forwarding is off on this computer.", retryable: false },
  E_BROWSER_DISABLED: { title: "The remote browser is off on this computer.", retryable: false },
  E_BROWSER_UNAVAILABLE: { title: "No browser could be started.", retryable: false },
  E_DESKTOP_DISABLED: { title: "Desktop streaming is off on this computer.", retryable: false },
  E_DESKTOP_UNAVAILABLE: { title: "The desktop cannot be streamed.", retryable: false },
  E_GIT: { title: "The git command failed.", retryable: false },
  E_LSP_TAKEN: { title: "Language features moved to another device.", retryable: false },
};

/**
 * Builds an AppError from a known code (no ad-hoc code strings or `as AppError`
 * casts at call sites). `retryable` defaults to the code's default.
 */
export function appError(
  code: AppErrorCode,
  message: string,
  extra: Partial<Omit<AppError, "code" | "message">> = {},
): AppError {
  return {
    code,
    message: message || APP_ERROR_INFO[code].title,
    ...extra,
    retryable: extra.retryable ?? APP_ERROR_INFO[code].retryable,
  };
}

/** How the UI shows an error: the line to read, plus a small reference with the code and backend version. */
export interface AppErrorDescription {
  readonly text: string;
  /** e.g. `E_GIT · host 1.0.0` — always shown, so a report names the code and version. */
  readonly reference: string;
}

/**
 * The one mapping from an error to what the user reads. A known code with a
 * message shows the message; without one, its friendly title. Anything that
 * isn't a valid AppError falls back to a generic line, still with a code.
 */
export function describeAppError(err: unknown): AppErrorDescription {
  const parsed = AppErrorSchema.safeParse(err);
  if (!parsed.success) {
    const raw = (err as { code?: unknown } | null)?.code;
    const code = typeof raw === "string" && raw ? raw : "E_INTERNAL";
    return { text: APP_ERROR_INFO.E_INTERNAL.title, reference: code };
  }
  const e = parsed.data;
  const text = e.message.trim() || APP_ERROR_INFO[e.code].title;
  return { text, reference: e.host_version ? `${e.code} · host ${e.host_version}` : e.code };
}

/**
 * Whether an AppError should be treated as retryable. Prefers the
 * explicit `retryable` flag on the error itself; falls back to a
 * code-based default when the flag disagrees is not applicable
 * (always trusts the explicit flag first since the server/producer
 * knows best).
 */
export function isRetryable(error: AppError): boolean {
  return error.retryable;
}

/** Default retryability for a given code, ignoring any explicit flag. */
export function defaultRetryableForCode(code: AppErrorCode): boolean {
  return APP_ERROR_INFO[code].retryable;
}

/**
 * Normalizes anything caught at a boundary into an AppError.
 *
 * - If `err` is already a valid AppError shape, it is returned as-is
 *   (parsed, so extra fields are stripped and types are enforced).
 * - If `err` is a ZodError, the first issue's path/message is used,
 *   with `z.prettifyError` output folded into `detail`.
 * - Otherwise (Error, string, or anything else) it becomes E_INTERNAL.
 */
export function toAppError(err: unknown): AppError {
  if (err instanceof z.ZodError) {
    return zodErrorToAppError(err);
  }

  const parsedExisting = AppErrorSchema.safeParse(err);
  if (parsedExisting.success) {
    return parsedExisting.data;
  }

  if (err instanceof Error) {
    return {
      code: "E_INTERNAL",
      message: err.message || "Internal error",
      retryable: false,
      detail: err.stack,
    };
  }

  if (typeof err === "string") {
    return {
      code: "E_INTERNAL",
      message: err,
      retryable: false,
    };
  }

  return {
    code: "E_INTERNAL",
    message: "Unknown error",
    retryable: false,
    detail: err,
  };
}

function zodErrorToAppError(zodError: z.ZodError): AppError {
  const [firstIssue] = zodError.issues;
  const message = firstIssue?.message ?? "Schema validation failed";
  const path = firstIssue?.path as (string | number)[] | undefined;

  return {
    code: "E_SCHEMA",
    message,
    path: path && path.length > 0 ? path : undefined,
    retryable: false,
    detail: z.prettifyError(zodError),
  };
}
