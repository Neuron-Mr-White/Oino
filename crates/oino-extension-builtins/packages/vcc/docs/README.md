# Oino VCC

Oino VCC is an optional built-in deterministic session compactor and recall surface inspired by `pi-vcc`, normalized for Oino session trees and Oino command names.

## Install

Open `/extensions`, press `i` for project install or `I` for global install, and enter:

```text
builtin:vcc
```

## Commands

```text
/compact
/recall [query]
```

`/compact` compacts older active-branch entries before the latest user message into structured sections: session goals, files/changes, commits, outstanding context, user preferences, a brief transcript, and a recall reminder. Oino appends the summary as a session compaction entry and refreshes provider context.

`/recall [query]` searches raw session history. In the TUI it also appends the recall result as branch context so the assistant can use it in the next turn. Non-interactive `/recall` prints the result.

## Tool

The package also enables the model-visible tool:

```text
vcc_recall({ query?, scope?, offset?, limit?, expand? })
```

Default scope is the active branch. Use `scope: "all"` only when off-branch history is relevant.

## Safety

VCC performs no LLM summarization and declares no filesystem, shell, network, or secret permissions. It only reads and mutates the current Oino session through host-owned session APIs.
