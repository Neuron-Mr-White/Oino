//! Design guards (design.md §2.1 contrast rules, §9.3 no literal styles). Toolkit-free.

use std::path::Path;

use crate::design::tokens::{DARK, LIGHT, Palette, contrast, over};

fn check(p: &Palette, name: &str) {
    for (bg_name, bg) in [
        ("frame", p.frame),
        ("card", p.card),
        ("input", p.input),
        ("menu", p.menu),
    ] {
        let bg = over(bg, p.window);
        let fg = contrast(p.fg, bg);
        assert!(fg >= 4.5, "{name}: fg on {bg_name} = {fg:.2}");
        let subtle = contrast(p.fg_subtle, bg);
        assert!(
            subtle >= 4.5,
            "{name}: fg_subtle on {bg_name} = {subtle:.2}"
        );
    }
    let frame = over(p.frame, p.window);
    for (pair, fg, surface) in [
        ("success", p.success_surface_fg, p.success_surface),
        ("warning", p.warning_surface_fg, p.warning_surface),
        ("danger", p.danger_surface_fg, p.danger_surface),
        ("progress", p.progress_surface_fg, p.progress_surface),
        ("waiting", p.waiting_fg, p.waiting_surface),
        ("info", p.info_surface_fg, p.info_surface),
    ] {
        let bg = over(surface, frame);
        let ratio = contrast(fg, bg);
        assert!(ratio >= 4.5, "{name}: {pair} surface = {ratio:.2}");
    }
    // A2.2 surfaces: bubble, sheet, chip; needs-you strip; diff text on diff fills.
    for (pair, fg, surface) in [
        ("fg on bubble", p.fg, p.bubble),
        ("fg_subtle on sheet", p.fg_subtle, p.sheet),
        ("fg on chip", p.fg, p.chip),
        ("attention", p.attention_fg, p.attention_surface),
        ("diff added", p.diff_added_fg, p.diff_added_bg),
        ("diff removed", p.diff_removed_fg, p.diff_removed_bg),
        ("strong on window", p.fg_strong, p.window),
        ("link on card", p.link, p.card),
    ] {
        let bg = over(over(surface, frame), p.window);
        let ratio = contrast(fg, bg);
        assert!(ratio >= 4.5, "{name}: {pair} = {ratio:.2}");
    }
    // The selected row tint keeps body text readable.
    let selected = over(p.selected, over(p.card, p.window));
    let ratio = contrast(p.fg, selected);
    assert!(ratio >= 4.5, "{name}: fg on selected = {ratio:.2}");
    // A1.2 fixes the pair: `face #181818` on `shell #F03018` (4.35:1). Primary labels are
    // medium-weight UI text; the floor is 4.3 so a palette drift still fails.
    let primary = contrast(p.primary_fg, p.primary);
    assert!(primary >= 4.3, "{name}: primary = {primary:.2}");
    // Destructive fills (`deep`) carry white labels.
    let deep = contrast(0xFFFF_FFFF, p.deep);
    assert!(deep >= 4.5, "{name}: deep = {deep:.2}");
}

#[test]
fn contrast_rules() {
    check(&LIGHT, "light");
    check(&DARK, "dark");
}

/// Files allowed to contain literal styles, with the reason.
const EXCEPTIONS: &[(&str, &str)] = &[];

fn scan(dir: &Path, root: &Path, hits: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            scan(&path, root, hits);
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .display()
            .to_string();
        if EXCEPTIONS.iter().any(|(p, _)| rel.starts_with(p)) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        for (n, line) in text.lines().enumerate() {
            let code = line.split("//").next().unwrap_or_default();
            let literal_px = code.match_indices("px(").any(|(i, _)| {
                code[i + 3..]
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_digit() || c == '-')
            });
            if literal_px
                || code.contains("hsla(")
                || code.contains("rgb(")
                || code.contains("rgba(")
            {
                hits.push(format!("{rel}:{}: {}", n + 1, line.trim()));
            }
        }
    }
}

#[test]
fn no_literal_styles() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut hits = Vec::new();
    for dir in ["src/views", "src/components", "src/mobile"] {
        scan(&root.join(dir), root, &mut hits);
    }
    assert!(
        hits.is_empty(),
        "literal styles outside design/:\n{}",
        hits.join("\n")
    );
}
