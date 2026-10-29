# @unipi/protocol

Zod 4 schemas for the UniPi App wire protocol (`unipi/app/1`). See
`unipi-app/DESIGN.md` §5.4–5.5. The Rust mirror is `crates/proto`
(`unipi-proto`); shared contract fixtures live in `protocol/fixtures/`.

## Exports (`src/index.ts`)

- `PROTO_VERSION` — `"1.0.0"`.
- `SemverSchema` — strict semver string schema.
- `AppErrorCodeSchema`, `AppErrorCode` — the 14 stable error codes
  (including `E_UNSUPPORTED`).
- `AppErrorSchema`, `AppError` — the one error envelope.
- `toAppError(err: unknown): AppError` — normalizes anything caught at
  a boundary (`ZodError`, `Error`, `string`, already-valid `AppError`,
  or anything else) into an `AppError`.
- `isRetryable(error: AppError): boolean` — reads the explicit
  `retryable` flag.
- `defaultRetryableForCode(code: AppErrorCode): boolean` — code-based
  default, ignoring any explicit flag.
- `MuxKindSchema`, `MuxKind` — `"herdr" | "tmux"`.
- `SizePolicySchema`, `SizePolicy` — `"fit_phone" | "keep_pc"`.
- `RequestSchema`, `Request` — the `hello` request discriminated union
  (`list`, `attach`, `pair`). `pair` has an optional `name` field
  (1..=64 chars).
- `ControlSchema`, `Control` — the control message discriminated union
  (`hello`, `ok`, `err`, `resize`, `exit`, `title`).
- `SessionSummarySchema`, `SessionSummary` — one session entry
  (`name` 1..=128 chars, `running: boolean`, `default: boolean`).
- `ListDataSchema`, `ListData` — typed `ok.data` payload for a `list`
  request: `{ sessions: SessionSummary[] }`.
- `PairDataSchema`, `PairData` — typed `ok.data` payload for a `pair`
  request: `{ host_id, host_name }`. `host_id` is a 64-char lowercase
  hex string or a 52-char z-base-32 string (iroh `NodeId` encodings);
  `host_name` is 1..=64 chars.
- `AttachDataSchema`, `AttachData` — typed `ok.data` payload for an
  `attach` request: `{ effective_size_policy: SizePolicy }`.
- `FRAME_HEADER_LEN`, `MAX_FRAME`, `FrameKind`, `FrameError` — framing
  constants and error type.
- `encode(kind, payload): Uint8Array` — encodes one frame.
- `Decoder` — incremental frame decoder (`push`/`pop`).

## Tests

```sh
npm test
```

Runs `vitest`, including contract tests that load every fixture from
`../../protocol/fixtures/control.{valid,invalid}.json` (asserting
`ControlSchema` accepts/rejects them as expected) and from
`../../protocol/fixtures/data.{valid,invalid}.json` (asserting
`ListDataSchema`/`PairDataSchema`/`AttachDataSchema` accept/reject
them as expected, by `schema` field).
