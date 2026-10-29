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
});

export type AppError = z.infer<typeof AppErrorSchema>;

/** Error codes that are safe to retry automatically (with backoff). */
const RETRYABLE_CODES: ReadonlySet<AppErrorCode> = new Set([
  "E_TRANSPORT",
  "E_TIMEOUT",
  "E_MUX_UNAVAILABLE",
  "E_BOARD_UNAVAILABLE",
  "E_CHAT_UNAVAILABLE",
  "E_BLOB_UNAVAILABLE",
]);

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
  return RETRYABLE_CODES.has(code);
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
