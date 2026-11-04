# Chat stability rig ([chat-load])

This rig reproduces the owner's "entering a chat is unstable" report on the real frontend.
It needs no app build: a vite dev server runs with the dev mock host and serves **copies** of
real chats, and the mock pages them the way the host does. Headless Chromium drives the
chat over CDP.

```sh
# copies only — never point it at ~/.oino/sessions (the script refuses ~/.oino)
mkdir -p ~/oino-qa-release/chat-load/chats && cp <some chat>.jsonl ~/oino-qa-release/chat-load/chats/
scripts/qa/chat-stability/run.sh apps/unipi ~/oino-qa-release/chat-load/chats after /tmp/out whitelabel.jsonl
# the old code: a worktree with the main tree's node_modules
git worktree add --detach /tmp/old ce3ff913 && ln -s $PWD/apps/unipi/node_modules /tmp/old/apps/unipi/node_modules
scripts/qa/chat-stability/run.sh /tmp/old/apps/unipi ~/oino-qa-release/chat-load/chats before /tmp/out whitelabel.jsonl
```

For each chat, the rig runs these steps (`drive.mjs`):

1. Open the chat from the sidebar.
2. Wheel up through **all** history, pausing at the top so chunks load.
3. Drag the scrollbar to the middle. Headless Chromium draws no scrollbar, so the rig
   presses the scroller and then fires scroll events with no wheel.
4. Open and close six folds or subagent cards.
5. Flick up and down in fast wheel bursts.
6. Return to the bottom with the jump button.

On every animation frame the page records:

| Metric | Meaning |
|---|---|
| `overlaps` / `gaps` | Row rects that intersect, or have a gap between them that is on screen |
| `jumps` / `maxJump` / `worst` | Moves of more than 2 px of the first visible turn, in frames with no reader input in the last 120 ms |
| `blank` | Frames where the chat is open but nothing of it is on screen (no row, no tail, no skeleton) |
| `hash` | `location.hash` history: opening a chat must never navigate anywhere else |
| `long`, `longMs`, `maxFrame` | Long tasks and the longest frame |
| `rowsAdded` | Row mounts |
| `pages` | Host pages served |
| `pagesDuringFlick` | Host pages served during a flick (must be 0) |
| `foldMove` | Max movement of a toggled fold header |

Output per chat:

- `<out>/<label>-<chat>.json`: per-step state and the sampler output.
- A screenshot sequence: `<label>-<chat>-NN-<step>.png`.
- A screencast: `<label>-<chat>.mp4`, written when `ffmpeg` is on PATH. Set `RL_CAST=0`
  to skip it.

Resources: one vite and one headless shell, with no build. `run.sh` kills both on exit and
deletes its temporary vite config.

The same assertions run in CI-style jsdom with a browser-like flow layout:
`apps/unipi/src/components/timeline/Timeline.stability.test.tsx`. The design is in
oino-internal `docs/dev/v2/design/chat-render-pipeline.md`.
