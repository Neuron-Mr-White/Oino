# unipi-kit

Shared guest-side kit for the unipi 3.0.0 ports (unipi `packages/core` + the umbrella). Depends
only on the `oino-extension` SDK. Add it with `unipi-kit.workspace = true`.

| Module | Use |
|---|---|
| `suite` | `LOAD_ORDER`, `pkg::*` (ids, dirs, npm names, namespaces, batch), `command("x")` → `x`, `namespace_owner` |
| `topics` | every `unipi:*` topic name, `STICKY`, payload structs implementing `Topic` |
| `bus` | `emit(&payload)` / `emit_raw(topic, &v)` (sticky applied automatically), `get::<T>()`, `subscribe(&[...])`, `Delivery::from_event(&ev)` for `oino_bus`, `route(name)` |
| `services` | fan-out names (`evidence.contribute`, `compaction-context.provide`, `skill-condition.check`, `move.handle`, `work-tray.tab`, `info-screen.groups`), point names, payload structs |
| `fanout` | `call_all(name, &payload, timeout, except)`, `gather_evidence(timeout)` |
| `settings` | `Namespace`/`Section`/`Field` builder (ui-protocol §6.6), `Store` (defaults ⊕ host values, refresh on `oino_settings_changed`), `Values` accessors |
| `jev` | `Settings::resolve(ext_settings, Some(ns))`, `ask(&settings, state, &questions)` (fail-open), `namespace()`, `consumer_section()` |
| `provenance` | `Harness::new(source, title).synopsis(..).delivery(HarnessDelivery::FollowUp).send_custom(..)` |
| `bounded` | `bound_model_output(text, &Options)` (64 KiB head/tail + 0600 artifact, host path shown) |
| `attach` | `find_paths`, `tokenize`, `expand_for_submit`, `[Image #N]` / `[File #N]` |
| `savings` | `compaction_savings(&branch)` / `Accumulator` |
| `tray` | work-tray contract (`TrayTab`, `TabRequest`, `Action`, `ActionReply`), `WorkItem`/`Dot`, `tray_hint_text` |
| `paths` | `global/workspace/session/tmp/cache/logs(rel)`, `host_path`, `read_json`/`write_json` |
| `fmt` | `format_tokens`, `format_bytes`, `compare_versions`, `parse_args`, `iso_from_ms`, `truncate_to_width` |
| `ous` | small OUS node builders |

Serving a work-tray tab (background-tasks, subagents, dream):

```rust
cx.service(unipi_kit::services::fanout::WORK_TRAY_TAB, |ext, req, _caller, _cx| async move {
    let req: TabRequest = serde_json::from_value(req).unwrap_or_default();
    Ok(json!([TrayTab { id: "bg".into(), label: "Background tasks".into(), short_label: Some("Bg".into()),
        order: 0, counts: ext.counts(), visible: true, body: req.open.then(|| ext.pane()), owner: None }]))
});
cx.service(unipi_kit::services::point::WORK_TRAY_ACTION, |ext, a, _caller, _cx| async move {
    let a: Action = serde_json::from_value(a)?;   // actionId without the "bg:" prefix
    Ok(json!(ActionReply { close: false, refresh: true }))
});
// on change:
unipi_kit::bus::emit_raw(unipi_kit::topics::WORK_TRAY_CHANGED, &json!({}));
```

Tests: `cargo test -p unipi-kit` (native, in `extensions/`). Deviations and open requests:
`docs/dev/v1/deviations/unipi-core.md`.
