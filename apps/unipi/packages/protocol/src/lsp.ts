import { z } from "zod";

/**
 * Language server wire types (app spec §3.12, WP-A21). Mirrors
 * `crates/unipi-proto/src/lsp.rs`.
 *
 * The host owns the servers (one per `(root, language)`); `lsp_open`
 * attaches the caller's stream and then the bytes pass through verbatim
 * — these are only the *envelope* types, the LSP payloads themselves
 * are opaque JSON-RPC.
 */

export const LSP_LANGUAGE_MAX = 64;

export const LspLanguageSchema = z
  .string()
  .min(1, "language must be at least 1 character")
  .max(LSP_LANGUAGE_MAX, `language must be at most ${LSP_LANGUAGE_MAX} characters`)
  .regex(/^[A-Za-z0-9_+.-]+$/, "language must be [A-Za-z0-9_+.-]");
export type LspLanguage = z.infer<typeof LspLanguageSchema>;

export const LspServerInfoSchema = z.object({
  language: z.string(),
  /** Display name (`rust-analyzer`, `typescript-language-server`, …). */
  name: z.string(),
  /** Whether the configured binary exists on the host's PATH. */
  available: z.boolean(),
  /** How to install it — shown when `available` is false. */
  install_hint: z.string().nullable().optional(),
  /** The resolved command line (display in Settings › Language servers). */
  command: z.string(),
});
export type LspServerInfo = z.infer<typeof LspServerInfoSchema>;

/** `lsp_servers` → `ok{data}`: project root + the server's roster. */
export const LspServersDataSchema = z.object({
  root: z.string().nullable().optional(),
  servers: z.array(LspServerInfoSchema),
});
export type LspServersData = z.infer<typeof LspServersDataSchema>;

/**
 * `lsp_open` → `ok{data}`: which server this stream attached to, whether
 * it already ran (`attached`), and — for re-attaches — the `initialize`
 * result the first client negotiated (the app can skip the round-trip).
 */
export const LspAttachDataSchema = z.object({
  server: z.string(),
  attached: z.boolean(),
  initialize_result: z.unknown().nullable().optional(),
});
export type LspAttachData = z.infer<typeof LspAttachDataSchema>;
