# Autonomy controls

Operate surface. Preserve Unipi's existing toolbar, warm orange token, Inter UI,
shared tooltip host, phone Sheet, dock action buttons, and Settings primitives.
Autonomy always has three visible bars with exact level tooltips. A click cycles
Low → Medium → High → Low. Plan remains an independent toggle.

Host-generated approvals sit in the existing dock above the composer. Show the
canonical command, summary, effective risk, ceiling and host reason. Allow once,
Always allow and Deny submit one request identity to its receiving session channel.
Foreground child requests retain the child session id but are answered on the
parent channel, per the host protocol. Pending submission disables every decision.
Disconnected views never submit or retain requests.

The render-cutoff divider marks the inclusive anchor on the active branch. It
never removes human history. Missing/invalid anchors do not create a divider.

## Motion
- Button: no catalog entry fits the dense operational controls without adding
  decorative motion. Reuse existing shared toolbar/dock hover and instant focus.
- Tooltip: reuse the shared TooltipHost's established 300 ms delay and 120 ms fade,
  including existing reduced-motion behavior. No duplicate tooltip surface.
- Sheet: reuse the existing shared phone Sheet motion and focus management.
- Divider and approval: static, no entrance travel or scale.

## Settings extension hook
Compaction/Search modules are discovered only when present. Each downstream-owned
`CompactionSettings.tsx` or `SearchSettings.tsx` exports a default Solid component.
Both desktop and phone Settings use the same registration and section body. No
placeholder controls or imports of absent modules are shipped.
