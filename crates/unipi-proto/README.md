# unipi-proto

Wire protocol types and framing for `unipi/oino/1` (serde mirror of
`apps/unipi/packages/protocol`). See `docs/dev/v2/specs/host.md` §5.1, §7.

## API

- `PROTO_VERSION` — `"2.0.0"` (v2: the chat/session layer rewrite is breaking).
- `is_strict_semver(&str) -> bool`, `Semver` — strict semver newtype,
  validated on deserialize.
- `AppErrorCode` — the 14 stable error codes (`E_*`, including
  `E_UNSUPPORTED`), with `default_retryable(self) -> bool`.
- `AppErrorDto` — the one error envelope
  (`code, message, hint?, path?, retryable, detail?`), shared with the
  TypeScript `AppErrorSchema`.
- `PathSegment` — `Key(String) | Index(i64)`.
- `ProtoError` — per-layer `thiserror` enum (transport, mux, board,
  settings, pairing, schema, ...) with `to_app_error(&self) -> AppErrorDto`
  and `from_path_error` to build an `E_SCHEMA` error (with path) from a
  `serde_path_to_error::Error`.
- `Control`, `Request`, `MuxKind`, `SizePolicy`, `SessionId`,
  `Dimension`, `PairCode`, `PairName`, `Title` — the control message
  and request types, internally tagged to match the TypeScript
  discriminated unions, with range/length validation on deserialize.
  `Request::Pair` has an optional `name: Option<PairName>` field
  (1..=64 chars).
- `SessionName`, `SessionSummary`, `ListData` — the typed `ok.data`
  payload for a `list` request: `{ sessions: Vec<SessionSummary> }`
  where `SessionSummary = { name: SessionName, running: bool, default: bool }`.
- `HostId`, `HostName`, `PairData` — the typed `ok.data` payload for a
  `pair` request: `{ host_id: HostId, host_name: HostName }`.
  `HostId` validates a 64-char lowercase hex string or a 52-char
  z-base-32 string (iroh `NodeId` encodings); `HostName` is 1..=64
  chars.
- `AttachData` — the typed `ok.data` payload for an `attach` request:
  `{ effective_size_policy: SizePolicy }`.
- `TermId`, `TermShell`, `TermInfo`, `TermAttachData` — the host-owned
  terminal layer (`term_open`/`term_attach`/`term_list`/`term_kill`,
  host.md §9.3): a validated terminal id and shell, the `term_list` row
  (`term_id, cwd, shell, alive, last_activity`) and the
  `ok{data}` reply both attach ops send before the stream starts.
- `frame` module: `FrameKind`, `FRAME_HEADER_LEN`, `MAX_FRAME`,
  `encode(kind, &[u8]) -> Result<Vec<u8>, FrameError>`, `Decoder`
  (`push`/`pop`) for incremental decoding.

No `unwrap`/`expect` outside `#[cfg(test)]` code.

## Tests

```sh
cargo test -p unipi-proto
cargo clippy -p unipi-proto --all-targets -- -D warnings
cargo fmt -p unipi-proto -- --check
```

`tests/fixtures.rs` is the contract test: it loads
`../../protocol/fixtures/control.{valid,invalid}.json` and asserts
`Control` deserializes/rejects them the same way `ControlSchema` does
in `packages/protocol`.

`tests/data_fixtures.rs` does the same for
`../../protocol/fixtures/data.{valid,invalid}.json`, dispatching on
each fixture's `schema` field (`ListData`, `PairData`, `AttachData`)
to match `ListDataSchema`/`PairDataSchema`/`AttachDataSchema` in
`packages/protocol`.
