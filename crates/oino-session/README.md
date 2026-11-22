<img src="https://raw.githubusercontent.com/Neuron-Mr-White/Oino/main/docs/assets/brand/png/oino-128.png" alt="oino" width="64">

# oino-session

Session format v3 for Oino: entries, migration, context projection and the `SessionManager`.

* `entry` — `SessionHeader`, `SessionEntry`, entry kinds, ids, ISO timestamps
* `messages` — LLM conversion, summary prefixes, bash-execution text
* `context` — entry index, session projection, session context
* `migrate` — format detection (v1/v2/v3), parsing, pre-v1 Oino legacy mapping
* `manager` — append-only persistence (feature `manager`): listing, repair, readonly handles

Files live at `<sessions root>/--<cwd>--/<ts>_<sessionId>.jsonl` and are append-only; a file
is created only once the first user or assistant message exists. Older v1/v2 files are
migrated in place when opened.
