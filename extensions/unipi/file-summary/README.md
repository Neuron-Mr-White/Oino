# `unipi.file-summary`

Turn file snapshots, a per-turn file summary and undo (app.md §3.13, zcode-audit D-1).

## Behaviour

- **Snapshot before `write`/`edit`.** On `tool_call` of the built-in `write` and `edit` tools
  the extension reads the target (through the `project` preopen plus any manifest `fs.paths`)
  the first time a path is touched in the current turn:
  - absent ⇒ `existed: false` (a new file);
  - ≤ 5 MiB ⇒ zstd, sha256, stored as `<ext-session>/<sid>/blobs/<sha256>.zst`
    (content-addressed, identical content is stored once);
  - > 5 MiB ⇒ `skipped: "too_large"` (listed, no diff/undo);
  - `bash`, extension tools and every other tool are not captured.
- The in-progress turn index is written to kv (`turn:<turnEntryId>`) at every snapshot, so a
  host killed mid-turn rebuilds the turn from kv + the session entries.
- **`agent_end`** fills each record's `afterSha` and the `similar` line counts `+add −del`,
  appends the session `custom` entry `unipi.file-summary/turn`
  `{turnEntryId, files:[{path, existed, beforeSha?, afterSha?, add, del, skipped?}]}` and
  deletes the kv index.
- The turn id is the id of the user message entry that started the run (`turnEntryId`, the
  app's `turnId`).
- **Services** (`_oino/service_call{target:"unipi.file-summary", name, payload}`):
  - `turn{turnEntryId}` → `{turnEntryId, files, undone, undonePaths, running}`;
  - `diff{turnEntryId, path}` → `{turnEntryId, path, diff, truncated}` with a unified diff of
    the snapshot vs. the file as it is now (≤ 1 MiB, `reason` when there is no snapshot);
  - `undo{turnEntryId, paths?, force?}` → `{restored, deleted, conflicts, failed, found}`;
    a file changed since the turn ended is a conflict unless `force` is set;
  - `undoFrom{turnEntryId, force?}` → undoes that turn and every later turn, newest first;
  - every undo appends a `unipi.file-summary/undo` custom entry and invalidates the turn
    footer.
- **Footer.** `register-renderer{target: turn-footer, key: "files"}` returns the node tree
  "3 files +12 −04" plus one row per path (path, `+X −Y`, `new`/`modified`/`deleted`/`skipped`/
  `undone`). The app renders the reserved id `unipi.file-summary/turn-files` natively
  (WP-A27); TUI and other frontends use this tree. A running turn's counts are computed on
  request (frontends re-request at most every 500 ms).

State is rebuildable from session `custom` entries + kv (DIRECTION §3.1.6). Blobs live in the
session's `ext-session` directory and are deleted with the session.

## Defaults taken where the spec leaves a choice

- Display path: relative to the session cwd when the file is inside it, absolute otherwise.
- Counts in the footer are zero-padded to two digits (`+12 −04`); per-row counts are not.
- A `skipped` file is listed but never undone.
- `undoFrom` with a turn whose user message is no longer in the session file undoes that turn's
  recorded files (there are no later ones to find).
- A live turn without a `turnEntryId` renders the kv index of the current user message, else
  the newest in-progress index of the session.
- `diff` on a deleted file diffs against the empty text (all removals).

## Tests

- Native unit tests: `cd extensions && cargo test -p unipi-file-summary --lib`.
- wasm-host harness: `extensions/unipi/file-summary/harness/run.sh` (snapshots, skipped files,
  no `bash` capture, resumed turn after a killed host, `+X −Y`, dedup, undo/undoFrom/conflict,
  footer golden at 40/100 columns).
