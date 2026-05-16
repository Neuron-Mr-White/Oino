# Streaming and async subskill

Use for live logs, network streams, Kafka records, model/tool token streams, background file scans, DB queries, spinners, redraw coalescing, and cancellation.

## Source inspirations
DeepSeek-TUI, Codex frame requester/event broker, oatmeal, bandwhich, dua-cli, oxker `Rerender`/`FrameData`, trippy trace snapshots/freeze, yozefu Kafka/search workers, rainfrog DB polling.

## Related references

- `architecture.md`
- `chat-agent.md`
- `telemetry-dashboards.md`
- `inspectors-explorers.md`

## Worker pattern

- Start workers outside render: network clients, Docker/Kafka consumers, DB queries, search parsers, filesystem scans, markdown parsers.
- Workers send typed actions/events over channels: `RecordsAndStats`, `TopicDetails`, `SearchFinished`, `TraceSnapshot`, `ParseComplete(doc_id, result)`.
- Use cancellation tokens for every restartable worker. New search/topic/query cancels old worker and clears UI state.
- Include progress/error events; never rely only on logs.

## Bounded ingestion

For high-volume streams:
- Use bounded/ring buffers for UI-visible records/logs.
- Drain a maximum number of channel messages per draw/tick (yozefu drains up to 500) so input remains responsive.
- Track metrics separately from visible rows (`read`, `matched`, `buffer_size`, `total_to_read`).
- Sort only when new records arrived; skip work if stats/read count did not change.

## Redraw control

Choose one:

1. Dirty flag + periodic poll for small sync apps.
2. Atomic rerender flag + timed fallback (oxker redraws when requested or after Docker interval).
3. Frame requester actor for streaming/animations (Codex coalesces deadlines and broadcasts draw events).
4. Fixed refresh-rate loop for telemetry (trippy snapshots then polls for one key per refresh interval).

Do not redraw directly from worker callbacks; send an action and request a frame.

## Snapshots for render

- Build a cheap render snapshot before drawing. Oxker’s `FrameData` collects chart data, status, selection, titles, ports, loading icon, and errors to reduce mutex reads inside draw blocks.
- Trippy snapshots trace data unless frozen, clamps selected hop, then renders from stable data.
- For documents, tag parse/highlight results with a version id and discard stale results.

## Loading and live states

- Show explicit `Loading`, `Live`, `Searching`, `Consuming`, `Pending`, `Cancelled`, and `Error` states.
- Use reference-counted spinners when multiple async requests can overlap; stop only when all UUIDs are done.
- Status animation should have its own low-cost tick path and should not force expensive data recomputation.

## Terminal handoff

For external editors/exec/subprocesses:
- Pause event stream or event broker.
- Leave alt screen / restore terminal modes.
- Run child.
- Re-enable modes, flush pending input, re-enter alt screen, clear/redraw, resume event stream.

## Testing checks

- Cancelling an old worker prevents stale results from hydrating current state.
- A burst of frames coalesces to bounded redraws.
- Ring buffer selection remains valid after overflow.
- Freeze/pause stops data mutation but still allows navigation/help.
- Terminal handoff resumes input and redraws correctly.


## Background traversal pattern (dua-cli)

A filesystem traversal/browser should select across terminal input and traversal events:

- Start a `BackgroundTraversal` when entering/rescanning a directory.
- Store previous selection as `(path/name, index)` before scan.
- `select!` over terminal events and traversal events.
- Integrate traversal events into the tree, update stats, and redraw after each meaningful batch.
- When traversal finishes, recompute sizes recursively and clear scan state.
- If the user has not interacted yet, restore previous selected entry by name, fallback to previous index, then first entry.
- Force “scanning”/progress messages by resetting transient message on traversal updates.


## Document parse/image pipeline (mdfried)

For expensive document parsing and terminal images:

- Worker thread owns parser and a small async runtime.
- Parse command includes document id and wrapping width.
- Emit sections incrementally so first content can appear before images finish.
- Emit `ParseDone` before slow uncached image/header tasks complete; placeholders can hydrate later.
- Cache terminal image protocols across reparses/reloads; send cached image events before `ParseDone` to avoid flicker.
- Stale event filtering by document id is mandatory because resize/reload can start a new parse while old image tasks still run.
