/** OIN-20's finish report, plus a layout stress case for the report reader. */
export const oin20Report = 'Commit e1cdd43f. Root cause: the installed ors.bridge extension registers provider+API `ors`, but the restricted Superagent session used an isolated extension loader (HostLoader::memory_only, apps/unipi/crates/unipi-host/src/sessions/services.rs) that admitted only unipi.memory, so no provider component was loaded and dispatch failed at crates/oino-core/src/provider.rs:26. Fix: the restricted loader also admits global (non-project) provider extensions; project extensions and other extensions stay excluded. Regression test restricted_loader_keeps_global_providers_and_memory_only; `cargo test -p unipi-host` passes. Not verified with a live ORS turn. Owner: install a build with the new host and restart; the existing Superagent session needs no model re-pick.';

export const stressReport = `# Layout stress test

A long path: \`/home/example/${'unbroken-path-segment-'.repeat(14)}file.rs\`

\`\`\`text
/home/example/${'unbroken-code-segment-'.repeat(14)}file.rs
\`\`\`

| First column | Second column |
| --- | --- |
| A value | Another value |
`;
