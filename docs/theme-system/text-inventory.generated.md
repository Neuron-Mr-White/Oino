# Oino TUI Text Inventory (Generated)

Generated from non-test portions of `crates/oino-tui/src/*.rs` as a first-pass inventory for theme classification. This intentionally includes status strings, labels, separators, theme token names, and command/help text so later iterations can decide which literals should remain plain text and which should become component roles.

Total string literals captured: **1574**.

Format: `file:line | classification | literal`.

```text
crates/oino-tui/src/render.rs:45 | uncategorized | › 
crates/oino-tui/src/render.rs:46 | uncategorized | Oino needs at least 20x8
crates/oino-tui/src/render.rs:208 | uncategorized | accent
crates/oino-tui/src/render.rs:213 | uncategorized | success
crates/oino-tui/src/render.rs:217 | uncategorized | text
crates/oino-tui/src/render.rs:217 | uncategorized | fg
crates/oino-tui/src/render.rs:218 | uncategorized | muted
crates/oino-tui/src/render.rs:222 | uncategorized | dim
crates/oino-tui/src/render.rs:226 | uncategorized | focused_border
crates/oino-tui/src/render.rs:226 | uncategorized | border_accent
crates/oino-tui/src/render.rs:227 | uncategorized | panel_border
crates/oino-tui/src/render.rs:227 | uncategorized | border
crates/oino-tui/src/render.rs:227 | uncategorized | border_muted
crates/oino-tui/src/render.rs:228 | uncategorized | user_border
crates/oino-tui/src/render.rs:228 | uncategorized | user_message_text
crates/oino-tui/src/render.rs:229 | uncategorized | assistant_border
crates/oino-tui/src/render.rs:229 | uncategorized | assistant_message_text
crates/oino-tui/src/render.rs:230 | uncategorized | tool_border
crates/oino-tui/src/render.rs:230 | uncategorized | tool_title
crates/oino-tui/src/render.rs:231 | uncategorized | title
crates/oino-tui/src/render.rs:232 | uncategorized | warning
crates/oino-tui/src/render.rs:233 | uncategorized | error
crates/oino-tui/src/render.rs:234 | uncategorized | footer
crates/oino-tui/src/render.rs:234 | uncategorized | status
crates/oino-tui/src/render.rs:234 | uncategorized | inline_status
crates/oino-tui/src/render.rs:235 | uncategorized | working
crates/oino-tui/src/render.rs:235 | uncategorized | working_indicator
crates/oino-tui/src/render.rs:262 | uncategorized | default
crates/oino-tui/src/render.rs:263 | uncategorized | reset
crates/oino-tui/src/render.rs:274 | uncategorized | black
crates/oino-tui/src/render.rs:275 | uncategorized | red
crates/oino-tui/src/render.rs:276 | uncategorized | green
crates/oino-tui/src/render.rs:277 | uncategorized | yellow
crates/oino-tui/src/render.rs:278 | uncategorized | blue
crates/oino-tui/src/render.rs:279 | uncategorized | magenta
crates/oino-tui/src/render.rs:280 | uncategorized | cyan
crates/oino-tui/src/render.rs:281 | uncategorized | gray
crates/oino-tui/src/render.rs:281 | uncategorized | grey
crates/oino-tui/src/render.rs:282 | uncategorized | dark_gray
crates/oino-tui/src/render.rs:282 | uncategorized | dark-grey
crates/oino-tui/src/render.rs:282 | uncategorized | darkgray
crates/oino-tui/src/render.rs:283 | uncategorized | light_red
crates/oino-tui/src/render.rs:283 | uncategorized | light-red
crates/oino-tui/src/render.rs:284 | uncategorized | light_green
crates/oino-tui/src/render.rs:284 | uncategorized | light-green
crates/oino-tui/src/render.rs:285 | uncategorized | light_yellow
crates/oino-tui/src/render.rs:285 | uncategorized | light-yellow
crates/oino-tui/src/render.rs:286 | uncategorized | light_blue
crates/oino-tui/src/render.rs:286 | uncategorized | light-blue
crates/oino-tui/src/render.rs:287 | uncategorized | light_magenta
crates/oino-tui/src/render.rs:287 | uncategorized | light-magenta
crates/oino-tui/src/render.rs:288 | uncategorized | light_cyan
crates/oino-tui/src/render.rs:288 | uncategorized | light-cyan
crates/oino-tui/src/render.rs:289 | uncategorized | white
crates/oino-tui/src/render.rs:416 | extension surfaces / chrome |  {}: Enter queue • / draft • s settings • q send panel • Esc cancel 
crates/oino-tui/src/render.rs:433 | extension surfaces / chrome | Ext: {}
crates/oino-tui/src/render.rs:433 | extension surfaces / chrome | , 
crates/oino-tui/src/render.rs:538 | extension surfaces / chrome | : 
crates/oino-tui/src/render.rs:543 | extension surfaces / chrome |  ⚠ conflict
crates/oino-tui/src/render.rs:587 | extension surfaces / chrome | Extensions
crates/oino-tui/src/render.rs:611 | extension surfaces / chrome | Extension Main
crates/oino-tui/src/render.rs:631 | extension surfaces / chrome | Extension Status
crates/oino-tui/src/render.rs:683 | extension surfaces / chrome | Extension Panel
crates/oino-tui/src/render.rs:734 | extension surfaces / chrome |  • 
crates/oino-tui/src/render.rs:747 | extension surfaces / chrome | Ext: {labels}
crates/oino-tui/src/render.rs:780 | extension surfaces / chrome |  Extension Settings 
crates/oino-tui/src/render.rs:825 | extension surfaces / chrome | ▶ 
crates/oino-tui/src/render.rs:825 | extension surfaces / chrome |   
crates/oino-tui/src/render.rs:842 | extension surfaces / chrome | tabs {slot}: 
crates/oino-tui/src/render.rs:845 | extension surfaces / chrome |  | 
crates/oino-tui/src/render.rs:868 | extension surfaces / chrome | FOCUS • 
crates/oino-tui/src/render.rs:877 | extension surfaces / chrome |  • FOCUSED
crates/oino-tui/src/render.rs:879 | extension surfaces / chrome |  {label} {count}{focus} 
crates/oino-tui/src/render.rs:882 | extension surfaces / chrome |  {label} {count} • {conflicts} conflict{}{focus} 
crates/oino-tui/src/render.rs:883 | extension surfaces / chrome | s
crates/oino-tui/src/render.rs:1042 | main shell / status | Generating…
crates/oino-tui/src/render.rs:1043 | main shell / status | Generating…
crates/oino-tui/src/render.rs:1072 | main shell / status | Oino
crates/oino-tui/src/render.rs:1074 | main shell / status | Oino • {title}
crates/oino-tui/src/render.rs:1077 | main shell / status |  {base} ↑{offset} 
crates/oino-tui/src/render.rs:1078 | main shell / status |  {base} 
crates/oino-tui/src/render.rs:1085 | main shell / status | Oino • {status}
crates/oino-tui/src/render.rs:1087 | main shell / status | Oino • {title} • {status}
crates/oino-tui/src/render.rs:1090 | main shell / status |  {base} ↑{offset} 
crates/oino-tui/src/render.rs:1091 | main shell / status |  {base} 
crates/oino-tui/src/render.rs:1167 | main shell / status | <empty>
crates/oino-tui/src/render.rs:1171 | main shell / status | tool:
crates/oino-tui/src/render.rs:1262 | main shell / status | No messages yet. Send a task to start.
crates/oino-tui/src/render.rs:1273 | main shell / status | ● 
crates/oino-tui/src/render.rs:1278 | main shell / status | • 
crates/oino-tui/src/render.rs:1386 | composer / suggestions |  ↗ 
crates/oino-tui/src/render.rs:1393 | composer / suggestions | • 
crates/oino-tui/src/render.rs:1393 | composer / suggestions | ✓ 
crates/oino-tui/src/render.rs:1393 | composer / suggestions | ○ 
crates/oino-tui/src/render.rs:1393 | composer / suggestions | ☑ 
crates/oino-tui/src/render.rs:1393 | composer / suggestions | ☐ 
crates/oino-tui/src/render.rs:1393 | composer / suggestions | : 
crates/oino-tui/src/render.rs:1393 | composer / suggestions | │ 
crates/oino-tui/src/render.rs:1393 | composer / suggestions | | 
crates/oino-tui/src/render.rs:1417 | composer / suggestions | [image:
crates/oino-tui/src/render.rs:1421 | composer / suggestions | ] (
crates/oino-tui/src/render.rs:1424 | composer / suggestions | ] (
crates/oino-tui/src/render.rs:1440 | composer / suggestions | https://
crates/oino-tui/src/render.rs:1440 | composer / suggestions | http://
crates/oino-tui/src/render.rs:1479 | composer / suggestions | │
crates/oino-tui/src/render.rs:1481 | composer / suggestions | ┃
crates/oino-tui/src/render.rs:1494 | composer / suggestions |  Task • steer while streaming 
crates/oino-tui/src/render.rs:1496 | composer / suggestions |  Task 
crates/oino-tui/src/render.rs:1545 | composer / suggestions | No suggestion matches `{}`
crates/oino-tui/src/render.rs:1565 | composer / suggestions |  
crates/oino-tui/src/render.rs:1571 | composer / suggestions |  {}
crates/oino-tui/src/render.rs:1573 | composer / suggestions |   {}
crates/oino-tui/src/render.rs:1585 | composer / suggestions |  {} 
crates/oino-tui/src/render.rs:1597 | composer / suggestions | Files
crates/oino-tui/src/render.rs:1598 | composer / suggestions | Models
crates/oino-tui/src/render.rs:1630 | composer / suggestions | Models
crates/oino-tui/src/render.rs:1632 | composer / suggestions | {} {}/{}
crates/oino-tui/src/render.rs:1650 | inspect / help overlay |  Inspect 
crates/oino-tui/src/render.rs:1673 | inspect / help overlay | › 
crates/oino-tui/src/render.rs:1675 | inspect / help overlay | Full prompt
crates/oino-tui/src/render.rs:1678 | inspect / help overlay |  • loading…
crates/oino-tui/src/render.rs:1682 | inspect / help overlay | › 
crates/oino-tui/src/render.rs:1684 | inspect / help overlay | Full prompt
crates/oino-tui/src/render.rs:1688 | inspect / help overlay |  • {} tokens
crates/oino-tui/src/render.rs:1696 | inspect / help overlay | Press e to export chat as HTML
crates/oino-tui/src/render.rs:1707 | inspect / help overlay | Loading inspect snapshot…
crates/oino-tui/src/render.rs:1712 | inspect / help overlay | No prompt snapshot available.
crates/oino-tui/src/render.rs:1739 | inspect / help overlay | ↑/↓ scroll • PgUp/PgDn page • e export • q/Esc close • {}/{}
crates/oino-tui/src/render.rs:1744 | inspect / help overlay | e export • q/Esc close
crates/oino-tui/src/render.rs:1757 | inspect / help overlay |  Help 
crates/oino-tui/src/render.rs:1785 | inspect / help overlay | No help topics match `{}`
crates/oino-tui/src/render.rs:1803 | inspect / help overlay |  Oino Help 
crates/oino-tui/src/render.rs:1806 | inspect / help overlay |  Oino Help {}-{} / {} 
crates/oino-tui/src/render.rs:1814 | inspect / help overlay |  Oino Help {} match{} for `{}` 
crates/oino-tui/src/render.rs:1819 | inspect / help overlay | es
crates/oino-tui/src/render.rs:1837 | inspect / help overlay | type to fuzzy search • ↑/↓ scroll • Enter keep results • Esc clear search
crates/oino-tui/src/render.rs:1839 | inspect / help overlay | Press / to search • Esc/q close
crates/oino-tui/src/render.rs:1841 | inspect / help overlay | ↑/↓ or j/k scroll • / search • PgUp/PgDn page • Home/End jump • Esc/q close
crates/oino-tui/src/render.rs:1852 | inspect / help overlay | Search: 
crates/oino-tui/src/render.rs:1854 | inspect / help overlay | █
crates/oino-tui/src/render.rs:1858 | inspect / help overlay | Press / to search help
crates/oino-tui/src/render.rs:1861 | inspect / help overlay | Search: 
crates/oino-tui/src/render.rs:1874 | inspect / help overlay |  — 
crates/oino-tui/src/render.rs:1875 | inspect / help overlay | {key}{separator}
crates/oino-tui/src/render.rs:1897 | inspect / help overlay |  Send Panel 
crates/oino-tui/src/render.rs:1918 | send panel / sessions overlay |  Steer / Queue / Draft 
crates/oino-tui/src/render.rs:1927 | send panel / sessions overlay | Press y to confirm deletion • n/Esc cancel
crates/oino-tui/src/render.rs:1929 | send panel / sessions overlay | ↑/↓ select • Enter load • q queue input • d draft input • x delete • Esc close
crates/oino-tui/src/render.rs:1934 | send panel / sessions overlay | {} • {controls}
crates/oino-tui/src/render.rs:1950 | send panel / sessions overlay | Input: 
crates/oino-tui/src/render.rs:1952 | send panel / sessions overlay | empty
crates/oino-tui/src/render.rs:1969 | send panel / sessions overlay | Steer ({count}) — Enter while streaming
crates/oino-tui/src/render.rs:1970 | send panel / sessions overlay | Queue ({count}) — q sends current input here
crates/oino-tui/src/render.rs:1971 | send panel / sessions overlay | Draft ({count}) — d parks current input
crates/oino-tui/src/render.rs:1990 | send panel / sessions overlay |   (empty)
crates/oino-tui/src/render.rs:2015 | send panel / sessions overlay | {marker} {}. 
crates/oino-tui/src/render.rs:2055 | send panel / sessions overlay |  
crates/oino-tui/src/render.rs:2064 | send panel / sessions overlay |  Sessions 
crates/oino-tui/src/render.rs:2083 | send panel / sessions overlay |  Saved Sessions 
crates/oino-tui/src/render.rs:2085 | send panel / sessions overlay |  Saved Sessions 0/{} 
crates/oino-tui/src/render.rs:2088 | send panel / sessions overlay |  Saved Sessions {}/{} 
crates/oino-tui/src/render.rs:2098 | send panel / sessions overlay |  Saved Sessions {}/{} ({} total) 
crates/oino-tui/src/render.rs:2118 | send panel / sessions overlay | type to fuzzy search • ↑/↓ move • Enter continue • Esc clear search
crates/oino-tui/src/render.rs:2120 | send panel / sessions overlay | ↑/↓ select • / search • Enter continue • r reload • Esc close
crates/oino-tui/src/render.rs:2122 | send panel / sessions overlay | {} • {controls}
crates/oino-tui/src/render.rs:2140 | send panel / sessions overlay | Loading saved sessions…
crates/oino-tui/src/render.rs:2147 | send panel / sessions overlay | No saved sessions yet.
crates/oino-tui/src/render.rs:2152 | send panel / sessions overlay | Send a prompt to create one, or use /new when you explicitly want a fresh session.
crates/oino-tui/src/render.rs:2164 | send panel / sessions overlay | No sessions match `{}`
crates/oino-tui/src/render.rs:2197 | send panel / sessions overlay | Search: 
crates/oino-tui/src/render.rs:2199 | send panel / sessions overlay | █
crates/oino-tui/src/render.rs:2204 | send panel / sessions overlay | Press / to search sessions
crates/oino-tui/src/render.rs:2209 | send panel / sessions overlay | Search: 
crates/oino-tui/src/render.rs:2223 | send panel / sessions overlay | ●
crates/oino-tui/src/render.rs:2223 | send panel / sessions overlay |  
crates/oino-tui/src/render.rs:2224 | send panel / sessions overlay | {marker} {current} {}. 
crates/oino-tui/src/render.rs:2230 | send panel / sessions overlay |  - 
crates/oino-tui/src/render.rs:2263 | extensions overlay |  Extensions 
crates/oino-tui/src/render.rs:2291 | extensions overlay | type path, Git URL, or owner/repo • Enter install • Esc cancel
crates/oino-tui/src/render.rs:2293 | extensions overlay | Enter/Y uninstall • N/Esc cancel
crates/oino-tui/src/render.rs:2295 | extensions overlay | type to fuzzy search • ↑/↓ move • Enter toggle project • Esc clear search
crates/oino-tui/src/render.rs:2297 | extensions overlay | Tab switch tab • 1 Manage • 2 Registered • ↑/↓ select • / search • i/I install • u/x uninstall • g/p toggles • o/O prefer winner • c/C clear override • Esc close
crates/oino-tui/src/render.rs:2299 | extensions overlay | {} • {controls}
crates/oino-tui/src/render.rs:2322 | extensions overlay | {current}/{total}
crates/oino-tui/src/render.rs:2324 | extensions overlay | {current}/{filtered_len} ({total} in tab)
crates/oino-tui/src/render.rs:2327 | extensions overlay |  Extensions • {} tab • {suffix} 
crates/oino-tui/src/render.rs:2342 | extensions overlay | <package path, Git URL, or owner/repo>
crates/oino-tui/src/render.rs:2347 | extensions overlay | Install {} package: {input}
crates/oino-tui/src/render.rs:2352 | extensions overlay | Confirm uninstall {} package `{}`
crates/oino-tui/src/render.rs:2357 | extensions overlay | Search: {}
crates/oino-tui/src/render.rs:2359 | extensions overlay | Press / to search • i/I install • u/x uninstall • o/O prefer conflict winner • c/C clear override
crates/oino-tui/src/render.rs:2361 | extensions overlay | Filter: {}
crates/oino-tui/src/render.rs:2374 | extensions overlay | No extensions discovered yet.
crates/oino-tui/src/render.rs:2384 | extensions overlay | No extension items match `{}`
crates/oino-tui/src/render.rs:2402 | extensions overlay | ›
crates/oino-tui/src/render.rs:2402 | extensions overlay |  
crates/oino-tui/src/render.rs:2406 | extensions overlay |  • {} diag
crates/oino-tui/src/render.rs:2411 | extensions overlay |  • {} conflict
crates/oino-tui/src/render.rs:2415 | extensions overlay | {marker} P:{} G:{}{overrides} [{}] {} — {} • {} • {}{}{}
crates/oino-tui/src/render.rs:2444 | extensions overlay | {} ({})
crates/oino-tui/src/render.rs:2446 | extensions overlay | [{label}]
crates/oino-tui/src/render.rs:2452 | extensions overlay |   
crates/oino-tui/src/render.rs:2454 | extensions overlay | Tabs: {segments}
crates/oino-tui/src/render.rs:2467 | extensions overlay |  • {} diag
crates/oino-tui/src/render.rs:2472 | extensions overlay |  • {} conflict
crates/oino-tui/src/render.rs:2475 | extensions overlay | Selected: P:{} G:{}{} • [{}] {} — {} • {} • {}{}{}
crates/oino-tui/src/render.rs:2495 | extensions overlay | package
crates/oino-tui/src/render.rs:2496 | extensions overlay | extension
crates/oino-tui/src/render.rs:2499 | extensions overlay | {} {kind}
crates/oino-tui/src/render.rs:2504 | prompts / skills / resources overlay | ON
crates/oino-tui/src/render.rs:2506 | prompts / skills / resources overlay | OFF
crates/oino-tui/src/render.rs:2513 | prompts / skills / resources overlay |  OVR:G
crates/oino-tui/src/render.rs:2514 | prompts / skills / resources overlay |  OVR:P
crates/oino-tui/src/render.rs:2515 | prompts / skills / resources overlay |  OVR:G/P
crates/oino-tui/src/render.rs:2523 | prompts / skills / resources overlay |  Prompts 
crates/oino-tui/src/render.rs:2540 | prompts / skills / resources overlay | Prompt Templates
crates/oino-tui/src/render.rs:2557 | prompts / skills / resources overlay | type to fuzzy search • ↑/↓ move • Enter expand • Tab complete • Esc clear search
crates/oino-tui/src/render.rs:2559 | prompts / skills / resources overlay | ↑/↓ select • / search • Enter expand • Tab complete • r reload • Esc close
crates/oino-tui/src/render.rs:2561 | prompts / skills / resources overlay | {} • {controls}
crates/oino-tui/src/render.rs:2572 | prompts / skills / resources overlay |  Skills 
crates/oino-tui/src/render.rs:2589 | prompts / skills / resources overlay | Skills
crates/oino-tui/src/render.rs:2606 | prompts / skills / resources overlay | type to fuzzy search • ↑/↓ move • Enter run • Tab complete • Esc clear search
crates/oino-tui/src/render.rs:2608 | prompts / skills / resources overlay | ↑/↓ select • / search • Enter run • Tab complete • r reload • Esc close
crates/oino-tui/src/render.rs:2610 | prompts / skills / resources overlay | {} • {controls}
crates/oino-tui/src/render.rs:2626 | prompts / skills / resources overlay |  {label} 
crates/oino-tui/src/render.rs:2628 | prompts / skills / resources overlay |  {label} 0/{total} 
crates/oino-tui/src/render.rs:2631 | prompts / skills / resources overlay |  {label} {}/{} 
crates/oino-tui/src/render.rs:2637 | prompts / skills / resources overlay |  {label} {}/{} ({} total) 
crates/oino-tui/src/render.rs:2655 | prompts / skills / resources overlay | Press / to search prompts
crates/oino-tui/src/render.rs:2663 | prompts / skills / resources overlay | Reloading resources…
crates/oino-tui/src/render.rs:2670 | prompts / skills / resources overlay | No prompt templates found.
crates/oino-tui/src/render.rs:2675 | prompts / skills / resources overlay | Add Markdown files under <project>/.oino/prompts/.
crates/oino-tui/src/render.rs:2686 | prompts / skills / resources overlay | No prompts match `{}`
crates/oino-tui/src/render.rs:2730 | prompts / skills / resources overlay | Press / to search skills
crates/oino-tui/src/render.rs:2738 | prompts / skills / resources overlay | Reloading resources…
crates/oino-tui/src/render.rs:2745 | prompts / skills / resources overlay | No skills found.
crates/oino-tui/src/render.rs:2750 | prompts / skills / resources overlay | Add skills under ~/.oino/skills/ or <project>/.oino/skills/.
crates/oino-tui/src/render.rs:2761 | prompts / skills / resources overlay | No skills match `{}`
crates/oino-tui/src/render.rs:2803 | prompts / skills / resources overlay | Search: 
crates/oino-tui/src/render.rs:2805 | prompts / skills / resources overlay | █
crates/oino-tui/src/render.rs:2812 | prompts / skills / resources overlay | Search: 
crates/oino-tui/src/render.rs:2834 | prompts / skills / resources overlay | {marker} {}. {} [{}] — 
crates/oino-tui/src/render.rs:2839 | prompts / skills / resources overlay | {} • {}
crates/oino-tui/src/render.rs:2840 | prompts / skills / resources overlay | {prefix}{detail}
crates/oino-tui/src/render.rs:2854 | settings / keymaps overlay |  Settings 
crates/oino-tui/src/render.rs:2888 | settings / keymaps overlay | Choose a settings page:
crates/oino-tui/src/render.rs:2896 | settings / keymaps overlay | current: {}
crates/oino-tui/src/render.rs:2898 | settings / keymaps overlay | current: {}
crates/oino-tui/src/render.rs:2902 | settings / keymaps overlay | thinking: {}, tool: {}
crates/oino-tui/src/render.rs:2907 | settings / keymaps overlay | current: {}
crates/oino-tui/src/render.rs:2909 | settings / keymaps overlay | {} registered
crates/oino-tui/src/render.rs:2910 | settings / keymaps overlay | preset: {}
crates/oino-tui/src/render.rs:2913 | settings / keymaps overlay | {marker} {}  {}
crates/oino-tui/src/render.rs:2921 | settings / keymaps overlay |  Settings Pages 
crates/oino-tui/src/render.rs:2939 | settings / keymaps overlay |  Model Selection 
crates/oino-tui/src/render.rs:2942 | settings / keymaps overlay |  Model Selection {}/{} ({} total, refreshing) 
crates/oino-tui/src/render.rs:2951 | settings / keymaps overlay |  Model Selection {}/{} ({} total) 
crates/oino-tui/src/render.rs:2962 | settings / keymaps overlay | Loading model catalog…
crates/oino-tui/src/render.rs:2967 | settings / keymaps overlay | No models match `{}`
crates/oino-tui/src/render.rs:2986 | settings / keymaps overlay | {marker} {}
crates/oino-tui/src/render.rs:3006 | settings / keymaps overlay | Search: 
crates/oino-tui/src/render.rs:3008 | settings / keymaps overlay | █
crates/oino-tui/src/render.rs:3012 | settings / keymaps overlay | Press / to search models
crates/oino-tui/src/render.rs:3015 | settings / keymaps overlay | Search: 
crates/oino-tui/src/render.rs:3029 | settings / keymaps overlay | Model: {}
crates/oino-tui/src/render.rs:3038 | settings / keymaps overlay | {marker} {}
crates/oino-tui/src/render.rs:3044 | settings / keymaps overlay |  Thinking Level 
crates/oino-tui/src/render.rs:3060 | settings / keymaps overlay | Thinking
crates/oino-tui/src/render.rs:3061 | settings / keymaps overlay | Tool
crates/oino-tui/src/render.rs:3064 | settings / keymaps overlay | Enter cycles: Full → Truncate → Collapse
crates/oino-tui/src/render.rs:3072 | settings / keymaps overlay | {marker} {label}: {}
crates/oino-tui/src/render.rs:3080 | settings / keymaps overlay |  Collapse Mode 
crates/oino-tui/src/render.rs:3096 | settings / keymaps overlay | current rounded chat bubbles
crates/oino-tui/src/render.rs:3097 | settings / keymaps overlay | Codex-like activity transcript
crates/oino-tui/src/render.rs:3098 | settings / keymaps overlay | jcode-like compact transcript
crates/oino-tui/src/render.rs:3101 | settings / keymaps overlay | Changing style re-renders the current transcript immediately.
crates/oino-tui/src/render.rs:3115 | settings / keymaps overlay | {marker} {} ({}) — {description}
crates/oino-tui/src/render.rs:3127 | settings / keymaps overlay |  Chat Style 
crates/oino-tui/src/render.rs:3143 | settings / keymaps overlay |  Tools 
crates/oino-tui/src/render.rs:3146 | settings / keymaps overlay |  Tools {}/{} 
crates/oino-tui/src/render.rs:3155 | settings / keymaps overlay | Project controls this workspace. Global is the default copied into new projects.
crates/oino-tui/src/render.rs:3161 | settings / keymaps overlay | No tools registered.
crates/oino-tui/src/render.rs:3178 | settings / keymaps overlay | {marker} {}
crates/oino-tui/src/render.rs:3226 | settings / keymaps overlay | Preset: {} • Chord key: {} • Enter action • g edit chord key • p preset
crates/oino-tui/src/render.rs:3243 | settings / keymaps overlay | {marker} [{}] {}  —  {}
crates/oino-tui/src/render.rs:3255 | settings / keymaps overlay |  Keymaps {}/{} 
crates/oino-tui/src/render.rs:3277 | settings / keymaps overlay | {} ({})
crates/oino-tui/src/render.rs:3284 | settings / keymaps overlay | › Unassigned
crates/oino-tui/src/render.rs:3290 | settings / keymaps overlay | {marker} {binding}
crates/oino-tui/src/render.rs:3299 | settings / keymaps overlay |  Keymap Action 
crates/oino-tui/src/render.rs:3317 | settings / keymaps overlay | Choose shortcut type for {}
crates/oino-tui/src/render.rs:3325 | settings / keymaps overlay | global chord key plus one suffix key
crates/oino-tui/src/render.rs:3326 | settings / keymaps overlay | one key event, e.g. F2 or Ctrl-S
crates/oino-tui/src/render.rs:3330 | settings / keymaps overlay | {} {} — {}
crates/oino-tui/src/render.rs:3342 | settings / keymaps overlay |  Shortcut Type 
crates/oino-tui/src/render.rs:3361 | settings / keymaps overlay | (none yet)
crates/oino-tui/src/render.rs:3367 | settings / keymaps overlay |  
crates/oino-tui/src/render.rs:3370 | settings / keymaps overlay | Press the key combination to assign. Esc cancels.
crates/oino-tui/src/render.rs:3372 | settings / keymaps overlay | Press the suffix key. The global chord key is prepended. Esc cancels.
crates/oino-tui/src/render.rs:3375 | settings / keymaps overlay | Press the suffix key. The global chord key is prepended. Esc cancels.
crates/oino-tui/src/render.rs:3380 | settings / keymaps overlay | Assigning {}
crates/oino-tui/src/render.rs:3384 | settings / keymaps overlay | Type: {}
crates/oino-tui/src/render.rs:3388 | settings / keymaps overlay | Captured: {captured}
crates/oino-tui/src/render.rs:3398 | settings / keymaps overlay |  Listening for Shortcut 
crates/oino-tui/src/render.rs:3415 | settings / keymaps overlay | Set the global chord key
crates/oino-tui/src/render.rs:3419 | settings / keymaps overlay | Current: {}
crates/oino-tui/src/render.rs:3424 | settings / keymaps overlay | Press one key event such as Ctrl-X, Alt-Space, or F12.
crates/oino-tui/src/render.rs:3428 | settings / keymaps overlay | Plain text keys are disallowed so normal typing still works. Esc cancels.
crates/oino-tui/src/render.rs:3436 | settings / keymaps overlay |  Global Chord Key 
crates/oino-tui/src/render.rs:3453 | settings / keymaps overlay | Select a preset. Applying it resets every keybind after confirmation.
crates/oino-tui/src/render.rs:3466 | settings / keymaps overlay | {} {}
crates/oino-tui/src/render.rs:3475 | settings / keymaps overlay |  Keymap Preset 
crates/oino-tui/src/render.rs:3492 | settings / keymaps overlay | Reset every keybind to the {} preset?
crates/oino-tui/src/render.rs:3497 | settings / keymaps overlay | Y confirms • N/Esc cancels
crates/oino-tui/src/render.rs:3505 | settings / keymaps overlay |  Confirm Preset Reset 
crates/oino-tui/src/render.rs:3521 | settings / keymaps overlay | arrows/jk move • Enter/→ open • Esc close
crates/oino-tui/src/render.rs:3523 | settings / keymaps overlay | type to search • arrows move matches • Enter keep search • Esc clear search
crates/oino-tui/src/render.rs:3525 | settings / keymaps overlay | arrows/jk move • / search • Enter apply • Esc/← back
crates/oino-tui/src/render.rs:3526 | settings / keymaps overlay | arrows/jk move • Enter apply • Esc/← back • Ctrl-C twice quit
crates/oino-tui/src/render.rs:3527 | settings / keymaps overlay | arrows/jk move • Enter/→ cycle • Esc/← back
crates/oino-tui/src/render.rs:3528 | settings / keymaps overlay | arrows/jk move • Enter apply • Esc/← back
crates/oino-tui/src/render.rs:3530 | settings / keymaps overlay | arrows/jk move • g toggle global • p/Space/Enter toggle project • Esc/← back
crates/oino-tui/src/render.rs:3534 | settings / keymaps overlay | arrows/jk move • Enter detail • g chord key • p preset • Esc/← back
crates/oino-tui/src/render.rs:3537 | settings / keymaps overlay | arrows/jk move • Enter edit • a add • x remove • c clear • r reset • Esc back
crates/oino-tui/src/render.rs:3539 | settings / keymaps overlay | arrows/jk choose type • Enter listen • Esc back
crates/oino-tui/src/render.rs:3540 | settings / keymaps overlay | press shortcut input • Esc cancel
crates/oino-tui/src/render.rs:3541 | settings / keymaps overlay | press global chord key • Esc cancel
crates/oino-tui/src/render.rs:3542 | settings / keymaps overlay | arrows/jk choose preset • Enter confirm • Esc back
crates/oino-tui/src/render.rs:3543 | settings / keymaps overlay | Y reset all keybinds • N/Esc cancel
crates/oino-tui/src/render.rs:3547 | settings / keymaps overlay | Project controls this workspace; Global seeds new projects • {controls}
crates/oino-tui/src/render.rs:3549 | settings / keymaps overlay | {} • {controls}
crates/oino-tui/src/render.rs:3599 | settings / keymaps overlay | ›
crates/oino-tui/src/render.rs:3601 | settings / keymaps overlay |  
crates/oino-tui/src/render.rs:3607 | settings / keymaps overlay | ● ›
crates/oino-tui/src/render.rs:3608 | settings / keymaps overlay |   ›
crates/oino-tui/src/render.rs:3609 | settings / keymaps overlay | ●  
crates/oino-tui/src/render.rs:3610 | settings / keymaps overlay |    
crates/oino-tui/src/render.rs:3651 | settings / keymaps overlay |  
crates/oino-tui/src/render.rs:3747 | settings / keymaps overlay | \n
crates/oino-tui/src/markdown.rs:301 | markdown / code rendering |  
crates/oino-tui/src/markdown.rs:304 | markdown / code rendering | [x] 
crates/oino-tui/src/markdown.rs:304 | markdown / code rendering | [ ] 
crates/oino-tui/src/markdown.rs:339 | markdown / code rendering |  ↗ 
crates/oino-tui/src/markdown.rs:388 | markdown / code rendering |  
crates/oino-tui/src/markdown.rs:394 | markdown / code rendering | ─
crates/oino-tui/src/markdown.rs:399 | markdown / code rendering | [{label}]
crates/oino-tui/src/markdown.rs:433 | markdown / code rendering | image
crates/oino-tui/src/markdown.rs:439 | markdown / code rendering | [image: {alt}] ({url})
crates/oino-tui/src/markdown.rs:441 | markdown / code rendering | [image: {alt}]
crates/oino-tui/src/markdown.rs:460 | markdown / code rendering | {}. 
crates/oino-tui/src/markdown.rs:464 | markdown / code rendering | • 
crates/oino-tui/src/markdown.rs:467 | markdown / code rendering | • 
crates/oino-tui/src/markdown.rs:469 | markdown / code rendering |  
crates/oino-tui/src/markdown.rs:480 | markdown / code rendering | ✓ 
crates/oino-tui/src/markdown.rs:482 | markdown / code rendering | ○ 
crates/oino-tui/src/markdown.rs:491 | markdown / code rendering |  
crates/oino-tui/src/markdown.rs:517 | markdown / code rendering | │ 
crates/oino-tui/src/markdown.rs:518 | markdown / code rendering | │ 
crates/oino-tui/src/markdown.rs:537 | markdown / code rendering | │ 
crates/oino-tui/src/markdown.rs:538 | markdown / code rendering | │ 
crates/oino-tui/src/markdown.rs:576 | markdown / code rendering | │ {} │
crates/oino-tui/src/markdown.rs:596 | markdown / code rendering | ▌ 
crates/oino-tui/src/markdown.rs:606 | markdown / code rendering | ─
crates/oino-tui/src/markdown.rs:613 | markdown / code rendering | ▌ 
crates/oino-tui/src/markdown.rs:638 | markdown / code rendering | code
crates/oino-tui/src/markdown.rs:656 | markdown / code rendering | │ 
crates/oino-tui/src/markdown.rs:659 | markdown / code rendering | {}{}
crates/oino-tui/src/markdown.rs:660 | markdown / code rendering |  
crates/oino-tui/src/markdown.rs:664 | markdown / code rendering |  
crates/oino-tui/src/markdown.rs:667 | markdown / code rendering |  │ 
crates/oino-tui/src/markdown.rs:671 | markdown / code rendering |  
crates/oino-tui/src/markdown.rs:673 | markdown / code rendering |  │
crates/oino-tui/src/markdown.rs:731 | markdown / code rendering | ┌
crates/oino-tui/src/markdown.rs:731 | markdown / code rendering | ┬
crates/oino-tui/src/markdown.rs:731 | markdown / code rendering | ┐
crates/oino-tui/src/markdown.rs:741 | markdown / code rendering | ├
crates/oino-tui/src/markdown.rs:741 | markdown / code rendering | ┼
crates/oino-tui/src/markdown.rs:741 | markdown / code rendering | ┤
crates/oino-tui/src/markdown.rs:744 | markdown / code rendering | └
crates/oino-tui/src/markdown.rs:744 | markdown / code rendering | ┴
crates/oino-tui/src/markdown.rs:744 | markdown / code rendering | ┘
crates/oino-tui/src/markdown.rs:757 | markdown / code rendering | ─
crates/oino-tui/src/markdown.rs:795 | markdown / code rendering | │ 
crates/oino-tui/src/markdown.rs:798 | markdown / code rendering |  │ 
crates/oino-tui/src/markdown.rs:811 | markdown / code rendering |  │
crates/oino-tui/src/markdown.rs:857 | markdown / code rendering | ─
crates/oino-tui/src/markdown.rs:861 | markdown / code rendering | {left}{inner}{right}
crates/oino-tui/src/markdown.rs:874 | markdown / code rendering | ─
crates/oino-tui/src/markdown.rs:898 | markdown / code rendering | {text}{}
crates/oino-tui/src/markdown.rs:898 | markdown / code rendering |  
crates/oino-tui/src/markdown.rs:971 | markdown / code rendering | c++
crates/oino-tui/src/markdown.rs:971 | markdown / code rendering | cpp
crates/oino-tui/src/markdown.rs:972 | markdown / code rendering | c#
crates/oino-tui/src/markdown.rs:972 | markdown / code rendering | cs
crates/oino-tui/src/markdown.rs:973 | markdown / code rendering | dockerfile
crates/oino-tui/src/markdown.rs:973 | markdown / code rendering | Dockerfile
crates/oino-tui/src/markdown.rs:974 | markdown / code rendering | golang
crates/oino-tui/src/markdown.rs:974 | markdown / code rendering | go
crates/oino-tui/src/markdown.rs:975 | markdown / code rendering | js
crates/oino-tui/src/markdown.rs:975 | markdown / code rendering | node
crates/oino-tui/src/markdown.rs:975 | markdown / code rendering | js
crates/oino-tui/src/markdown.rs:976 | markdown / code rendering | jsx
crates/oino-tui/src/markdown.rs:976 | markdown / code rendering | react
crates/oino-tui/src/markdown.rs:976 | markdown / code rendering | jsx
crates/oino-tui/src/markdown.rs:977 | markdown / code rendering | md
crates/oino-tui/src/markdown.rs:977 | markdown / code rendering | mdx
crates/oino-tui/src/markdown.rs:977 | markdown / code rendering | md
crates/oino-tui/src/markdown.rs:978 | markdown / code rendering | py
crates/oino-tui/src/markdown.rs:978 | markdown / code rendering | py
crates/oino-tui/src/markdown.rs:979 | markdown / code rendering | shell
crates/oino-tui/src/markdown.rs:979 | markdown / code rendering | sh
crates/oino-tui/src/markdown.rs:980 | markdown / code rendering | typescriptreact
crates/oino-tui/src/markdown.rs:980 | markdown / code rendering | tsx
crates/oino-tui/src/markdown.rs:981 | markdown / code rendering | yml
crates/oino-tui/src/markdown.rs:981 | markdown / code rendering | yaml
crates/oino-tui/src/markdown.rs:1024 | markdown / code rendering | ```
crates/oino-tui/src/markdown.rs:1024 | markdown / code rendering | ~~~
crates/oino-tui/src/markdown.rs:1112 | markdown / code rendering | md
crates/oino-tui/src/markdown.rs:1112 | markdown / code rendering | markdown
crates/oino-tui/src/markdown.rs:1219 | markdown / code rendering | ---
crates/oino-tui/src/markdown.rs:1374 | markdown / code rendering | {}{text}
crates/oino-tui/src/markdown.rs:1374 | markdown / code rendering |  
crates/oino-tui/src/markdown.rs:1378 | markdown / code rendering | {}{text}{}
crates/oino-tui/src/markdown.rs:1378 | markdown / code rendering |  
crates/oino-tui/src/markdown.rs:1378 | markdown / code rendering |  
crates/oino-tui/src/markdown.rs:1380 | markdown / code rendering | {text}{}
crates/oino-tui/src/markdown.rs:1380 | markdown / code rendering |  
crates/oino-tui/src/composer.rs:5 | composer text model | Ask Oino • /help • @ file paths
crates/oino-tui/src/composer.rs:428 | composer text model | \r\n
crates/oino-tui/src/composer.rs:428 | composer text model | \n
crates/oino-tui/src/composer.rs:428 | composer text model | \n
crates/oino-tui/src/composer.rs:441 | composer text model | pasted {lines} lines · {}
crates/oino-tui/src/composer.rs:446 | composer text model | ⟦{} · #{id} · Ctrl-O e expand⟧
crates/oino-tui/src/composer.rs:453 | composer text model | {:.1}k chars
crates/oino-tui/src/composer.rs:455 | composer text model | {chars} chars
crates/oino-tui/src/settings.rs:54 | settings state / statuses | Global
crates/oino-tui/src/settings.rs:55 | settings state / statuses | Project
crates/oino-tui/src/settings.rs:111 | settings state / statuses | {} - [Global - {}] [Project - {}]
crates/oino-tui/src/settings.rs:150 | settings state / statuses | snake_case
crates/oino-tui/src/settings.rs:170 | settings state / statuses | snake_case
crates/oino-tui/src/settings.rs:205 | settings state / statuses | Model Selection
crates/oino-tui/src/settings.rs:206 | settings state / statuses | Thinking Level
crates/oino-tui/src/settings.rs:207 | settings state / statuses | Collapse Mode
crates/oino-tui/src/settings.rs:208 | settings state / statuses | Chat Style
crates/oino-tui/src/settings.rs:209 | settings state / statuses | Tools
crates/oino-tui/src/settings.rs:210 | settings state / statuses | Keymaps
crates/oino-tui/src/settings.rs:299 | settings state / statuses | Model catalog not loaded yet
crates/oino-tui/src/settings.rs:583 | settings state / statuses | Listening for global chord key • Esc cancel
crates/oino-tui/src/settings.rs:673 | settings state / statuses | Listening for one key combination • Esc cancel
crates/oino-tui/src/settings.rs:675 | settings state / statuses | Listening for chord suffix key • Esc cancel
crates/oino-tui/src/settings.rs:691 | settings state / statuses | Unsupported terminal key event
crates/oino-tui/src/settings.rs:696 | settings state / statuses | Shortcut capture canceled
crates/oino-tui/src/settings.rs:713 | settings state / statuses | Unsupported terminal key event
crates/oino-tui/src/settings.rs:718 | settings state / statuses | Chord key capture canceled
crates/oino-tui/src/settings.rs:722 | settings state / statuses | Global chord key cannot be plain text; use Ctrl/Alt/F-key to avoid blocking typing
crates/oino-tui/src/settings.rs:727 | settings state / statuses | {} conflicts with {} ({})
crates/oino-tui/src/settings.rs:736 | settings state / statuses | Global chord key set to {stroke}
crates/oino-tui/src/settings.rs:775 | settings state / statuses | Reset all keybinds to {} preset
crates/oino-tui/src/settings.rs:780 | settings state / statuses | Preset reset canceled
crates/oino-tui/src/settings.rs:799 | settings state / statuses | No shortcut to remove
crates/oino-tui/src/settings.rs:810 | settings state / statuses | Removed {} from {}
crates/oino-tui/src/settings.rs:818 | settings state / statuses | {} is now unassigned
crates/oino-tui/src/settings.rs:826 | settings state / statuses | Reset {} to preset default
crates/oino-tui/src/settings.rs:836 | settings state / statuses | Shortcut cannot be empty; use Clear to unassign
crates/oino-tui/src/settings.rs:851 | settings state / statuses | {} is already assigned to {}
crates/oino-tui/src/settings.rs:857 | settings state / statuses | {} conflicts with {} ({})
crates/oino-tui/src/settings.rs:876 | settings state / statuses | Set {} to {}
crates/oino-tui/src/settings.rs:1002 | settings state / statuses | {} {}
crates/oino-tui/src/settings.rs:1113 | settings state / statuses | Full
crates/oino-tui/src/settings.rs:1114 | settings state / statuses | Truncate
crates/oino-tui/src/settings.rs:1115 | settings state / statuses | Collapse
crates/oino-tui/src/settings.rs:1122 | settings state / statuses | Chat
crates/oino-tui/src/settings.rs:1123 | settings state / statuses | Agentic
crates/oino-tui/src/settings.rs:1124 | settings state / statuses | Minimal
crates/oino-tui/src/settings.rs:1131 | settings state / statuses | chat
crates/oino-tui/src/settings.rs:1132 | settings state / statuses | agentic
crates/oino-tui/src/settings.rs:1133 | settings state / statuses | minimal
crates/oino-tui/src/settings.rs:1140 | settings state / statuses | chat
crates/oino-tui/src/settings.rs:1141 | settings state / statuses | agentic
crates/oino-tui/src/settings.rs:1142 | settings state / statuses | minimal
crates/oino-tui/src/settings.rs:1149 | settings state / statuses | Off
crates/oino-tui/src/settings.rs:1150 | settings state / statuses | Minimal
crates/oino-tui/src/settings.rs:1151 | settings state / statuses | Low
crates/oino-tui/src/settings.rs:1152 | settings state / statuses | Medium
crates/oino-tui/src/settings.rs:1153 | settings state / statuses | High
crates/oino-tui/src/settings.rs:1154 | settings state / statuses | X High
crates/oino-tui/src/settings.rs:1182 | settings state / statuses | ON
crates/oino-tui/src/settings.rs:1184 | settings state / statuses | OFF
crates/oino-tui/src/settings.rs:1194 | settings state / statuses | {}{}
crates/oino-tui/src/settings.rs:1198 | settings state / statuses |  
crates/oino-tui/src/settings.rs:1210 | settings state / statuses |  
crates/oino-tui/src/help.rs:31 | help content | Type /help any time to reopen this guide. {} or q closes it.
crates/oino-tui/src/help.rs:35 | help content | Composer
crates/oino-tui/src/help.rs:38 | help content | send a prompt; while the assistant is streaming, send steering text
crates/oino-tui/src/help.rs:42 | help content | insert a newline
crates/oino-tui/src/help.rs:45 | help content | /
crates/oino-tui/src/help.rs:46 | help content | open fuzzy command suggestions at the start of the input
crates/oino-tui/src/help.rs:49 | help content | @
crates/oino-tui/src/help.rs:50 | help content | fuzzy search project file paths; Tab inserts the highlighted path
crates/oino-tui/src/help.rs:53 | help content | /prompt:<name> / /skill:<name>
crates/oino-tui/src/help.rs:54 | help content | include prompt templates or skills in the submitted message
crates/oino-tui/src/help.rs:57 | help content | /P:<query> / /S:<query>
crates/oino-tui/src/help.rs:58 | help content | search prompt templates or skills anywhere in the composer
crates/oino-tui/src/help.rs:61 | help content | Paste
crates/oino-tui/src/help.rs:62 | help content | large or multiline pastes collapse visually but still submit in full
crates/oino-tui/src/help.rs:66 | help content | expand a collapsed pasted block at the cursor or prompt template references
crates/oino-tui/src/help.rs:69 | help content | Commands
crates/oino-tui/src/help.rs:70 | help content | /help
crates/oino-tui/src/help.rs:70 | help content | open this help overlay
crates/oino-tui/src/help.rs:71 | help content | /new
crates/oino-tui/src/help.rs:71 | help content | start a fresh session after this one has messages
crates/oino-tui/src/help.rs:73 | help content | /sessions
crates/oino-tui/src/help.rs:74 | help content | browse saved sessions; press Enter to continue one
crates/oino-tui/src/help.rs:76 | help content | /settings
crates/oino-tui/src/help.rs:76 | help content | open settings pages
crates/oino-tui/src/help.rs:78 | help content | /prompts
crates/oino-tui/src/help.rs:79 | help content | browse prompt templates from <project>/.oino/prompts/
crates/oino-tui/src/help.rs:82 | help content | /skills
crates/oino-tui/src/help.rs:83 | help content | browse skills from ~/.oino/skills/ and <project>/.oino/skills/
crates/oino-tui/src/help.rs:85 | help content | /reload
crates/oino-tui/src/help.rs:85 | help content | reload SYSTEM.md, AGENT.md, prompts, and skills
crates/oino-tui/src/help.rs:87 | help content | /inspect
crates/oino-tui/src/help.rs:88 | help content | inspect full prompt; press e there to export chat HTML
crates/oino-tui/src/help.rs:91 | help content | /skill:<name>
crates/oino-tui/src/help.rs:92 | help content | include a skill explicitly; repeat tokens to combine resources
crates/oino-tui/src/help.rs:95 | help content | /model <provider:model>
crates/oino-tui/src/help.rs:96 | help content | change model directly, or /model to open model selection
crates/oino-tui/src/help.rs:99 | help content | /thinking <level>
crates/oino-tui/src/help.rs:100 | help content | set reasoning level: off, minimal, low, medium, high, xhigh
crates/oino-tui/src/help.rs:103 | help content | /title <text>
crates/oino-tui/src/help.rs:104 | help content | set the title shown in the transcript and sessions list
crates/oino-tui/src/help.rs:107 | help content | /settings tools
crates/oino-tui/src/help.rs:108 | help content | show registered agent tools by global/project scope
crates/oino-tui/src/help.rs:111 | help content | Transcript
crates/oino-tui/src/help.rs:114 | help content | scroll by page up
crates/oino-tui/src/help.rs:118 | help content | scroll by page down
crates/oino-tui/src/help.rs:122 | help content | scroll by line up
crates/oino-tui/src/help.rs:126 | help content | scroll by line down
crates/oino-tui/src/help.rs:128 | help content | jump to top
crates/oino-tui/src/help.rs:131 | help content | jump to bottom
crates/oino-tui/src/help.rs:135 | help content | focus transcript; navigation shortcuts then target transcript first
crates/oino-tui/src/help.rs:138 | help content | Ctrl-click links/images
crates/oino-tui/src/help.rs:139 | help content | open visible URL or image placeholders when the terminal supports it
crates/oino-tui/src/help.rs:142 | help content | Streaming, queue, and drafts
crates/oino-tui/src/help.rs:145 | help content | while streaming, steer the current response with the current input
crates/oino-tui/src/help.rs:149 | help content | queue current input for the next turn without opening the send panel
crates/oino-tui/src/help.rs:153 | help content | move current input to Draft without opening the send panel
crates/oino-tui/src/help.rs:155 | help content | open settings
crates/oino-tui/src/help.rs:158 | help content | open the send panel for steering history, queue, and drafts
crates/oino-tui/src/help.rs:161 | help content | Send panel {}
crates/oino-tui/src/help.rs:162 | help content | queue current input for the next turn
crates/oino-tui/src/help.rs:165 | help content | Send panel {}
crates/oino-tui/src/help.rs:166 | help content | move current input to Draft
crates/oino-tui/src/help.rs:170 | help content | Send panel {}
crates/oino-tui/src/help.rs:173 | help content | delete selected queued/draft item after confirmation
crates/oino-tui/src/help.rs:176 | help content | Overlays and exit
crates/oino-tui/src/help.rs:179 | help content | close the top overlay, clear search, or stop a running response; it does not quit
crates/oino-tui/src/help.rs:183 | help content | press twice to quit Oino; Ctrl-C twice remains a hard safety fallback
crates/oino-tui/src/help.rs:192 | help content | {key} {description}
crates/oino-tui/src/command.rs:29 | slash commands / suggestions | /settings
crates/oino-tui/src/command.rs:30 | slash commands / suggestions | Open or change settings
crates/oino-tui/src/command.rs:34 | slash commands / suggestions | /model
crates/oino-tui/src/command.rs:35 | slash commands / suggestions | Open or change model
crates/oino-tui/src/command.rs:39 | slash commands / suggestions | /thinking
crates/oino-tui/src/command.rs:40 | slash commands / suggestions | Open or change thinking level
crates/oino-tui/src/command.rs:44 | slash commands / suggestions | /title
crates/oino-tui/src/command.rs:45 | slash commands / suggestions | Set the current session title
crates/oino-tui/src/command.rs:49 | slash commands / suggestions | /new
crates/oino-tui/src/command.rs:50 | slash commands / suggestions | Start a new session
crates/oino-tui/src/command.rs:54 | slash commands / suggestions | /sessions
crates/oino-tui/src/command.rs:55 | slash commands / suggestions | Browse saved sessions
crates/oino-tui/src/command.rs:59 | slash commands / suggestions | /help
crates/oino-tui/src/command.rs:60 | slash commands / suggestions | Open keyboard and command help
crates/oino-tui/src/command.rs:64 | slash commands / suggestions | /inspect
crates/oino-tui/src/command.rs:65 | slash commands / suggestions | Inspect debug runtime state
crates/oino-tui/src/command.rs:69 | slash commands / suggestions | /extensions
crates/oino-tui/src/command.rs:70 | slash commands / suggestions | Manage installed extensions and contributions
crates/oino-tui/src/command.rs:74 | slash commands / suggestions | /prompts
crates/oino-tui/src/command.rs:75 | slash commands / suggestions | Browse prompt templates
crates/oino-tui/src/command.rs:79 | slash commands / suggestions | /skills
crates/oino-tui/src/command.rs:80 | slash commands / suggestions | Browse skills
crates/oino-tui/src/command.rs:84 | slash commands / suggestions | /reload
crates/oino-tui/src/command.rs:85 | slash commands / suggestions | Reload Oino resources
crates/oino-tui/src/command.rs:92 | slash commands / suggestions | /prompt:
crates/oino-tui/src/command.rs:93 | slash commands / suggestions | Include a prompt template by name
crates/oino-tui/src/command.rs:97 | slash commands / suggestions | /skill:
crates/oino-tui/src/command.rs:98 | slash commands / suggestions | Include a skill by name
crates/oino-tui/src/command.rs:264 | slash commands / suggestions | [SYS]
crates/oino-tui/src/command.rs:265 | slash commands / suggestions | [PROMPT]
crates/oino-tui/src/command.rs:266 | slash commands / suggestions | [SKILL]
crates/oino-tui/src/command.rs:267 | slash commands / suggestions | [EXT]
crates/oino-tui/src/command.rs:310 | slash commands / suggestions | /settings
crates/oino-tui/src/command.rs:311 | slash commands / suggestions | /settings
crates/oino-tui/src/command.rs:311 | slash commands / suggestions | model
crates/oino-tui/src/command.rs:314 | slash commands / suggestions | /model
crates/oino-tui/src/command.rs:315 | slash commands / suggestions | /settings
crates/oino-tui/src/command.rs:315 | slash commands / suggestions | thinking
crates/oino-tui/src/command.rs:318 | slash commands / suggestions | /thinking
crates/oino-tui/src/command.rs:319 | slash commands / suggestions | /settings
crates/oino-tui/src/command.rs:319 | slash commands / suggestions | collapse
crates/oino-tui/src/command.rs:323 | slash commands / suggestions | /settings
crates/oino-tui/src/command.rs:324 | slash commands / suggestions | collapse
crates/oino-tui/src/command.rs:330 | slash commands / suggestions | /settings
crates/oino-tui/src/command.rs:330 | slash commands / suggestions | chat-style
crates/oino-tui/src/command.rs:330 | slash commands / suggestions | chat_style
crates/oino-tui/src/command.rs:335 | slash commands / suggestions | /settings
crates/oino-tui/src/command.rs:335 | slash commands / suggestions | tools
crates/oino-tui/src/command.rs:335 | slash commands / suggestions | keymaps
crates/oino-tui/src/command.rs:372 | slash commands / suggestions | Files
crates/oino-tui/src/command.rs:382 | slash commands / suggestions | file
crates/oino-tui/src/command.rs:383 | slash commands / suggestions | @{file}
crates/oino-tui/src/command.rs:416 | slash commands / suggestions | /title 
crates/oino-tui/src/command.rs:424 | slash commands / suggestions | /help
crates/oino-tui/src/command.rs:425 | slash commands / suggestions | /new
crates/oino-tui/src/command.rs:426 | slash commands / suggestions | /sessions
crates/oino-tui/src/command.rs:427 | slash commands / suggestions | /prompts
crates/oino-tui/src/command.rs:428 | slash commands / suggestions | /skills
crates/oino-tui/src/command.rs:429 | slash commands / suggestions | /reload
crates/oino-tui/src/command.rs:430 | slash commands / suggestions | /inspect
crates/oino-tui/src/command.rs:431 | slash commands / suggestions | /extensions
crates/oino-tui/src/command.rs:432 | slash commands / suggestions | /settings
crates/oino-tui/src/command.rs:433 | slash commands / suggestions | /model
crates/oino-tui/src/command.rs:434 | slash commands / suggestions | /thinking
crates/oino-tui/src/command.rs:435 | slash commands / suggestions | /settings
crates/oino-tui/src/command.rs:435 | slash commands / suggestions | chat-style
crates/oino-tui/src/command.rs:435 | slash commands / suggestions | /settings
crates/oino-tui/src/command.rs:435 | slash commands / suggestions | chat_style
crates/oino-tui/src/command.rs:438 | slash commands / suggestions | /settings
crates/oino-tui/src/command.rs:438 | slash commands / suggestions | tools
crates/oino-tui/src/command.rs:439 | slash commands / suggestions | /settings
crates/oino-tui/src/command.rs:439 | slash commands / suggestions | keymaps
crates/oino-tui/src/command.rs:439 | slash commands / suggestions | /settings
crates/oino-tui/src/command.rs:439 | slash commands / suggestions | keymap
crates/oino-tui/src/command.rs:442 | slash commands / suggestions | /settings
crates/oino-tui/src/command.rs:442 | slash commands / suggestions | model
crates/oino-tui/src/command.rs:442 | slash commands / suggestions | /model
crates/oino-tui/src/command.rs:445 | slash commands / suggestions | /settings
crates/oino-tui/src/command.rs:445 | slash commands / suggestions | thinking
crates/oino-tui/src/command.rs:445 | slash commands / suggestions | /thinking
crates/oino-tui/src/command.rs:448 | slash commands / suggestions | /settings
crates/oino-tui/src/command.rs:448 | slash commands / suggestions | collapse
crates/oino-tui/src/command.rs:456 | slash commands / suggestions | /settings
crates/oino-tui/src/command.rs:456 | slash commands / suggestions | chat-style
crates/oino-tui/src/command.rs:456 | slash commands / suggestions | /settings
crates/oino-tui/src/command.rs:456 | slash commands / suggestions | chat_style
crates/oino-tui/src/command.rs:468 | slash commands / suggestions | off
crates/oino-tui/src/command.rs:469 | slash commands / suggestions | minimal
crates/oino-tui/src/command.rs:470 | slash commands / suggestions | low
crates/oino-tui/src/command.rs:471 | slash commands / suggestions | medium
crates/oino-tui/src/command.rs:472 | slash commands / suggestions | high
crates/oino-tui/src/command.rs:473 | slash commands / suggestions | xhigh
crates/oino-tui/src/command.rs:481 | slash commands / suggestions | off
crates/oino-tui/src/command.rs:482 | slash commands / suggestions | minimal
crates/oino-tui/src/command.rs:483 | slash commands / suggestions | low
crates/oino-tui/src/command.rs:484 | slash commands / suggestions | medium
crates/oino-tui/src/command.rs:485 | slash commands / suggestions | high
crates/oino-tui/src/command.rs:486 | slash commands / suggestions | xhigh
crates/oino-tui/src/command.rs:493 | slash commands / suggestions | thinking
crates/oino-tui/src/command.rs:494 | slash commands / suggestions | tool
crates/oino-tui/src/command.rs:502 | slash commands / suggestions | thinking
crates/oino-tui/src/command.rs:503 | slash commands / suggestions | tool
crates/oino-tui/src/command.rs:510 | slash commands / suggestions | full
crates/oino-tui/src/command.rs:511 | slash commands / suggestions | truncate
crates/oino-tui/src/command.rs:512 | slash commands / suggestions | collapse
crates/oino-tui/src/command.rs:520 | slash commands / suggestions | full
crates/oino-tui/src/command.rs:521 | slash commands / suggestions | truncate
crates/oino-tui/src/command.rs:522 | slash commands / suggestions | collapse
crates/oino-tui/src/command.rs:557 | slash commands / suggestions | Commands
crates/oino-tui/src/command.rs:562 | slash commands / suggestions | skill:
crates/oino-tui/src/command.rs:562 | slash commands / suggestions | /skill:
crates/oino-tui/src/command.rs:563 | slash commands / suggestions | prompt:
crates/oino-tui/src/command.rs:563 | slash commands / suggestions | /prompt:
crates/oino-tui/src/command.rs:622 | slash commands / suggestions | Prompts
crates/oino-tui/src/command.rs:638 | slash commands / suggestions | Skills
crates/oino-tui/src/command.rs:643 | slash commands / suggestions | /P:
crates/oino-tui/src/command.rs:644 | slash commands / suggestions | /prompt:
crates/oino-tui/src/command.rs:645 | slash commands / suggestions | /S:
crates/oino-tui/src/command.rs:646 | slash commands / suggestions | /skill:
crates/oino-tui/src/command.rs:667 | slash commands / suggestions | prompt:{} {}
crates/oino-tui/src/command.rs:706 | slash commands / suggestions | prompt:
crates/oino-tui/src/command.rs:708 | slash commands / suggestions |  
crates/oino-tui/src/command.rs:735 | slash commands / suggestions | skill:{} {}
crates/oino-tui/src/command.rs:774 | slash commands / suggestions | skill:
crates/oino-tui/src/command.rs:776 | slash commands / suggestions |  
crates/oino-tui/src/command.rs:787 | slash commands / suggestions | {} {}
crates/oino-tui/src/command.rs:792 | slash commands / suggestions | model
crates/oino-tui/src/command.rs:792 | slash commands / suggestions | Set selected model
crates/oino-tui/src/command.rs:793 | slash commands / suggestions | thinking
crates/oino-tui/src/command.rs:793 | slash commands / suggestions | Set thinking level
crates/oino-tui/src/command.rs:794 | slash commands / suggestions | collapse
crates/oino-tui/src/command.rs:794 | slash commands / suggestions | Set thinking/tool collapse mode
crates/oino-tui/src/command.rs:795 | slash commands / suggestions | chat-style
crates/oino-tui/src/command.rs:795 | slash commands / suggestions | Set transcript rendering style
crates/oino-tui/src/command.rs:796 | slash commands / suggestions | tools
crates/oino-tui/src/command.rs:796 | slash commands / suggestions | Show registered agent tools by scope
crates/oino-tui/src/command.rs:797 | slash commands / suggestions | keymaps
crates/oino-tui/src/command.rs:797 | slash commands / suggestions | Configure keyboard shortcuts
crates/oino-tui/src/command.rs:804 | slash commands / suggestions | {} {}
crates/oino-tui/src/command.rs:814 | slash commands / suggestions | Settings
crates/oino-tui/src/command.rs:829 | slash commands / suggestions | {} {}
crates/oino-tui/src/command.rs:847 | slash commands / suggestions | Models
crates/oino-tui/src/command.rs:860 | slash commands / suggestions |  
crates/oino-tui/src/command.rs:870 | slash commands / suggestions | Disable provider reasoning
crates/oino-tui/src/command.rs:871 | slash commands / suggestions | Minimal reasoning
crates/oino-tui/src/command.rs:872 | slash commands / suggestions | Low reasoning
crates/oino-tui/src/command.rs:873 | slash commands / suggestions | Medium reasoning
crates/oino-tui/src/command.rs:874 | slash commands / suggestions | High reasoning
crates/oino-tui/src/command.rs:875 | slash commands / suggestions | Extra-high reasoning
crates/oino-tui/src/command.rs:882 | slash commands / suggestions | {} {}
crates/oino-tui/src/command.rs:899 | slash commands / suggestions | Thinking
crates/oino-tui/src/command.rs:904 | slash commands / suggestions | thinking
crates/oino-tui/src/command.rs:904 | slash commands / suggestions | Thinking section
crates/oino-tui/src/command.rs:905 | slash commands / suggestions | tool
crates/oino-tui/src/command.rs:905 | slash commands / suggestions | Tool result bubbles
crates/oino-tui/src/command.rs:912 | slash commands / suggestions | {} {}
crates/oino-tui/src/command.rs:920 | slash commands / suggestions | Collapse Target
crates/oino-tui/src/command.rs:925 | slash commands / suggestions | full
crates/oino-tui/src/command.rs:925 | slash commands / suggestions | Show full content
crates/oino-tui/src/command.rs:926 | slash commands / suggestions | truncate
crates/oino-tui/src/command.rs:926 | slash commands / suggestions | Show short preview
crates/oino-tui/src/command.rs:927 | slash commands / suggestions | collapse
crates/oino-tui/src/command.rs:927 | slash commands / suggestions | Hide detailed content
crates/oino-tui/src/command.rs:934 | slash commands / suggestions | {} {}
crates/oino-tui/src/command.rs:950 | slash commands / suggestions | Collapse Mode
crates/oino-tui/src/command.rs:955 | slash commands / suggestions | Current bubble-style transcript
crates/oino-tui/src/command.rs:956 | slash commands / suggestions | Codex-like agent activity transcript
crates/oino-tui/src/command.rs:957 | slash commands / suggestions | jcode-like compact transcript
crates/oino-tui/src/command.rs:964 | slash commands / suggestions | {} {}
crates/oino-tui/src/command.rs:981 | slash commands / suggestions | Chat Style
crates/oino-tui/src/command.rs:1054 | slash commands / suggestions | /P:
crates/oino-tui/src/command.rs:1056 | slash commands / suggestions | /prompt:
crates/oino-tui/src/command.rs:1058 | slash commands / suggestions | /S:
crates/oino-tui/src/command.rs:1060 | slash commands / suggestions | /skill:
crates/oino-tui/src/app.rs:33 | state/status/action text | Type /help for shortcuts and commands
crates/oino-tui/src/app.rs:61 | state/status/action text | Steer
crates/oino-tui/src/app.rs:62 | state/status/action text | Queue
crates/oino-tui/src/app.rs:63 | state/status/action text | Draft
crates/oino-tui/src/app.rs:142 | state/status/action text | Manage
crates/oino-tui/src/app.rs:143 | state/status/action text | Registered
crates/oino-tui/src/app.rs:163 | state/status/action text | extension
crates/oino-tui/src/app.rs:164 | state/status/action text | package
crates/oino-tui/src/app.rs:165 | state/status/action text | contribution
crates/oino-tui/src/app.rs:195 | state/status/action text | {} {} {} {} {} {} {} {} {}
crates/oino-tui/src/app.rs:351 | state/status/action text | global
crates/oino-tui/src/app.rs:352 | state/status/action text | project
crates/oino-tui/src/app.rs:484 | state/status/action text | primary
crates/oino-tui/src/app.rs:489 | state/status/action text | {:?}:{slot}
crates/oino-tui/src/app.rs:686 | state/status/action text | built-in:{}
crates/oino-tui/src/app.rs:689 | state/status/action text | built-in:prefix
crates/oino-tui/src/app.rs:707 | state/status/action text | extension:{right_action}
crates/oino-tui/src/app.rs:710 | state/status/action text | extension:{left_action}
crates/oino-tui/src/app.rs:750 | state/status/action text | {} item{}
crates/oino-tui/src/app.rs:752 | state/status/action text | summary
crates/oino-tui/src/app.rs:755 | state/status/action text | title
crates/oino-tui/src/app.rs:758 | state/status/action text | rows
crates/oino-tui/src/app.rs:759 | state/status/action text | {} row{}
crates/oino-tui/src/app.rs:761 | state/status/action text | {} field{}
crates/oino-tui/src/app.rs:772 | state/status/action text | s
crates/oino-tui/src/app.rs:1038 | state/status/action text | No saved sessions yet
crates/oino-tui/src/app.rs:1040 | state/status/action text | Loaded {} saved sessions
crates/oino-tui/src/app.rs:1059 | state/status/action text | Loaded {} prompts and {} skills
crates/oino-tui/src/app.rs:1360 | state/status/action text | Transcript scrolled • PgUp/PgDn page • Alt-↑/↓ line • Ctrl-Home top • End bottom
crates/oino-tui/src/app.rs:1423 | state/status/action text | {} {} {}
crates/oino-tui/src/app.rs:1431 | state/status/action text | {} • {}
crates/oino-tui/src/app.rs:1442 | state/status/action text | Extension Suggestions
crates/oino-tui/src/app.rs:1516 | state/status/action text | assistant
crates/oino-tui/src/app.rs:1517 | state/status/action text | assistant
crates/oino-tui/src/app.rs:1562 | state/status/action text | assistant
crates/oino-tui/src/app.rs:1581 | state/status/action text | Calling {}… type and Enter to steer • Ctrl-O q queue/drafts
crates/oino-tui/src/app.rs:1803 | state/status/action text | Paste rejected: {paste_chars} chars exceeds the {MAX_PASTE_CHARS} char limit
crates/oino-tui/src/app.rs:1814 | state/status/action text | Collapsed {summary} • Ctrl-O e expand • Enter sends full text
crates/oino-tui/src/app.rs:1844 | state/status/action text | Press Ctrl-C again to quit • Esc stops a running response
crates/oino-tui/src/app.rs:1859 | state/status/action text | Closed focused extension surface slot
crates/oino-tui/src/app.rs:1899 | state/status/action text | Closed focused extension surface slot
crates/oino-tui/src/app.rs:1903 | state/status/action text | Stopping response…
crates/oino-tui/src/app.rs:1906 | state/status/action text | Esc ignored • press Ctrl-C twice to quit
crates/oino-tui/src/app.rs:1938 | state/status/action text | ctrl-o
crates/oino-tui/src/app.rs:1945 | state/status/action text | {} prefix active • press next key or Esc cancel
crates/oino-tui/src/app.rs:1950 | state/status/action text |  
crates/oino-tui/src/app.rs:1966 | state/status/action text | {} extension prefix active • press next key or Esc cancel
crates/oino-tui/src/app.rs:1971 | state/status/action text |  
crates/oino-tui/src/app.rs:1983 | state/status/action text | Unknown key chord
crates/oino-tui/src/app.rs:1998 | state/status/action text | {} extension prefix active • press next key or Esc cancel
crates/oino-tui/src/app.rs:2003 | state/status/action text |  
crates/oino-tui/src/app.rs:2126 | state/status/action text | Transcript focus
crates/oino-tui/src/app.rs:2136 | state/status/action text | \n
crates/oino-tui/src/app.rs:2143 | state/status/action text | Moved current input to Draft
crates/oino-tui/src/app.rs:2145 | state/status/action text | No input to draft
crates/oino-tui/src/app.rs:2237 | state/status/action text | Press quit again to exit • Esc stops a running response
crates/oino-tui/src/app.rs:2244 | state/status/action text | No input to queue
crates/oino-tui/src/app.rs:2248 | state/status/action text | Queued {}
crates/oino-tui/src/app.rs:2254 | state/status/action text | Expanded pasted block
crates/oino-tui/src/app.rs:2258 | state/status/action text | s
crates/oino-tui/src/app.rs:2259 | state/status/action text | Expanded {count} prompt template{plural}
crates/oino-tui/src/app.rs:2262 | state/status/action text | No collapsed paste block or prompt reference to expand
crates/oino-tui/src/app.rs:2265 | state/status/action text | Incomplete resource reference `{token}`
crates/oino-tui/src/app.rs:2269 | state/status/action text | Unknown prompt `/prompt:{name}`
crates/oino-tui/src/app.rs:2323 | state/status/action text | Stopping response…
crates/oino-tui/src/app.rs:2326 | state/status/action text | Esc ignored • press Ctrl-C twice to quit
crates/oino-tui/src/app.rs:2353 | state/status/action text | Help search active
crates/oino-tui/src/app.rs:2402 | state/status/action text | Help search cleared
crates/oino-tui/src/app.rs:2445 | state/status/action text | Moved current input to Draft
crates/oino-tui/src/app.rs:2447 | state/status/action text | No input to draft
crates/oino-tui/src/app.rs:2454 | state/status/action text | Press y to confirm deletion • n/Esc cancel
crates/oino-tui/src/app.rs:2456 | state/status/action text | Nothing selected to delete
crates/oino-tui/src/app.rs:2462 | state/status/action text | Nothing selected to load
crates/oino-tui/src/app.rs:2466 | state/status/action text | Moved current input to Draft and loaded selection
crates/oino-tui/src/app.rs:2468 | state/status/action text | Loaded selection into input
crates/oino-tui/src/app.rs:2500 | state/status/action text | Nothing selected to delete
crates/oino-tui/src/app.rs:2501 | state/status/action text | Deleted {}
crates/oino-tui/src/app.rs:2506 | state/status/action text | Delete canceled
crates/oino-tui/src/app.rs:2537 | state/status/action text | Session search active
crates/oino-tui/src/app.rs:2542 | state/status/action text | Loading sessions…
crates/oino-tui/src/app.rs:2556 | state/status/action text | Session search cleared
crates/oino-tui/src/app.rs:2583 | state/status/action text | Extension uninstall canceled
crates/oino-tui/src/app.rs:2611 | state/status/action text | Extension search active
crates/oino-tui/src/app.rs:2618 | state/status/action text | Extension snapshot is already loaded
crates/oino-tui/src/app.rs:2629 | state/status/action text | Extension install canceled
crates/oino-tui/src/app.rs:2648 | state/status/action text | Extension search cleared
crates/oino-tui/src/app.rs:2703 | state/status/action text | Extension uninstall canceled
crates/oino-tui/src/app.rs:2764 | state/status/action text | Extensions {} tab • {} items
crates/oino-tui/src/app.rs:2774 | state/status/action text | Extensions {} tab • {} items
crates/oino-tui/src/app.rs:2784 | state/status/action text | Enter a package path, Git URL, or owner/repo to install
crates/oino-tui/src/app.rs:2789 | state/status/action text | Installing extension package from `{source}`…
crates/oino-tui/src/app.rs:2795 | state/status/action text | Select a package row to uninstall
crates/oino-tui/src/app.rs:2799 | state/status/action text | Uninstall {} package `{}`? Enter/Y confirms • N/Esc cancels
crates/oino-tui/src/app.rs:2812 | state/status/action text | Uninstalling {} package `{}`…
crates/oino-tui/src/app.rs:2824 | state/status/action text | No extension item selected
crates/oino-tui/src/app.rs:2830 | state/status/action text | enabled
crates/oino-tui/src/app.rs:2830 | state/status/action text | disabled
crates/oino-tui/src/app.rs:2832 | state/status/action text | {} {} `{}` {status}
crates/oino-tui/src/app.rs:2849 | state/status/action text | Select a contribution row to prefer as a conflict winner
crates/oino-tui/src/app.rs:2853 | state/status/action text | {} conflict override: `{contribution_id}` now prefers `{entry_key}`
crates/oino-tui/src/app.rs:2865 | state/status/action text | No extension contribution selected
crates/oino-tui/src/app.rs:2869 | state/status/action text | Select a contribution row to clear an override
crates/oino-tui/src/app.rs:2874 | state/status/action text | {} conflict override cleared for `{contribution_id}`
crates/oino-tui/src/app.rs:2916 | state/status/action text | Prompt search active
crates/oino-tui/src/app.rs:2921 | state/status/action text | Skill search active
crates/oino-tui/src/app.rs:2931 | state/status/action text | Reloading resources…
crates/oino-tui/src/app.rs:2952 | state/status/action text | Prompt search cleared
crates/oino-tui/src/app.rs:2973 | state/status/action text | Skill search cleared
crates/oino-tui/src/app.rs:3018 | state/status/action text | Exporting chat…
crates/oino-tui/src/app.rs:3034 | state/status/action text | Focused next extension surface slot
crates/oino-tui/src/app.rs:3036 | state/status/action text | No extension surfaces to focus
crates/oino-tui/src/app.rs:3041 | state/status/action text | Focused previous extension surface slot
crates/oino-tui/src/app.rs:3043 | state/status/action text | No extension surfaces to focus
crates/oino-tui/src/app.rs:3048 | state/status/action text | Activated next extension surface tab
crates/oino-tui/src/app.rs:3050 | state/status/action text | No extension surface tabs to switch
crates/oino-tui/src/app.rs:3055 | state/status/action text | Activated previous extension surface tab
crates/oino-tui/src/app.rs:3057 | state/status/action text | No extension surface tabs to switch
crates/oino-tui/src/app.rs:3062 | state/status/action text | Closed focused extension surface slot
crates/oino-tui/src/app.rs:3064 | state/status/action text | No focused extension surface to close
crates/oino-tui/src/app.rs:3069 | state/status/action text | Toggled extension sidebar slots
crates/oino-tui/src/app.rs:3071 | state/status/action text | No extension sidebar slots registered
crates/oino-tui/src/app.rs:3076 | state/status/action text | Toggled extension main panel slots
crates/oino-tui/src/app.rs:3078 | state/status/action text | No extension main panel slots registered
crates/oino-tui/src/app.rs:3204 | state/status/action text | /skill:{name}
crates/oino-tui/src/app.rs:3206 | state/status/action text |  
crates/oino-tui/src/app.rs:3209 | state/status/action text | \n\n
crates/oino-tui/src/app.rs:3212 | state/status/action text | \n\n
crates/oino-tui/src/app.rs:3232 | state/status/action text | {} 
crates/oino-tui/src/app.rs:3255 | state/status/action text | No prompt selected
crates/oino-tui/src/app.rs:3261 | state/status/action text | Completed {command}
crates/oino-tui/src/app.rs:3267 | state/status/action text | No skill selected
crates/oino-tui/src/app.rs:3273 | state/status/action text | Completed {command}
crates/oino-tui/src/app.rs:3282 | state/status/action text | No saved session selected
crates/oino-tui/src/app.rs:3285 | state/status/action text | Opening session {session_id}…
crates/oino-tui/src/app.rs:3298 | state/status/action text | Model set to {model}
crates/oino-tui/src/app.rs:3303 | state/status/action text | Thinking level set to {}
crates/oino-tui/src/app.rs:3309 | state/status/action text | Collapse mode set to {}
crates/oino-tui/src/app.rs:3313 | state/status/action text | Chat style set to {}
crates/oino-tui/src/app.rs:3325 | state/status/action text | ON
crates/oino-tui/src/app.rs:3325 | state/status/action text | OFF
crates/oino-tui/src/app.rs:3326 | state/status/action text | {} tool `{name}` set {status}
crates/oino-tui/src/app.rs:3350 | state/status/action text | Unknown command `{prompt}`
crates/oino-tui/src/app.rs:3356 | state/status/action text | Steering current response…
crates/oino-tui/src/app.rs:3370 | state/status/action text | Unknown command `{prompt}`
crates/oino-tui/src/app.rs:3383 | state/status/action text | Incomplete resource reference `{token}`
crates/oino-tui/src/app.rs:3396 | state/status/action text | Unknown prompt `/prompt:{name}`
crates/oino-tui/src/app.rs:3411 | state/status/action text | Unknown skill `/skill:{name}`
crates/oino-tui/src/app.rs:3434 | state/status/action text | Already in a blank session
crates/oino-tui/src/app.rs:3437 | state/status/action text | Starting new session…
crates/oino-tui/src/app.rs:3455 | state/status/action text | Reloading resources…
crates/oino-tui/src/app.rs:3492 | state/status/action text | Session title set to {title}
crates/oino-tui/src/app.rs:3499 | state/status/action text | Model set to {identifier}
crates/oino-tui/src/app.rs:3506 | state/status/action text | Thinking level set to {}
crates/oino-tui/src/app.rs:3514 | state/status/action text | Collapse mode set to {}
crates/oino-tui/src/app.rs:3520 | state/status/action text | Chat style set to {}
crates/oino-tui/src/app.rs:3536 | state/status/action text | Started new session {session_id}
crates/oino-tui/src/app.rs:3542 | state/status/action text | Continuing session {session_id}
crates/oino-tui/src/app.rs:3575 | state/status/action text | Send panel: ↑/↓ select • q queue input • Enter load • d draft input • x delete • Esc close
crates/oino-tui/src/app.rs:3585 | state/status/action text | Loading sessions…
crates/oino-tui/src/app.rs:3595 | state/status/action text | Prompts
crates/oino-tui/src/app.rs:3605 | state/status/action text | Skills
crates/oino-tui/src/app.rs:3614 | state/status/action text | Extensions: Tab switch Manage/Registered • / search • i/I install • u/x uninstall • g/p toggles • o/O prefer conflict winner • c/C clear override • Esc close
crates/oino-tui/src/app.rs:3625 | state/status/action text | Inspect: loading full prompt…
crates/oino-tui/src/app.rs:3633 | state/status/action text | Inspect: full prompt
crates/oino-tui/src/app.rs:3653 | state/status/action text | Help
crates/oino-tui/src/app.rs:3660 | state/status/action text | Model Selection: arrows/jk move • Enter apply • Esc back
crates/oino-tui/src/app.rs:3667 | state/status/action text | Thinking Level: arrows/jk move • Enter apply • Esc back
crates/oino-tui/src/app.rs:3674 | state/status/action text | Chat Style: arrows/jk move • Enter apply • Esc back
crates/oino-tui/src/app.rs:3681 | state/status/action text | Tools: arrows/jk move • g global • p/Enter project • Esc back
crates/oino-tui/src/app.rs:3688 | state/status/action text | Keymaps: Enter detail • a add in detail • p preset • Esc back
crates/oino-tui/src/app.rs:3695 | state/status/action text | Settings: arrows/jk move • Enter open • Esc close
crates/oino-tui/src/app.rs:3774 | state/status/action text |  
crates/oino-tui/src/app.rs:3776 | state/status/action text |  
crates/oino-tui/src/app.rs:3778 | state/status/action text |  
crates/oino-tui/src/app.rs:3813 | state/status/action text |  
crates/oino-tui/src/app.rs:3815 | state/status/action text |  
crates/oino-tui/src/app.rs:3817 | state/status/action text |  
crates/oino-tui/src/app.rs:3852 | state/status/action text |  
crates/oino-tui/src/app.rs:3854 | state/status/action text |  
crates/oino-tui/src/app.rs:3856 | state/status/action text |  
crates/oino-tui/src/app.rs:3868 | state/status/action text | {} {} {} {}
crates/oino-tui/src/app.rs:3875 | state/status/action text | {} {} {} {}
crates/oino-tui/src/app.rs:3882 | state/status/action text | {} {} {} {}
crates/oino-tui/src/app.rs:3889 | state/status/action text | Session search active
crates/oino-tui/src/app.rs:3891 | state/status/action text | Searching sessions for `{query}`
crates/oino-tui/src/app.rs:3897 | state/status/action text | Prompt search active
crates/oino-tui/src/app.rs:3899 | state/status/action text | Searching prompts for `{query}`
crates/oino-tui/src/app.rs:3905 | state/status/action text | Extension search active
crates/oino-tui/src/app.rs:3907 | state/status/action text | Searching extensions for `{query}`
crates/oino-tui/src/app.rs:3913 | state/status/action text | <package path, Git URL, or owner/repo>
crates/oino-tui/src/app.rs:3918 | state/status/action text | Install {} extension package from {input} • Enter confirms • Esc cancels
crates/oino-tui/src/app.rs:3925 | state/status/action text | Skill search active
crates/oino-tui/src/app.rs:3927 | state/status/action text | Searching skills for `{query}`
crates/oino-tui/src/app.rs:3933 | state/status/action text | Help search active
crates/oino-tui/src/app.rs:3935 | state/status/action text | Searching help for `{query}`
crates/oino-tui/src/app.rs:4003 | state/status/action text | /prompt:
crates/oino-tui/src/app.rs:4007 | state/status/action text | /skill:
crates/oino-tui/src/app.rs:4048 | state/status/action text | \n
crates/oino-tui/src/app.rs:4063 | state/status/action text | \n\n
crates/oino-tui/src/app.rs:4066 | state/status/action text | Use the following Oino resources for this request.
crates/oino-tui/src/app.rs:4068 | state/status/action text | \n\n# Included Prompt Templates
crates/oino-tui/src/app.rs:4070 | state/status/action text | \n\n
crates/oino-tui/src/app.rs:4072 | state/status/action text | Prompt
crates/oino-tui/src/app.rs:4080 | state/status/action text | \n\n# Included Skills
crates/oino-tui/src/app.rs:4082 | state/status/action text | \n\n
crates/oino-tui/src/app.rs:4084 | state/status/action text | Skill
crates/oino-tui/src/app.rs:4092 | state/status/action text | \n\n# User Request\n\n
crates/oino-tui/src/app.rs:4100 | state/status/action text | ## Included {kind}: `{name}`\nSource: `{source}`\n\n{}
crates/oino-tui/src/app.rs:4106 | state/status/action text | `
crates/oino-tui/src/app.rs:4107 | state/status/action text | {fence}markdown\n{}\n{fence}
crates/oino-tui/src/app.rs:4126 | state/status/action text | No Oino resources included
crates/oino-tui/src/app.rs:4127 | state/status/action text | Included {prompts} prompt resource(s)
crates/oino-tui/src/app.rs:4128 | state/status/action text | Included {skills} skill resource(s)
crates/oino-tui/src/app.rs:4130 | state/status/action text | Included {prompts} prompt resource(s) and {skills} skill resource(s)
crates/oino-tui/src/app.rs:4140 | state/status/action text | openrouter
crates/oino-tui/src/app.rs:4140 | state/status/action text | OpenRouter
crates/oino-tui/src/app.rs:4141 | state/status/action text | openai
crates/oino-tui/src/app.rs:4141 | state/status/action text | OpenAI
crates/oino-tui/src/app.rs:4142 | state/status/action text | model
crates/oino-tui/src/app.rs:4146 | state/status/action text | {}{}
crates/oino-tui/src/app.rs:4147 | state/status/action text | model
crates/oino-tui/src/app.rs:4162 | state/status/action text | `{summary}`
crates/oino-tui/src/app.rs:4164 | state/status/action text | `{}…`
crates/oino-tui/src/keymap.rs:10 | keymap labels / descriptions | snake_case
crates/oino-tui/src/keymap.rs:21 | keymap labels / descriptions | Chord
crates/oino-tui/src/keymap.rs:22 | keymap labels / descriptions | Combination
crates/oino-tui/src/keymap.rs:42 | keymap labels / descriptions | Chord
crates/oino-tui/src/keymap.rs:43 | keymap labels / descriptions | Combination
crates/oino-tui/src/keymap.rs:81 | keymap labels / descriptions | Common
crates/oino-tui/src/keymap.rs:82 | keymap labels / descriptions | Global
crates/oino-tui/src/keymap.rs:83 | keymap labels / descriptions | Composer
crates/oino-tui/src/keymap.rs:84 | keymap labels / descriptions | Suggestions
crates/oino-tui/src/keymap.rs:85 | keymap labels / descriptions | Transcript
crates/oino-tui/src/keymap.rs:86 | keymap labels / descriptions | Help
crates/oino-tui/src/keymap.rs:87 | keymap labels / descriptions | Help Search
crates/oino-tui/src/keymap.rs:88 | keymap labels / descriptions | Send Panel
crates/oino-tui/src/keymap.rs:89 | keymap labels / descriptions | Send Confirm
crates/oino-tui/src/keymap.rs:90 | keymap labels / descriptions | Sessions
crates/oino-tui/src/keymap.rs:91 | keymap labels / descriptions | Search Input
crates/oino-tui/src/keymap.rs:92 | keymap labels / descriptions | Resource Browser
crates/oino-tui/src/keymap.rs:93 | keymap labels / descriptions | Inspect
crates/oino-tui/src/keymap.rs:94 | keymap labels / descriptions | Settings
crates/oino-tui/src/keymap.rs:95 | keymap labels / descriptions | Settings Tools
crates/oino-tui/src/keymap.rs:96 | keymap labels / descriptions | Keymaps
crates/oino-tui/src/keymap.rs:97 | keymap labels / descriptions | Keymap Detail
crates/oino-tui/src/keymap.rs:98 | keymap labels / descriptions | Shortcut Type
crates/oino-tui/src/keymap.rs:99 | keymap labels / descriptions | Keymap Preset
crates/oino-tui/src/keymap.rs:100 | keymap labels / descriptions | Preset Confirm
crates/oino-tui/src/keymap.rs:106 | keymap labels / descriptions | snake_case
crates/oino-tui/src/keymap.rs:232 | keymap labels / descriptions | all key actions have metadata
crates/oino-tui/src/keymap.rs:239 | keymap labels / descriptions | common.close
crates/oino-tui/src/keymap.rs:240 | keymap labels / descriptions | common.back
crates/oino-tui/src/keymap.rs:241 | keymap labels / descriptions | common.up
crates/oino-tui/src/keymap.rs:242 | keymap labels / descriptions | common.down
crates/oino-tui/src/keymap.rs:243 | keymap labels / descriptions | common.page_up
crates/oino-tui/src/keymap.rs:244 | keymap labels / descriptions | common.page_down
crates/oino-tui/src/keymap.rs:245 | keymap labels / descriptions | common.top
crates/oino-tui/src/keymap.rs:246 | keymap labels / descriptions | common.bottom
crates/oino-tui/src/keymap.rs:247 | keymap labels / descriptions | common.confirm
crates/oino-tui/src/keymap.rs:248 | keymap labels / descriptions | common.search
crates/oino-tui/src/keymap.rs:249 | keymap labels / descriptions | common.refresh
crates/oino-tui/src/keymap.rs:250 | keymap labels / descriptions | common.backspace
crates/oino-tui/src/keymap.rs:251 | keymap labels / descriptions | common.next
crates/oino-tui/src/keymap.rs:252 | keymap labels / descriptions | common.previous
crates/oino-tui/src/keymap.rs:253 | keymap labels / descriptions | app.quit
crates/oino-tui/src/keymap.rs:254 | keymap labels / descriptions | help.open
crates/oino-tui/src/keymap.rs:255 | keymap labels / descriptions | settings.open
crates/oino-tui/src/keymap.rs:256 | keymap labels / descriptions | send_panel.open
crates/oino-tui/src/keymap.rs:257 | keymap labels / descriptions | transcript.focus
crates/oino-tui/src/keymap.rs:258 | keymap labels / descriptions | composer.expand_reference
crates/oino-tui/src/keymap.rs:259 | keymap labels / descriptions | composer.submit
crates/oino-tui/src/keymap.rs:260 | keymap labels / descriptions | composer.newline
crates/oino-tui/src/keymap.rs:261 | keymap labels / descriptions | composer.queue_prompt
crates/oino-tui/src/keymap.rs:262 | keymap labels / descriptions | composer.draft_prompt
crates/oino-tui/src/keymap.rs:263 | keymap labels / descriptions | suggestions.close
crates/oino-tui/src/keymap.rs:264 | keymap labels / descriptions | suggestions.up
crates/oino-tui/src/keymap.rs:265 | keymap labels / descriptions | suggestions.down
crates/oino-tui/src/keymap.rs:266 | keymap labels / descriptions | suggestions.accept
crates/oino-tui/src/keymap.rs:267 | keymap labels / descriptions | suggestions.confirm
crates/oino-tui/src/keymap.rs:268 | keymap labels / descriptions | transcript.unfocus
crates/oino-tui/src/keymap.rs:269 | keymap labels / descriptions | transcript.page_up
crates/oino-tui/src/keymap.rs:270 | keymap labels / descriptions | transcript.page_down
crates/oino-tui/src/keymap.rs:271 | keymap labels / descriptions | transcript.line_up
crates/oino-tui/src/keymap.rs:272 | keymap labels / descriptions | transcript.line_down
crates/oino-tui/src/keymap.rs:273 | keymap labels / descriptions | transcript.top
crates/oino-tui/src/keymap.rs:274 | keymap labels / descriptions | transcript.bottom
crates/oino-tui/src/keymap.rs:275 | keymap labels / descriptions | help.close
crates/oino-tui/src/keymap.rs:276 | keymap labels / descriptions | help.search
crates/oino-tui/src/keymap.rs:277 | keymap labels / descriptions | help.up
crates/oino-tui/src/keymap.rs:278 | keymap labels / descriptions | help.down
crates/oino-tui/src/keymap.rs:279 | keymap labels / descriptions | help.page_up
crates/oino-tui/src/keymap.rs:280 | keymap labels / descriptions | help.page_down
crates/oino-tui/src/keymap.rs:281 | keymap labels / descriptions | help.top
crates/oino-tui/src/keymap.rs:282 | keymap labels / descriptions | help.bottom
crates/oino-tui/src/keymap.rs:283 | keymap labels / descriptions | search.close
crates/oino-tui/src/keymap.rs:284 | keymap labels / descriptions | search.accept
crates/oino-tui/src/keymap.rs:285 | keymap labels / descriptions | search.backspace
crates/oino-tui/src/keymap.rs:286 | keymap labels / descriptions | search.up
crates/oino-tui/src/keymap.rs:287 | keymap labels / descriptions | search.down
crates/oino-tui/src/keymap.rs:288 | keymap labels / descriptions | search.page_up
crates/oino-tui/src/keymap.rs:289 | keymap labels / descriptions | search.page_down
crates/oino-tui/src/keymap.rs:290 | keymap labels / descriptions | search.top
crates/oino-tui/src/keymap.rs:291 | keymap labels / descriptions | search.bottom
crates/oino-tui/src/keymap.rs:292 | keymap labels / descriptions | send_panel.close
crates/oino-tui/src/keymap.rs:293 | keymap labels / descriptions | send_panel.up
crates/oino-tui/src/keymap.rs:294 | keymap labels / descriptions | send_panel.down
crates/oino-tui/src/keymap.rs:295 | keymap labels / descriptions | send_panel.queue
crates/oino-tui/src/keymap.rs:296 | keymap labels / descriptions | send_panel.draft
crates/oino-tui/src/keymap.rs:297 | keymap labels / descriptions | send_panel.delete
crates/oino-tui/src/keymap.rs:298 | keymap labels / descriptions | send_panel.load
crates/oino-tui/src/keymap.rs:299 | keymap labels / descriptions | confirm.yes
crates/oino-tui/src/keymap.rs:300 | keymap labels / descriptions | confirm.no
crates/oino-tui/src/keymap.rs:301 | keymap labels / descriptions | sessions.close
crates/oino-tui/src/keymap.rs:302 | keymap labels / descriptions | sessions.up
crates/oino-tui/src/keymap.rs:303 | keymap labels / descriptions | sessions.down
crates/oino-tui/src/keymap.rs:304 | keymap labels / descriptions | sessions.search
crates/oino-tui/src/keymap.rs:305 | keymap labels / descriptions | sessions.refresh
crates/oino-tui/src/keymap.rs:306 | keymap labels / descriptions | sessions.open
crates/oino-tui/src/keymap.rs:307 | keymap labels / descriptions | resources.close
crates/oino-tui/src/keymap.rs:308 | keymap labels / descriptions | resources.up
crates/oino-tui/src/keymap.rs:309 | keymap labels / descriptions | resources.down
crates/oino-tui/src/keymap.rs:310 | keymap labels / descriptions | resources.search
crates/oino-tui/src/keymap.rs:311 | keymap labels / descriptions | resources.refresh
crates/oino-tui/src/keymap.rs:312 | keymap labels / descriptions | resources.complete
crates/oino-tui/src/keymap.rs:313 | keymap labels / descriptions | inspect.close
crates/oino-tui/src/keymap.rs:314 | keymap labels / descriptions | inspect.up
crates/oino-tui/src/keymap.rs:315 | keymap labels / descriptions | inspect.down
crates/oino-tui/src/keymap.rs:316 | keymap labels / descriptions | inspect.page_up
crates/oino-tui/src/keymap.rs:317 | keymap labels / descriptions | inspect.page_down
crates/oino-tui/src/keymap.rs:318 | keymap labels / descriptions | inspect.top
crates/oino-tui/src/keymap.rs:319 | keymap labels / descriptions | inspect.export_html
crates/oino-tui/src/keymap.rs:320 | keymap labels / descriptions | settings.close
crates/oino-tui/src/keymap.rs:321 | keymap labels / descriptions | settings.back
crates/oino-tui/src/keymap.rs:322 | keymap labels / descriptions | settings.open_page
crates/oino-tui/src/keymap.rs:323 | keymap labels / descriptions | settings.up
crates/oino-tui/src/keymap.rs:324 | keymap labels / descriptions | settings.down
crates/oino-tui/src/keymap.rs:325 | keymap labels / descriptions | settings.next
crates/oino-tui/src/keymap.rs:326 | keymap labels / descriptions | settings.previous
crates/oino-tui/src/keymap.rs:327 | keymap labels / descriptions | settings.apply
crates/oino-tui/src/keymap.rs:328 | keymap labels / descriptions | settings.search
crates/oino-tui/src/keymap.rs:329 | keymap labels / descriptions | settings.tools.toggle_global
crates/oino-tui/src/keymap.rs:330 | keymap labels / descriptions | settings.tools.toggle_project
crates/oino-tui/src/keymap.rs:331 | keymap labels / descriptions | settings.keymaps.edit_chord_key
crates/oino-tui/src/keymap.rs:332 | keymap labels / descriptions | settings.keymaps.add_shortcut
crates/oino-tui/src/keymap.rs:333 | keymap labels / descriptions | settings.keymaps.remove_shortcut
crates/oino-tui/src/keymap.rs:334 | keymap labels / descriptions | settings.keymaps.clear_shortcuts
crates/oino-tui/src/keymap.rs:335 | keymap labels / descriptions | settings.keymaps.reset_action
crates/oino-tui/src/keymap.rs:336 | keymap labels / descriptions | settings.keymaps.select_preset
crates/oino-tui/src/keymap.rs:337 | keymap labels / descriptions | extensions.surface.focus_next
crates/oino-tui/src/keymap.rs:338 | keymap labels / descriptions | extensions.surface.focus_previous
crates/oino-tui/src/keymap.rs:339 | keymap labels / descriptions | extensions.surface.tab_next
crates/oino-tui/src/keymap.rs:340 | keymap labels / descriptions | extensions.surface.tab_previous
crates/oino-tui/src/keymap.rs:341 | keymap labels / descriptions | extensions.surface.close
crates/oino-tui/src/keymap.rs:342 | keymap labels / descriptions | extensions.sidebar.toggle
crates/oino-tui/src/keymap.rs:343 | keymap labels / descriptions | extensions.main_panel.toggle
crates/oino-tui/src/keymap.rs:358 | keymap labels / descriptions | Close / Cancel
crates/oino-tui/src/keymap.rs:359 | keymap labels / descriptions | close the current overlay, cancel search, or return from transient focus
crates/oino-tui/src/keymap.rs:364 | keymap labels / descriptions | Back
crates/oino-tui/src/keymap.rs:365 | keymap labels / descriptions | return to the previous page inside an overlay
crates/oino-tui/src/keymap.rs:370 | keymap labels / descriptions | Move Up
crates/oino-tui/src/keymap.rs:371 | keymap labels / descriptions | move the active list or document up
crates/oino-tui/src/keymap.rs:376 | keymap labels / descriptions | Move Down
crates/oino-tui/src/keymap.rs:377 | keymap labels / descriptions | move the active list or document down
crates/oino-tui/src/keymap.rs:382 | keymap labels / descriptions | Page Up
crates/oino-tui/src/keymap.rs:383 | keymap labels / descriptions | page the active list or document up
crates/oino-tui/src/keymap.rs:388 | keymap labels / descriptions | Page Down
crates/oino-tui/src/keymap.rs:389 | keymap labels / descriptions | page the active list or document down
crates/oino-tui/src/keymap.rs:394 | keymap labels / descriptions | Jump Top
crates/oino-tui/src/keymap.rs:395 | keymap labels / descriptions | jump the active list or document to top
crates/oino-tui/src/keymap.rs:400 | keymap labels / descriptions | Jump Bottom
crates/oino-tui/src/keymap.rs:401 | keymap labels / descriptions | jump the active list or document to bottom
crates/oino-tui/src/keymap.rs:406 | keymap labels / descriptions | Confirm / Open
crates/oino-tui/src/keymap.rs:407 | keymap labels / descriptions | confirm the active selection
crates/oino-tui/src/keymap.rs:412 | keymap labels / descriptions | Search
crates/oino-tui/src/keymap.rs:413 | keymap labels / descriptions | start search in the active overlay
crates/oino-tui/src/keymap.rs:418 | keymap labels / descriptions | Refresh
crates/oino-tui/src/keymap.rs:419 | keymap labels / descriptions | refresh the active browser
crates/oino-tui/src/keymap.rs:424 | keymap labels / descriptions | Backspace
crates/oino-tui/src/keymap.rs:425 | keymap labels / descriptions | delete one character in active search input
crates/oino-tui/src/keymap.rs:430 | keymap labels / descriptions | Next
crates/oino-tui/src/keymap.rs:431 | keymap labels / descriptions | move to the next focusable/settings item
crates/oino-tui/src/keymap.rs:436 | keymap labels / descriptions | Previous
crates/oino-tui/src/keymap.rs:437 | keymap labels / descriptions | move to the previous focusable/settings item
crates/oino-tui/src/keymap.rs:442 | keymap labels / descriptions | Quit
crates/oino-tui/src/keymap.rs:443 | keymap labels / descriptions | quit Oino after confirmation
crates/oino-tui/src/keymap.rs:448 | keymap labels / descriptions | Open Help
crates/oino-tui/src/keymap.rs:449 | keymap labels / descriptions | open keyboard and command help
crates/oino-tui/src/keymap.rs:454 | keymap labels / descriptions | Open Settings
crates/oino-tui/src/keymap.rs:455 | keymap labels / descriptions | open settings pages
crates/oino-tui/src/keymap.rs:460 | keymap labels / descriptions | Open Send Panel
crates/oino-tui/src/keymap.rs:461 | keymap labels / descriptions | open steering, queue, and draft panel
crates/oino-tui/src/keymap.rs:466 | keymap labels / descriptions | Focus Transcript
crates/oino-tui/src/keymap.rs:467 | keymap labels / descriptions | move focus from composer to transcript
crates/oino-tui/src/keymap.rs:472 | keymap labels / descriptions | Expand Reference
crates/oino-tui/src/keymap.rs:473 | keymap labels / descriptions | expand a collapsed paste block or prompt reference
crates/oino-tui/src/keymap.rs:478 | keymap labels / descriptions | Submit Composer
crates/oino-tui/src/keymap.rs:479 | keymap labels / descriptions | submit current input
crates/oino-tui/src/keymap.rs:484 | keymap labels / descriptions | Insert Newline
crates/oino-tui/src/keymap.rs:485 | keymap labels / descriptions | insert a composer newline
crates/oino-tui/src/keymap.rs:490 | keymap labels / descriptions | Queue Composer
crates/oino-tui/src/keymap.rs:491 | keymap labels / descriptions | queue the current composer input for the next turn
crates/oino-tui/src/keymap.rs:496 | keymap labels / descriptions | Draft Composer
crates/oino-tui/src/keymap.rs:497 | keymap labels / descriptions | move the current composer input to Draft
crates/oino-tui/src/keymap.rs:502 | keymap labels / descriptions | Close Suggestions
crates/oino-tui/src/keymap.rs:503 | keymap labels / descriptions | dismiss command suggestions
crates/oino-tui/src/keymap.rs:508 | keymap labels / descriptions | Suggestion Up
crates/oino-tui/src/keymap.rs:509 | keymap labels / descriptions | move suggestion selection up
crates/oino-tui/src/keymap.rs:514 | keymap labels / descriptions | Suggestion Down
crates/oino-tui/src/keymap.rs:515 | keymap labels / descriptions | move suggestion selection down
crates/oino-tui/src/keymap.rs:520 | keymap labels / descriptions | Accept Suggestion
crates/oino-tui/src/keymap.rs:521 | keymap labels / descriptions | accept suggestion without submitting
crates/oino-tui/src/keymap.rs:526 | keymap labels / descriptions | Confirm Suggestion
crates/oino-tui/src/keymap.rs:527 | keymap labels / descriptions | accept suggestion and submit when ready
crates/oino-tui/src/keymap.rs:532 | keymap labels / descriptions | Return to Composer
crates/oino-tui/src/keymap.rs:533 | keymap labels / descriptions | leave transcript focus
crates/oino-tui/src/keymap.rs:538 | keymap labels / descriptions | Transcript Page Up
crates/oino-tui/src/keymap.rs:539 | keymap labels / descriptions | scroll transcript up by page
crates/oino-tui/src/keymap.rs:544 | keymap labels / descriptions | Transcript Page Down
crates/oino-tui/src/keymap.rs:545 | keymap labels / descriptions | scroll transcript down by page
crates/oino-tui/src/keymap.rs:550 | keymap labels / descriptions | Transcript Line Up
crates/oino-tui/src/keymap.rs:551 | keymap labels / descriptions | scroll transcript up by line
crates/oino-tui/src/keymap.rs:556 | keymap labels / descriptions | Transcript Line Down
crates/oino-tui/src/keymap.rs:557 | keymap labels / descriptions | scroll transcript down by line
crates/oino-tui/src/keymap.rs:562 | keymap labels / descriptions | Transcript Top
crates/oino-tui/src/keymap.rs:563 | keymap labels / descriptions | jump transcript to top
crates/oino-tui/src/keymap.rs:568 | keymap labels / descriptions | Transcript Bottom
crates/oino-tui/src/keymap.rs:569 | keymap labels / descriptions | jump transcript to bottom
crates/oino-tui/src/keymap.rs:574 | keymap labels / descriptions | Close Help
crates/oino-tui/src/keymap.rs:575 | keymap labels / descriptions | close help overlay
crates/oino-tui/src/keymap.rs:580 | keymap labels / descriptions | Search Help
crates/oino-tui/src/keymap.rs:581 | keymap labels / descriptions | start help fuzzy search
crates/oino-tui/src/keymap.rs:586 | keymap labels / descriptions | Help Up
crates/oino-tui/src/keymap.rs:587 | keymap labels / descriptions | scroll help up
crates/oino-tui/src/keymap.rs:592 | keymap labels / descriptions | Help Down
crates/oino-tui/src/keymap.rs:593 | keymap labels / descriptions | scroll help down
crates/oino-tui/src/keymap.rs:598 | keymap labels / descriptions | Help Page Up
crates/oino-tui/src/keymap.rs:599 | keymap labels / descriptions | page help up
crates/oino-tui/src/keymap.rs:604 | keymap labels / descriptions | Help Page Down
crates/oino-tui/src/keymap.rs:605 | keymap labels / descriptions | page help down
crates/oino-tui/src/keymap.rs:610 | keymap labels / descriptions | Help Top
crates/oino-tui/src/keymap.rs:611 | keymap labels / descriptions | jump help to top
crates/oino-tui/src/keymap.rs:616 | keymap labels / descriptions | Help Bottom
crates/oino-tui/src/keymap.rs:617 | keymap labels / descriptions | jump help to bottom
crates/oino-tui/src/keymap.rs:622 | keymap labels / descriptions | Clear Search
crates/oino-tui/src/keymap.rs:623 | keymap labels / descriptions | close or clear active search input
crates/oino-tui/src/keymap.rs:628 | keymap labels / descriptions | Accept Search
crates/oino-tui/src/keymap.rs:629 | keymap labels / descriptions | keep active search results
crates/oino-tui/src/keymap.rs:634 | keymap labels / descriptions | Search Backspace
crates/oino-tui/src/keymap.rs:635 | keymap labels / descriptions | delete one search character
crates/oino-tui/src/keymap.rs:640 | keymap labels / descriptions | Search Up
crates/oino-tui/src/keymap.rs:641 | keymap labels / descriptions | move search selection up
crates/oino-tui/src/keymap.rs:646 | keymap labels / descriptions | Search Down
crates/oino-tui/src/keymap.rs:647 | keymap labels / descriptions | move search selection down
crates/oino-tui/src/keymap.rs:652 | keymap labels / descriptions | Search Page Up
crates/oino-tui/src/keymap.rs:653 | keymap labels / descriptions | page search results up
crates/oino-tui/src/keymap.rs:658 | keymap labels / descriptions | Search Page Down
crates/oino-tui/src/keymap.rs:659 | keymap labels / descriptions | page search results down
crates/oino-tui/src/keymap.rs:664 | keymap labels / descriptions | Search Top
crates/oino-tui/src/keymap.rs:665 | keymap labels / descriptions | jump search results to top
crates/oino-tui/src/keymap.rs:670 | keymap labels / descriptions | Search Bottom
crates/oino-tui/src/keymap.rs:671 | keymap labels / descriptions | jump search results to bottom
crates/oino-tui/src/keymap.rs:676 | keymap labels / descriptions | Close Send Panel
crates/oino-tui/src/keymap.rs:677 | keymap labels / descriptions | close send panel
crates/oino-tui/src/keymap.rs:682 | keymap labels / descriptions | Send Panel Up
crates/oino-tui/src/keymap.rs:683 | keymap labels / descriptions | move send panel selection up
crates/oino-tui/src/keymap.rs:688 | keymap labels / descriptions | Send Panel Down
crates/oino-tui/src/keymap.rs:689 | keymap labels / descriptions | move send panel selection down
crates/oino-tui/src/keymap.rs:694 | keymap labels / descriptions | Queue Prompt
crates/oino-tui/src/keymap.rs:695 | keymap labels / descriptions | queue current input for the next turn
crates/oino-tui/src/keymap.rs:700 | keymap labels / descriptions | Draft Prompt
crates/oino-tui/src/keymap.rs:701 | keymap labels / descriptions | move current input to draft
crates/oino-tui/src/keymap.rs:706 | keymap labels / descriptions | Delete Panel Item
crates/oino-tui/src/keymap.rs:707 | keymap labels / descriptions | delete selected queued or draft item
crates/oino-tui/src/keymap.rs:712 | keymap labels / descriptions | Load Panel Item
crates/oino-tui/src/keymap.rs:713 | keymap labels / descriptions | load selected panel item into the composer
crates/oino-tui/src/keymap.rs:718 | keymap labels / descriptions | Confirm Yes
crates/oino-tui/src/keymap.rs:719 | keymap labels / descriptions | answer yes in a confirmation
crates/oino-tui/src/keymap.rs:724 | keymap labels / descriptions | Confirm No
crates/oino-tui/src/keymap.rs:725 | keymap labels / descriptions | answer no in a confirmation
crates/oino-tui/src/keymap.rs:730 | keymap labels / descriptions | Close Sessions
crates/oino-tui/src/keymap.rs:731 | keymap labels / descriptions | close sessions browser
crates/oino-tui/src/keymap.rs:736 | keymap labels / descriptions | Sessions Up
crates/oino-tui/src/keymap.rs:737 | keymap labels / descriptions | move session selection up
crates/oino-tui/src/keymap.rs:742 | keymap labels / descriptions | Sessions Down
crates/oino-tui/src/keymap.rs:743 | keymap labels / descriptions | move session selection down
crates/oino-tui/src/keymap.rs:748 | keymap labels / descriptions | Search Sessions
crates/oino-tui/src/keymap.rs:749 | keymap labels / descriptions | start sessions search
crates/oino-tui/src/keymap.rs:754 | keymap labels / descriptions | Refresh Sessions
crates/oino-tui/src/keymap.rs:755 | keymap labels / descriptions | reload saved sessions
crates/oino-tui/src/keymap.rs:760 | keymap labels / descriptions | Open Session
crates/oino-tui/src/keymap.rs:761 | keymap labels / descriptions | open selected session
crates/oino-tui/src/keymap.rs:766 | keymap labels / descriptions | Close Resource Browser
crates/oino-tui/src/keymap.rs:767 | keymap labels / descriptions | close prompts or skills browser
crates/oino-tui/src/keymap.rs:772 | keymap labels / descriptions | Resource Up
crates/oino-tui/src/keymap.rs:773 | keymap labels / descriptions | move resource selection up
crates/oino-tui/src/keymap.rs:778 | keymap labels / descriptions | Resource Down
crates/oino-tui/src/keymap.rs:779 | keymap labels / descriptions | move resource selection down
crates/oino-tui/src/keymap.rs:784 | keymap labels / descriptions | Search Resources
crates/oino-tui/src/keymap.rs:785 | keymap labels / descriptions | start resource search
crates/oino-tui/src/keymap.rs:790 | keymap labels / descriptions | Refresh Resources
crates/oino-tui/src/keymap.rs:791 | keymap labels / descriptions | reload prompts and skills
crates/oino-tui/src/keymap.rs:796 | keymap labels / descriptions | Complete Resource
crates/oino-tui/src/keymap.rs:797 | keymap labels / descriptions | insert selected prompt or skill command
crates/oino-tui/src/keymap.rs:802 | keymap labels / descriptions | Close Inspect
crates/oino-tui/src/keymap.rs:803 | keymap labels / descriptions | close inspect overlay
crates/oino-tui/src/keymap.rs:808 | keymap labels / descriptions | Inspect Up
crates/oino-tui/src/keymap.rs:809 | keymap labels / descriptions | scroll inspect up
crates/oino-tui/src/keymap.rs:814 | keymap labels / descriptions | Inspect Down
crates/oino-tui/src/keymap.rs:815 | keymap labels / descriptions | scroll inspect down
crates/oino-tui/src/keymap.rs:820 | keymap labels / descriptions | Inspect Page Up
crates/oino-tui/src/keymap.rs:821 | keymap labels / descriptions | page inspect up
crates/oino-tui/src/keymap.rs:826 | keymap labels / descriptions | Inspect Page Down
crates/oino-tui/src/keymap.rs:827 | keymap labels / descriptions | page inspect down
crates/oino-tui/src/keymap.rs:832 | keymap labels / descriptions | Inspect Top
crates/oino-tui/src/keymap.rs:833 | keymap labels / descriptions | jump inspect to top
crates/oino-tui/src/keymap.rs:838 | keymap labels / descriptions | Export Chat HTML
crates/oino-tui/src/keymap.rs:839 | keymap labels / descriptions | export chat HTML from inspect
crates/oino-tui/src/keymap.rs:844 | keymap labels / descriptions | Close Settings
crates/oino-tui/src/keymap.rs:845 | keymap labels / descriptions | close settings overlay
crates/oino-tui/src/keymap.rs:850 | keymap labels / descriptions | Settings Back
crates/oino-tui/src/keymap.rs:851 | keymap labels / descriptions | return to settings menu
crates/oino-tui/src/keymap.rs:856 | keymap labels / descriptions | Open Settings Page
crates/oino-tui/src/keymap.rs:857 | keymap labels / descriptions | open selected settings page
crates/oino-tui/src/keymap.rs:862 | keymap labels / descriptions | Settings Up
crates/oino-tui/src/keymap.rs:863 | keymap labels / descriptions | move settings selection up
crates/oino-tui/src/keymap.rs:868 | keymap labels / descriptions | Settings Down
crates/oino-tui/src/keymap.rs:869 | keymap labels / descriptions | move settings selection down
crates/oino-tui/src/keymap.rs:874 | keymap labels / descriptions | Settings Next
crates/oino-tui/src/keymap.rs:875 | keymap labels / descriptions | move to next settings item
crates/oino-tui/src/keymap.rs:880 | keymap labels / descriptions | Settings Previous
crates/oino-tui/src/keymap.rs:881 | keymap labels / descriptions | move to previous settings item
crates/oino-tui/src/keymap.rs:886 | keymap labels / descriptions | Apply Setting
crates/oino-tui/src/keymap.rs:887 | keymap labels / descriptions | apply selected setting
crates/oino-tui/src/keymap.rs:892 | keymap labels / descriptions | Search Settings List
crates/oino-tui/src/keymap.rs:893 | keymap labels / descriptions | start search inside a settings list
crates/oino-tui/src/keymap.rs:898 | keymap labels / descriptions | Toggle Global Tool
crates/oino-tui/src/keymap.rs:899 | keymap labels / descriptions | toggle global tool availability
crates/oino-tui/src/keymap.rs:904 | keymap labels / descriptions | Toggle Project Tool
crates/oino-tui/src/keymap.rs:905 | keymap labels / descriptions | toggle project tool availability
crates/oino-tui/src/keymap.rs:910 | keymap labels / descriptions | Edit Chord Key
crates/oino-tui/src/keymap.rs:911 | keymap labels / descriptions | set the global chord prefix key
crates/oino-tui/src/keymap.rs:916 | keymap labels / descriptions | Add Shortcut
crates/oino-tui/src/keymap.rs:917 | keymap labels / descriptions | add another shortcut for an action
crates/oino-tui/src/keymap.rs:922 | keymap labels / descriptions | Remove Shortcut
crates/oino-tui/src/keymap.rs:923 | keymap labels / descriptions | remove selected shortcut
crates/oino-tui/src/keymap.rs:928 | keymap labels / descriptions | Clear Shortcuts
crates/oino-tui/src/keymap.rs:929 | keymap labels / descriptions | unassign all shortcuts for an action
crates/oino-tui/src/keymap.rs:934 | keymap labels / descriptions | Reset Action
crates/oino-tui/src/keymap.rs:935 | keymap labels / descriptions | reset one action to preset defaults
crates/oino-tui/src/keymap.rs:940 | keymap labels / descriptions | Select Preset
crates/oino-tui/src/keymap.rs:941 | keymap labels / descriptions | reset all keybinds to a preset
crates/oino-tui/src/keymap.rs:946 | keymap labels / descriptions | Focus Next Extension Surface
crates/oino-tui/src/keymap.rs:947 | keymap labels / descriptions | move focus to the next visible extension surface slot
crates/oino-tui/src/keymap.rs:952 | keymap labels / descriptions | Focus Previous Extension Surface
crates/oino-tui/src/keymap.rs:953 | keymap labels / descriptions | move focus to the previous visible extension surface slot
crates/oino-tui/src/keymap.rs:958 | keymap labels / descriptions | Next Extension Tab
crates/oino-tui/src/keymap.rs:959 | keymap labels / descriptions | activate the next extension surface in the focused slot
crates/oino-tui/src/keymap.rs:964 | keymap labels / descriptions | Previous Extension Tab
crates/oino-tui/src/keymap.rs:965 | keymap labels / descriptions | activate the previous extension surface in the focused slot
crates/oino-tui/src/keymap.rs:970 | keymap labels / descriptions | Close Extension Surface
crates/oino-tui/src/keymap.rs:971 | keymap labels / descriptions | hide the focused extension surface slot
crates/oino-tui/src/keymap.rs:976 | keymap labels / descriptions | Toggle Extension Sidebar
crates/oino-tui/src/keymap.rs:977 | keymap labels / descriptions | show or hide extension sidebar slots
crates/oino-tui/src/keymap.rs:982 | keymap labels / descriptions | Toggle Extension Main Panel
crates/oino-tui/src/keymap.rs:983 | keymap labels / descriptions | show or hide extension main panel slots
crates/oino-tui/src/keymap.rs:1105 | keymap labels / descriptions | Ctrl
crates/oino-tui/src/keymap.rs:1108 | keymap labels / descriptions | Alt
crates/oino-tui/src/keymap.rs:1111 | keymap labels / descriptions | Shift
crates/oino-tui/src/keymap.rs:1114 | keymap labels / descriptions | Super
crates/oino-tui/src/keymap.rs:1121 | keymap labels / descriptions | Enter
crates/oino-tui/src/keymap.rs:1122 | keymap labels / descriptions | Esc
crates/oino-tui/src/keymap.rs:1123 | keymap labels / descriptions | Backspace
crates/oino-tui/src/keymap.rs:1124 | keymap labels / descriptions | Delete
crates/oino-tui/src/keymap.rs:1125 | keymap labels / descriptions | Tab
crates/oino-tui/src/keymap.rs:1126 | keymap labels / descriptions | Shift-Tab
crates/oino-tui/src/keymap.rs:1127 | keymap labels / descriptions | Left
crates/oino-tui/src/keymap.rs:1128 | keymap labels / descriptions | Right
crates/oino-tui/src/keymap.rs:1129 | keymap labels / descriptions | Up
crates/oino-tui/src/keymap.rs:1130 | keymap labels / descriptions | Down
crates/oino-tui/src/keymap.rs:1131 | keymap labels / descriptions | Home
crates/oino-tui/src/keymap.rs:1132 | keymap labels / descriptions | End
crates/oino-tui/src/keymap.rs:1133 | keymap labels / descriptions | PgUp
crates/oino-tui/src/keymap.rs:1134 | keymap labels / descriptions | PgDn
crates/oino-tui/src/keymap.rs:1135 | keymap labels / descriptions | Insert
crates/oino-tui/src/keymap.rs:1136 | keymap labels / descriptions | F{n}
crates/oino-tui/src/keymap.rs:1137 | keymap labels / descriptions | Space
crates/oino-tui/src/keymap.rs:1139 | keymap labels / descriptions | -
crates/oino-tui/src/keymap.rs:1149 | keymap labels / descriptions | empty key stroke
crates/oino-tui/src/keymap.rs:1156 | keymap labels / descriptions | ctrl
crates/oino-tui/src/keymap.rs:1156 | keymap labels / descriptions | control
crates/oino-tui/src/keymap.rs:1157 | keymap labels / descriptions | alt
crates/oino-tui/src/keymap.rs:1157 | keymap labels / descriptions | option
crates/oino-tui/src/keymap.rs:1158 | keymap labels / descriptions | shift
crates/oino-tui/src/keymap.rs:1159 | keymap labels / descriptions | cmd
crates/oino-tui/src/keymap.rs:1159 | keymap labels / descriptions | command
crates/oino-tui/src/keymap.rs:1159 | keymap labels / descriptions | super
crates/oino-tui/src/keymap.rs:1159 | keymap labels / descriptions | win
crates/oino-tui/src/keymap.rs:1159 | keymap labels / descriptions | meta
crates/oino-tui/src/keymap.rs:1160 | keymap labels / descriptions | unknown modifier `{unknown}`
crates/oino-tui/src/keymap.rs:1164 | keymap labels / descriptions | missing key
crates/oino-tui/src/keymap.rs:1168 | keymap labels / descriptions | enter
crates/oino-tui/src/keymap.rs:1168 | keymap labels / descriptions | return
crates/oino-tui/src/keymap.rs:1169 | keymap labels / descriptions | esc
crates/oino-tui/src/keymap.rs:1169 | keymap labels / descriptions | escape
crates/oino-tui/src/keymap.rs:1170 | keymap labels / descriptions | backspace
crates/oino-tui/src/keymap.rs:1170 | keymap labels / descriptions | bs
crates/oino-tui/src/keymap.rs:1171 | keymap labels / descriptions | delete
crates/oino-tui/src/keymap.rs:1171 | keymap labels / descriptions | del
crates/oino-tui/src/keymap.rs:1172 | keymap labels / descriptions | tab
crates/oino-tui/src/keymap.rs:1173 | keymap labels / descriptions | backtab
crates/oino-tui/src/keymap.rs:1174 | keymap labels / descriptions | left
crates/oino-tui/src/keymap.rs:1175 | keymap labels / descriptions | right
crates/oino-tui/src/keymap.rs:1176 | keymap labels / descriptions | up
crates/oino-tui/src/keymap.rs:1177 | keymap labels / descriptions | down
crates/oino-tui/src/keymap.rs:1178 | keymap labels / descriptions | home
crates/oino-tui/src/keymap.rs:1179 | keymap labels / descriptions | end
crates/oino-tui/src/keymap.rs:1180 | keymap labels / descriptions | pageup
crates/oino-tui/src/keymap.rs:1180 | keymap labels / descriptions | pgup
crates/oino-tui/src/keymap.rs:1181 | keymap labels / descriptions | pagedown
crates/oino-tui/src/keymap.rs:1181 | keymap labels / descriptions | pgdn
crates/oino-tui/src/keymap.rs:1182 | keymap labels / descriptions | insert
crates/oino-tui/src/keymap.rs:1182 | keymap labels / descriptions | ins
crates/oino-tui/src/keymap.rs:1183 | keymap labels / descriptions | space
crates/oino-tui/src/keymap.rs:1187 | keymap labels / descriptions | invalid function key `{key}`
crates/oino-tui/src/keymap.rs:1194 | keymap labels / descriptions | unknown key `{key}`
crates/oino-tui/src/keymap.rs:1261 | keymap labels / descriptions |  
crates/oino-tui/src/keymap.rs:1273 | keymap labels / descriptions | empty key sequence
crates/oino-tui/src/keymap.rs:1319 | keymap labels / descriptions | default_chord_key
crates/oino-tui/src/keymap.rs:1361 | keymap labels / descriptions | Unassigned
crates/oino-tui/src/keymap.rs:1368 | keymap labels / descriptions | Unassigned
crates/oino-tui/src/keymap.rs:1374 | keymap labels / descriptions | , 
crates/oino-tui/src/keymap.rs:1562 | keymap labels / descriptions | esc
crates/oino-tui/src/keymap.rs:1563 | keymap labels / descriptions | left
crates/oino-tui/src/keymap.rs:1563 | keymap labels / descriptions | backspace
crates/oino-tui/src/keymap.rs:1564 | keymap labels / descriptions | up
crates/oino-tui/src/keymap.rs:1564 | keymap labels / descriptions | k
crates/oino-tui/src/keymap.rs:1564 | keymap labels / descriptions | K
crates/oino-tui/src/keymap.rs:1565 | keymap labels / descriptions | down
crates/oino-tui/src/keymap.rs:1565 | keymap labels / descriptions | j
crates/oino-tui/src/keymap.rs:1565 | keymap labels / descriptions | J
crates/oino-tui/src/keymap.rs:1566 | keymap labels / descriptions | pageup
crates/oino-tui/src/keymap.rs:1567 | keymap labels / descriptions | pagedown
crates/oino-tui/src/keymap.rs:1568 | keymap labels / descriptions | home
crates/oino-tui/src/keymap.rs:1568 | keymap labels / descriptions | ctrl-home
crates/oino-tui/src/keymap.rs:1569 | keymap labels / descriptions | end
crates/oino-tui/src/keymap.rs:1569 | keymap labels / descriptions | ctrl-end
crates/oino-tui/src/keymap.rs:1570 | keymap labels / descriptions | enter
crates/oino-tui/src/keymap.rs:1571 | keymap labels / descriptions | /
crates/oino-tui/src/keymap.rs:1572 | keymap labels / descriptions | r
crates/oino-tui/src/keymap.rs:1572 | keymap labels / descriptions | R
crates/oino-tui/src/keymap.rs:1573 | keymap labels / descriptions | backspace
crates/oino-tui/src/keymap.rs:1574 | keymap labels / descriptions | tab
crates/oino-tui/src/keymap.rs:1575 | keymap labels / descriptions | shift-tab
crates/oino-tui/src/keymap.rs:1576 | keymap labels / descriptions | ctrl-c
crates/oino-tui/src/keymap.rs:1577 | keymap labels / descriptions | h
crates/oino-tui/src/keymap.rs:1578 | keymap labels / descriptions | f1
crates/oino-tui/src/keymap.rs:1579 | keymap labels / descriptions | s
crates/oino-tui/src/keymap.rs:1580 | keymap labels / descriptions | f2
crates/oino-tui/src/keymap.rs:1582 | keymap labels / descriptions | q
crates/oino-tui/src/keymap.rs:1584 | keymap labels / descriptions | f4
crates/oino-tui/src/keymap.rs:1586 | keymap labels / descriptions | t
crates/oino-tui/src/keymap.rs:1588 | keymap labels / descriptions | f3
crates/oino-tui/src/keymap.rs:1590 | keymap labels / descriptions | e
crates/oino-tui/src/keymap.rs:1592 | keymap labels / descriptions | f5
crates/oino-tui/src/keymap.rs:1593 | keymap labels / descriptions | enter
crates/oino-tui/src/keymap.rs:1594 | keymap labels / descriptions | ctrl-j
crates/oino-tui/src/keymap.rs:1594 | keymap labels / descriptions | alt-enter
crates/oino-tui/src/keymap.rs:1594 | keymap labels / descriptions | shift-enter
crates/oino-tui/src/keymap.rs:1595 | keymap labels / descriptions | enter
crates/oino-tui/src/keymap.rs:1596 | keymap labels / descriptions | /
crates/oino-tui/src/keymap.rs:1597 | keymap labels / descriptions | esc
crates/oino-tui/src/keymap.rs:1598 | keymap labels / descriptions | up
crates/oino-tui/src/keymap.rs:1599 | keymap labels / descriptions | down
crates/oino-tui/src/keymap.rs:1600 | keymap labels / descriptions | tab
crates/oino-tui/src/keymap.rs:1601 | keymap labels / descriptions | enter
crates/oino-tui/src/keymap.rs:1602 | keymap labels / descriptions | esc
crates/oino-tui/src/keymap.rs:1603 | keymap labels / descriptions | pageup
crates/oino-tui/src/keymap.rs:1604 | keymap labels / descriptions | pagedown
crates/oino-tui/src/keymap.rs:1605 | keymap labels / descriptions | alt-up
crates/oino-tui/src/keymap.rs:1605 | keymap labels / descriptions | up
crates/oino-tui/src/keymap.rs:1605 | keymap labels / descriptions | k
crates/oino-tui/src/keymap.rs:1605 | keymap labels / descriptions | K
crates/oino-tui/src/keymap.rs:1606 | keymap labels / descriptions | alt-down
crates/oino-tui/src/keymap.rs:1606 | keymap labels / descriptions | down
crates/oino-tui/src/keymap.rs:1606 | keymap labels / descriptions | j
crates/oino-tui/src/keymap.rs:1606 | keymap labels / descriptions | J
crates/oino-tui/src/keymap.rs:1607 | keymap labels / descriptions | ctrl-home
crates/oino-tui/src/keymap.rs:1607 | keymap labels / descriptions | home
crates/oino-tui/src/keymap.rs:1607 | keymap labels / descriptions | g
crates/oino-tui/src/keymap.rs:1608 | keymap labels / descriptions | ctrl-end
crates/oino-tui/src/keymap.rs:1608 | keymap labels / descriptions | end
crates/oino-tui/src/keymap.rs:1608 | keymap labels / descriptions | G
crates/oino-tui/src/keymap.rs:1609 | keymap labels / descriptions | esc
crates/oino-tui/src/keymap.rs:1609 | keymap labels / descriptions | q
crates/oino-tui/src/keymap.rs:1609 | keymap labels / descriptions | Q
crates/oino-tui/src/keymap.rs:1610 | keymap labels / descriptions | /
crates/oino-tui/src/keymap.rs:1611 | keymap labels / descriptions | up
crates/oino-tui/src/keymap.rs:1611 | keymap labels / descriptions | k
crates/oino-tui/src/keymap.rs:1611 | keymap labels / descriptions | K
crates/oino-tui/src/keymap.rs:1612 | keymap labels / descriptions | down
crates/oino-tui/src/keymap.rs:1612 | keymap labels / descriptions | j
crates/oino-tui/src/keymap.rs:1612 | keymap labels / descriptions | J
crates/oino-tui/src/keymap.rs:1613 | keymap labels / descriptions | pageup
crates/oino-tui/src/keymap.rs:1614 | keymap labels / descriptions | pagedown
crates/oino-tui/src/keymap.rs:1615 | keymap labels / descriptions | home
crates/oino-tui/src/keymap.rs:1615 | keymap labels / descriptions | ctrl-home
crates/oino-tui/src/keymap.rs:1616 | keymap labels / descriptions | end
crates/oino-tui/src/keymap.rs:1616 | keymap labels / descriptions | ctrl-end
crates/oino-tui/src/keymap.rs:1617 | keymap labels / descriptions | esc
crates/oino-tui/src/keymap.rs:1618 | keymap labels / descriptions | enter
crates/oino-tui/src/keymap.rs:1618 | keymap labels / descriptions | tab
crates/oino-tui/src/keymap.rs:1619 | keymap labels / descriptions | backspace
crates/oino-tui/src/keymap.rs:1620 | keymap labels / descriptions | up
crates/oino-tui/src/keymap.rs:1621 | keymap labels / descriptions | down
crates/oino-tui/src/keymap.rs:1622 | keymap labels / descriptions | pageup
crates/oino-tui/src/keymap.rs:1623 | keymap labels / descriptions | pagedown
crates/oino-tui/src/keymap.rs:1624 | keymap labels / descriptions | home
crates/oino-tui/src/keymap.rs:1624 | keymap labels / descriptions | ctrl-home
crates/oino-tui/src/keymap.rs:1625 | keymap labels / descriptions | end
crates/oino-tui/src/keymap.rs:1625 | keymap labels / descriptions | ctrl-end
crates/oino-tui/src/keymap.rs:1626 | keymap labels / descriptions | esc
crates/oino-tui/src/keymap.rs:1627 | keymap labels / descriptions | up
crates/oino-tui/src/keymap.rs:1627 | keymap labels / descriptions | k
crates/oino-tui/src/keymap.rs:1627 | keymap labels / descriptions | K
crates/oino-tui/src/keymap.rs:1628 | keymap labels / descriptions | down
crates/oino-tui/src/keymap.rs:1628 | keymap labels / descriptions | j
crates/oino-tui/src/keymap.rs:1628 | keymap labels / descriptions | J
crates/oino-tui/src/keymap.rs:1629 | keymap labels / descriptions | q
crates/oino-tui/src/keymap.rs:1629 | keymap labels / descriptions | Q
crates/oino-tui/src/keymap.rs:1630 | keymap labels / descriptions | d
crates/oino-tui/src/keymap.rs:1630 | keymap labels / descriptions | D
crates/oino-tui/src/keymap.rs:1631 | keymap labels / descriptions | x
crates/oino-tui/src/keymap.rs:1631 | keymap labels / descriptions | X
crates/oino-tui/src/keymap.rs:1632 | keymap labels / descriptions | enter
crates/oino-tui/src/keymap.rs:1633 | keymap labels / descriptions | y
crates/oino-tui/src/keymap.rs:1633 | keymap labels / descriptions | Y
crates/oino-tui/src/keymap.rs:1634 | keymap labels / descriptions | n
crates/oino-tui/src/keymap.rs:1634 | keymap labels / descriptions | N
crates/oino-tui/src/keymap.rs:1634 | keymap labels / descriptions | esc
crates/oino-tui/src/keymap.rs:1635 | keymap labels / descriptions | esc
crates/oino-tui/src/keymap.rs:1636 | keymap labels / descriptions | up
crates/oino-tui/src/keymap.rs:1636 | keymap labels / descriptions | k
crates/oino-tui/src/keymap.rs:1636 | keymap labels / descriptions | K
crates/oino-tui/src/keymap.rs:1637 | keymap labels / descriptions | down
crates/oino-tui/src/keymap.rs:1637 | keymap labels / descriptions | j
crates/oino-tui/src/keymap.rs:1637 | keymap labels / descriptions | J
crates/oino-tui/src/keymap.rs:1638 | keymap labels / descriptions | /
crates/oino-tui/src/keymap.rs:1639 | keymap labels / descriptions | r
crates/oino-tui/src/keymap.rs:1639 | keymap labels / descriptions | R
crates/oino-tui/src/keymap.rs:1640 | keymap labels / descriptions | enter
crates/oino-tui/src/keymap.rs:1641 | keymap labels / descriptions | esc
crates/oino-tui/src/keymap.rs:1642 | keymap labels / descriptions | up
crates/oino-tui/src/keymap.rs:1642 | keymap labels / descriptions | k
crates/oino-tui/src/keymap.rs:1642 | keymap labels / descriptions | K
crates/oino-tui/src/keymap.rs:1643 | keymap labels / descriptions | down
crates/oino-tui/src/keymap.rs:1643 | keymap labels / descriptions | j
crates/oino-tui/src/keymap.rs:1643 | keymap labels / descriptions | J
crates/oino-tui/src/keymap.rs:1644 | keymap labels / descriptions | /
crates/oino-tui/src/keymap.rs:1645 | keymap labels / descriptions | r
crates/oino-tui/src/keymap.rs:1645 | keymap labels / descriptions | R
crates/oino-tui/src/keymap.rs:1646 | keymap labels / descriptions | tab
crates/oino-tui/src/keymap.rs:1646 | keymap labels / descriptions | enter
crates/oino-tui/src/keymap.rs:1647 | keymap labels / descriptions | esc
crates/oino-tui/src/keymap.rs:1647 | keymap labels / descriptions | q
crates/oino-tui/src/keymap.rs:1647 | keymap labels / descriptions | Q
crates/oino-tui/src/keymap.rs:1648 | keymap labels / descriptions | up
crates/oino-tui/src/keymap.rs:1648 | keymap labels / descriptions | k
crates/oino-tui/src/keymap.rs:1648 | keymap labels / descriptions | K
crates/oino-tui/src/keymap.rs:1649 | keymap labels / descriptions | down
crates/oino-tui/src/keymap.rs:1649 | keymap labels / descriptions | j
crates/oino-tui/src/keymap.rs:1649 | keymap labels / descriptions | J
crates/oino-tui/src/keymap.rs:1650 | keymap labels / descriptions | pageup
crates/oino-tui/src/keymap.rs:1651 | keymap labels / descriptions | pagedown
crates/oino-tui/src/keymap.rs:1652 | keymap labels / descriptions | home
crates/oino-tui/src/keymap.rs:1652 | keymap labels / descriptions | ctrl-home
crates/oino-tui/src/keymap.rs:1653 | keymap labels / descriptions | e
crates/oino-tui/src/keymap.rs:1653 | keymap labels / descriptions | E
crates/oino-tui/src/keymap.rs:1654 | keymap labels / descriptions | esc
crates/oino-tui/src/keymap.rs:1655 | keymap labels / descriptions | left
crates/oino-tui/src/keymap.rs:1655 | keymap labels / descriptions | backspace
crates/oino-tui/src/keymap.rs:1656 | keymap labels / descriptions | right
crates/oino-tui/src/keymap.rs:1657 | keymap labels / descriptions | up
crates/oino-tui/src/keymap.rs:1657 | keymap labels / descriptions | k
crates/oino-tui/src/keymap.rs:1658 | keymap labels / descriptions | down
crates/oino-tui/src/keymap.rs:1658 | keymap labels / descriptions | j
crates/oino-tui/src/keymap.rs:1659 | keymap labels / descriptions | tab
crates/oino-tui/src/keymap.rs:1660 | keymap labels / descriptions | shift-tab
crates/oino-tui/src/keymap.rs:1661 | keymap labels / descriptions | enter
crates/oino-tui/src/keymap.rs:1662 | keymap labels / descriptions | /
crates/oino-tui/src/keymap.rs:1663 | keymap labels / descriptions | g
crates/oino-tui/src/keymap.rs:1663 | keymap labels / descriptions | G
crates/oino-tui/src/keymap.rs:1664 | keymap labels / descriptions | p
crates/oino-tui/src/keymap.rs:1664 | keymap labels / descriptions | P
crates/oino-tui/src/keymap.rs:1664 | keymap labels / descriptions | space
crates/oino-tui/src/keymap.rs:1664 | keymap labels / descriptions | enter
crates/oino-tui/src/keymap.rs:1664 | keymap labels / descriptions | right
crates/oino-tui/src/keymap.rs:1665 | keymap labels / descriptions | g
crates/oino-tui/src/keymap.rs:1665 | keymap labels / descriptions | G
crates/oino-tui/src/keymap.rs:1666 | keymap labels / descriptions | a
crates/oino-tui/src/keymap.rs:1666 | keymap labels / descriptions | A
crates/oino-tui/src/keymap.rs:1667 | keymap labels / descriptions | x
crates/oino-tui/src/keymap.rs:1667 | keymap labels / descriptions | X
crates/oino-tui/src/keymap.rs:1668 | keymap labels / descriptions | c
crates/oino-tui/src/keymap.rs:1668 | keymap labels / descriptions | C
crates/oino-tui/src/keymap.rs:1669 | keymap labels / descriptions | r
crates/oino-tui/src/keymap.rs:1669 | keymap labels / descriptions | R
crates/oino-tui/src/keymap.rs:1670 | keymap labels / descriptions | p
crates/oino-tui/src/keymap.rs:1670 | keymap labels / descriptions | P
crates/oino-tui/src/keymap.rs:1671 | keymap labels / descriptions | tab
crates/oino-tui/src/keymap.rs:1673 | keymap labels / descriptions | shift-tab
crates/oino-tui/src/keymap.rs:1675 | keymap labels / descriptions | ]
crates/oino-tui/src/keymap.rs:1676 | keymap labels / descriptions | [
crates/oino-tui/src/keymap.rs:1677 | keymap labels / descriptions | w
crates/oino-tui/src/keymap.rs:1678 | keymap labels / descriptions | b
crates/oino-tui/src/keymap.rs:1679 | keymap labels / descriptions | m
crates/oino-tui/src/keymap.rs:1689 | keymap labels / descriptions | ctrl-o
crates/oino-tui/src/keymap.rs:1691 | keymap labels / descriptions | default chord key parses
crates/oino-tui/src/message.rs:29 | message projection | user
crates/oino-tui/src/message.rs:34 | message projection | assistant
crates/oino-tui/src/message.rs:50 | message projection | user
crates/oino-tui/src/message.rs:69 | message projection | assistant
crates/oino-tui/src/message.rs:93 | message projection | tool:{tool_name}
crates/oino-tui/src/message.rs:105 | message projection | custom:{name}
crates/oino-tui/src/message.rs:107 | message projection | <custom>
crates/oino-tui/src/message.rs:116 | message projection | compaction
crates/oino-tui/src/message.rs:127 | message projection | branch
crates/oino-tui/src/message.rs:160 | message projection | <image:{media_type}>
crates/oino-tui/src/message.rs:178 | message projection | <empty>
crates/oino-tui/src/message.rs:180 | message projection |  
crates/oino-tui/src/message.rs:185 | message projection | \n
crates/oino-tui/src/resource.rs:16 | resource browser/model | /prompt:{}
crates/oino-tui/src/resource.rs:22 | resource browser/model | /{} {}
crates/oino-tui/src/resource.rs:45 | resource browser/model | /skill:{}
crates/oino-tui/src/resource.rs:52 | resource browser/model | Use the `{}` skill.\n\nSkill file: {}\n\n{}
crates/oino-tui/src/resource.rs:57 | resource browser/model | Use the `{}` skill with this user input:\n\n{}\n\nSkill file: {}\n\n{}
crates/oino-tui/src/resource.rs:82 | resource browser/model | $ARGUMENTS
crates/oino-tui/src/resource.rs:82 | resource browser/model | $@
crates/oino-tui/src/resource.rs:86 | resource browser/model | ${index}
crates/oino-tui/src/theme.rs:66 | theme data model | user
crates/oino-tui/src/theme.rs:67 | theme data model | assistant
crates/oino-tui/src/theme.rs:68 | theme data model | tool:
```
