# UPSTREAM — imported verbatim from unipi-app

- **Source path**: `/home/oi/Projects/Personal/archived/unipi/unipi-app` (READ-ONLY archive;
  never modified by the import)
- **Source commit**: `336b60aff0825c03ff1b627f11bb62a2eac3814b`
  ("release 1.0.0 — first stable UniPi app (for UniPi 3.0); iPhone build no longer published",
  2026-10-25)
- **Imported paths**: `apps/mobile`, `crates`, `packages`, `protocol`, `Cargo.toml`,
  `Cargo.lock`, `package.json`, `package-lock.json`, `DESIGN.md`, `.gitignore`
- **v2 layout (WP-H1)**: the import is dissolved — `crates/{proto,host}` moved to the
  root workspace as `crates/unipi-{proto,host}`, `apps/mobile` became this directory
  (`apps/unipi`, own npm workspace + `packages/protocol`, src-tauri is its own cargo
  workspace), the shared fixtures stay in `protocol/fixtures`, and the old workspace
  manifests (`Cargo.toml`, `Cargo.lock`, `DESIGN.md`) are gone.
- **Not imported**: `docs/` (≈180 MB of screenshots) — read it from the archive path above;
  `LICENSE` and `README.md` (do not exist upstream); `.agents`, `scripts`, `TASK*.md`.
- **Licence**: MIT (per the UniPi project; no LICENSE file exists in the archived tree).
- **Provenance check**: all 825 imported files are byte-identical to the source blobs at the
  commit above (`git ls-files -s` vs `git hash-object`).

Imported verbatim; v2 changes follow in later commits.
