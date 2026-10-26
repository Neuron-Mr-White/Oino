//! pi `list-models.ts`: the `--list-models [search]` table.

use oino_ai::Model;

/// pi `formatTokenCount` (`200000` → `200K`, `1000000` → `1M`, `1500` → `1.5K`).
#[must_use]
pub fn format_token_count(count: u64) -> String {
    let scaled = |value: u64, unit: u64, suffix: &str| {
        if value.is_multiple_of(unit) {
            format!("{}{suffix}", value / unit)
        } else {
            // One decimal, rounded half up like `toFixed(1)` for these magnitudes.
            let tenths = (value * 10 + unit / 2) / unit;
            format!("{}.{}{suffix}", tenths / 10, tenths % 10)
        }
    };
    if count >= 1_000_000 {
        scaled(count, 1_000_000, "M")
    } else if count >= 1_000 {
        scaled(count, 1_000, "K")
    } else {
        count.to_string()
    }
}

/// Case-insensitive subsequence match of every whitespace-separated token (pi-tui
/// `fuzzyFilter` approximation).
fn fuzzy_matches(text: &str, query: &str) -> bool {
    let text = text.to_lowercase();
    query.split_whitespace().all(|token| {
        let mut chars = text.chars();
        token
            .to_lowercase()
            .chars()
            .all(|wanted| chars.any(|c| c == wanted))
    })
}

/// Rows of the table (header first), or the pi message when nothing matches.
#[must_use]
pub fn list_models_output(models: &[Model], search: Option<&str>, no_models: &str) -> String {
    if models.is_empty() {
        return format!("{no_models}\n");
    }
    let mut filtered: Vec<&Model> = models
        .iter()
        .filter(|m| search.is_none_or(|q| fuzzy_matches(&format!("{} {}", m.provider, m.id), q)))
        .collect();
    if filtered.is_empty() {
        return format!("No models matching \"{}\"\n", search.unwrap_or_default());
    }
    filtered.sort_by(|a, b| {
        (
            a.provider.to_lowercase(),
            &a.provider,
            a.id.to_lowercase(),
            &a.id,
        )
            .cmp(&(
                b.provider.to_lowercase(),
                &b.provider,
                b.id.to_lowercase(),
                &b.id,
            ))
    });
    let header = [
        "provider", "model", "context", "max-out", "thinking", "images",
    ]
    .map(str::to_owned);
    let rows: Vec<[String; 6]> = filtered
        .iter()
        .map(|m| {
            [
                m.provider.clone(),
                m.id.clone(),
                format_token_count(m.context_window),
                format_token_count(m.max_tokens),
                if m.reasoning { "yes" } else { "no" }.to_owned(),
                if m.input.contains(&oino_ai::InputModality::Image) {
                    "yes"
                } else {
                    "no"
                }
                .to_owned(),
            ]
        })
        .collect();
    let widths: Vec<usize> = (0..6)
        .map(|c| {
            rows.iter()
                .map(|r| r[c].chars().count())
                .chain([header[c].len()])
                .max()
                .unwrap_or(0)
        })
        .collect();
    let line = |cells: &[String; 6]| {
        let padded: Vec<String> = cells
            .iter()
            .zip(&widths)
            .map(|(cell, width)| format!("{cell:<width$}"))
            .collect();
        padded.join("  ")
    };
    let mut out = String::new();
    for cells in std::iter::once(&header).chain(rows.iter()) {
        out.push_str(&line(cells));
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_counts() {
        assert_eq!(format_token_count(200_000), "200K");
        assert_eq!(format_token_count(1_000_000), "1M");
        assert_eq!(format_token_count(1_500), "1.5K");
        assert_eq!(format_token_count(128_000), "128K");
        assert_eq!(format_token_count(999), "999");
    }

    #[test]
    fn fuzzy_tokens() {
        assert!(fuzzy_matches("faux faux-1", "fx 1"));
        assert!(!fuzzy_matches("faux faux-1", "zz"));
    }
}
