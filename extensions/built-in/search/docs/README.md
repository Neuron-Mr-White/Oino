# oino.search

Deferred `grep`/`find` tools over the project tree. They are registered but
not model-visible until loaded through the host `tool_search` tool, keeping
the cached prompt prefix small.

- `grep` — `{pattern, path?, glob?, case_insensitive?, max_results?}` regex
  search; respects `.gitignore`; sorted by path then line; 30k char cap.
- `find` — `{pattern, path?, max_results?}` glob file search; sorted; 30k cap.

Read-only filesystem permission on `${project}` only.
