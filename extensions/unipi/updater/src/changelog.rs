//! Keep-a-Changelog parser (unipi `src/changelog.ts`, `src/remote-changelog.ts`).

use unipi_kit::fmt::{compare_versions, is_newer_version};

/// Raw changelog base (`<base>/<ref>/CHANGELOG.md`).
pub(crate) const CHANGELOG_RAW_BASE: &str =
    "https://raw.githubusercontent.com/Neuron-Mr-White/Oino";

/// One version entry (unipi `ChangelogEntry`).
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub(crate) struct Entry {
    /// `0.1.15` or `Unreleased`.
    pub version: String,
    /// `2026-04-30` (empty for Unreleased).
    pub date: String,
    /// Sections in file order: `(Added, [items])`.
    pub sections: Vec<(String, Vec<String>)>,
    /// Rebuilt body for rendering.
    pub body: String,
}

/// `## [x.y.z] — date` / `## [Unreleased]`.
fn version_header(line: &str) -> Option<(String, String)> {
    let rest = line.strip_prefix("## [")?;
    let close = rest.find(']')?;
    let version = rest[..close].trim().to_owned();
    if version.is_empty() {
        return None;
    }
    let tail = rest[close + 1..].trim();
    let date = if tail.is_empty() {
        String::new()
    } else {
        let t = tail.trim_start_matches(['-', '—', '–']);
        if t.len() == tail.len() {
            // Anything after `]` that is not a dash separator does not match unipi's regex.
            return None;
        }
        t.trim().to_owned()
    };
    Some((version, date))
}

/// unipi `parseChangelogContent`.
pub(crate) fn parse(content: &str) -> Vec<Entry> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    let mut entries: Vec<Entry> = Vec::new();
    let mut section: Option<String> = None;
    let mut items: Vec<String> = Vec::new();
    let flush =
        |entries: &mut Vec<Entry>, section: &mut Option<String>, items: &mut Vec<String>| {
            if let (Some(e), Some(s)) = (entries.last_mut(), section.take())
                && !items.is_empty()
            {
                e.sections.push((s, std::mem::take(items)));
            }
            items.clear();
        };
    for line in trimmed.lines() {
        if let Some((version, date)) = version_header(line) {
            flush(&mut entries, &mut section, &mut items);
            entries.push(Entry {
                version,
                date,
                ..Entry::default()
            });
            continue;
        }
        if entries.is_empty() {
            continue;
        }
        if let Some(name) = line.strip_prefix("### ") {
            flush(&mut entries, &mut section, &mut items);
            section = Some(name.trim().to_owned());
            continue;
        }
        let t = line.trim();
        if section.is_some() && (t.starts_with("- ") || t.starts_with("* ")) {
            items.push(t[2..].trim().to_owned());
        } else if !t.is_empty()
            && section.is_some()
            && let Some(last) = items.last_mut()
        {
            last.push(' ');
            last.push_str(t);
        }
    }
    flush(&mut entries, &mut section, &mut items);
    for e in &mut entries {
        let mut lines = Vec::new();
        for (name, list) in &e.sections {
            lines.push(format!("### {name}"));
            lines.extend(list.iter().map(|i| format!("- {i}")));
            lines.push(String::new());
        }
        lines.join("\n").trim().clone_into(&mut e.body);
    }
    entries
}

/// unipi `getNewerVersions`: entries newer than `installed`, newest first; a stable install
/// skips prerelease entries; non-empty `Unreleased` is always kept.
pub(crate) fn newer_versions(entries: &[Entry], installed: &str) -> Vec<Entry> {
    let on_prerelease = installed.contains('-');
    let mut out = Vec::new();
    for e in entries {
        if e.version == "Unreleased" {
            if !e.sections.is_empty() {
                out.push(e.clone());
            }
            continue;
        }
        if !is_newer_version(&e.version, installed) {
            break;
        }
        if !on_prerelease && e.version.contains('-') {
            continue;
        }
        out.push(e.clone());
    }
    out
}

/// Entries up to and including `latest` (Unreleased kept).
pub(crate) fn up_to(entries: Vec<Entry>, latest: &str) -> Vec<Entry> {
    entries
        .into_iter()
        .filter(|e| e.version == "Unreleased" || compare_versions(&e.version, latest).is_le())
        .collect()
}

/// unipi `fallbackRefs`: branch refs to try when the release tag has no changelog.
pub(crate) fn fallback_refs(version: &str) -> Vec<String> {
    let v = version.strip_prefix('v').unwrap_or(version);
    // `3.0.0-alpha.36` → `v3.0.0-alpha`.
    if let Some((core, pre)) = v.split_once('-')
        && core.split('.').count() == 3
        && let Some((id, rest)) = pre.split_once('.')
        && !id.is_empty()
        && !rest.is_empty()
    {
        return vec![format!("v{core}-{id}"), "main".to_owned()];
    }
    vec!["main".to_owned()]
}

/// Markdown for a list of entries (update screen and detail view).
pub(crate) fn render(entries: &[Entry]) -> String {
    entries
        .iter()
        .map(|e| {
            let head = if e.date.is_empty() {
                format!("## {}", e.version)
            } else {
                format!("## {} — {}", e.version, e.date)
            };
            if e.body.is_empty() {
                head
            } else {
                format!("{head}\n\n{}", e.body)
            }
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "# Changelog\n\nIntro text.\n\n## [Unreleased]\n\n### Added\n- Next thing\n\n## [3.0.0] — 2026-05-01\n\n### Added\n- Big feature\n  continued here\n* Star item\n\n### Fixed\n- A bug\n\n## [3.0.0-alpha.2] - 2026-04-20\n\n### Changed\n- Alpha change\n\n## [2.0.0] – 2026-01-01\n\n### Removed\n- Old thing\n";

    #[test]
    fn parses_keep_a_changelog() {
        let e = parse(SAMPLE);
        assert_eq!(e.len(), 4);
        assert_eq!(e[0].version, "Unreleased");
        assert_eq!(e[0].date, "");
        assert_eq!(e[1].version, "3.0.0");
        assert_eq!(e[1].date, "2026-05-01");
        assert_eq!(
            e[1].sections,
            vec![
                (
                    "Added".to_owned(),
                    vec![
                        "Big feature continued here".to_owned(),
                        "Star item".to_owned()
                    ]
                ),
                ("Fixed".to_owned(), vec!["A bug".to_owned()]),
            ]
        );
        assert_eq!(
            e[1].body,
            "### Added\n- Big feature continued here\n- Star item\n\n### Fixed\n- A bug"
        );
        assert_eq!(e[2].date, "2026-04-20");
        assert_eq!(e[3].date, "2026-01-01");
        assert_eq!(parse(""), Vec::new());
        assert_eq!(parse("   \n"), Vec::new());
    }

    #[test]
    fn newer_versions_respect_channel() {
        let e = parse(SAMPLE);
        let stable: Vec<String> = newer_versions(&e, "2.0.0")
            .into_iter()
            .map(|e| e.version)
            .collect();
        assert_eq!(stable, ["Unreleased", "3.0.0"], "stable skips prereleases");
        let alpha: Vec<String> = newer_versions(&e, "3.0.0-alpha.1")
            .into_iter()
            .map(|e| e.version)
            .collect();
        assert_eq!(alpha, ["Unreleased", "3.0.0", "3.0.0-alpha.2"]);
        assert_eq!(
            newer_versions(&e, "9.9.9")
                .into_iter()
                .map(|e| e.version)
                .collect::<Vec<_>>(),
            ["Unreleased"],
            "a version absent from the changelog does not report all history"
        );
        let capped: Vec<String> = up_to(newer_versions(&e, "3.0.0-alpha.1"), "3.0.0-alpha.2")
            .into_iter()
            .map(|e| e.version)
            .collect();
        assert_eq!(capped, ["Unreleased", "3.0.0-alpha.2"]);
    }

    #[test]
    fn fallback_refs_like_unipi() {
        assert_eq!(fallback_refs("3.0.0-alpha.36"), ["v3.0.0-alpha", "main"]);
        assert_eq!(fallback_refs("3.0.0"), ["main"]);
        assert_eq!(fallback_refs("3.0.0-beta"), ["main"]);
    }

    #[test]
    fn renders_markdown() {
        let e = parse(SAMPLE);
        let md = render(&e[1..2]);
        assert!(md.starts_with("## 3.0.0 — 2026-05-01\n\n### Added"));
        assert_eq!(render(&e[..1]), "## Unreleased\n\n### Added\n- Next thing");
    }
}
