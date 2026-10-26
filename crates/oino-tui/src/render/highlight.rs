//! Syntax highlighting with syntect, mapped onto pi's `syntax*` theme tokens.

use std::sync::OnceLock;

use ratatui::style::{Modifier, Style};
use ratatui::text::Span;
use syntect::easy::ScopeRegionIterator;
use syntect::parsing::{ParseState, ScopeStack, SyntaxReference, SyntaxSet};

use crate::theme::Theme;

fn syntaxes() -> &'static SyntaxSet {
    static SET: OnceLock<SyntaxSet> = OnceLock::new();
    SET.get_or_init(SyntaxSet::load_defaults_newlines)
}

fn syntax_for(lang: &str) -> Option<&'static SyntaxReference> {
    let set = syntaxes();
    let lang = lang.trim().trim_start_matches('.');
    if lang.is_empty() {
        return None;
    }
    let alias = match lang.to_ascii_lowercase().as_str() {
        "c++" => "cpp",
        "c#" => "cs",
        "golang" => "go",
        "js" | "node" | "javascript" | "jsx" | "ts" | "typescript" | "tsx" => "js",
        "py" | "python" => "py",
        "shell" | "bash" | "zsh" | "sh" => "sh",
        "yml" => "yaml",
        "rs" | "rust" => "rs",
        "md" | "markdown" => "md",
        other => {
            return set
                .find_syntax_by_token(other)
                .or_else(|| set.find_syntax_by_extension(other));
        }
    };
    set.find_syntax_by_extension(alias)
        .or_else(|| set.find_syntax_by_token(alias))
}

/// pi `getLanguageFromPath`: language token from a file path.
#[must_use]
pub(crate) fn language_from_path(path: &str) -> Option<String> {
    let name = path.rsplit('/').next().unwrap_or(path);
    let ext = name.rsplit_once('.').map(|(_, e)| e)?;
    syntax_for(ext).map(|_| ext.to_owned())
}

fn style_for(scopes: &ScopeStack, theme: &Theme, base: Style) -> Style {
    let scopes = scopes.to_string();
    let fg = |c| base.fg(c);
    if scopes.contains("comment") {
        return fg(theme.syntax_comment).add_modifier(Modifier::ITALIC);
    }
    if scopes.contains("string") || scopes.contains("constant.character") {
        return fg(theme.syntax_string);
    }
    if scopes.contains("constant.numeric") {
        return fg(theme.syntax_number);
    }
    if scopes.contains("keyword.operator") || scopes.contains("punctuation.operator") {
        return fg(theme.syntax_operator);
    }
    if scopes.contains("keyword") || scopes.contains("storage.modifier") {
        return fg(theme.syntax_keyword);
    }
    if scopes.contains("entity.name.function")
        || scopes.contains("support.function")
        || scopes.contains("variable.function")
    {
        return fg(theme.syntax_function);
    }
    if scopes.contains("entity.name.type")
        || scopes.contains("support.type")
        || scopes.contains("storage.type")
    {
        return fg(theme.syntax_type);
    }
    if scopes.contains("variable") || scopes.contains("constant.language") {
        return fg(theme.syntax_variable);
    }
    if scopes.contains("punctuation") {
        return fg(theme.syntax_punctuation);
    }
    base
}

/// Highlights `code` line by line; unknown languages use `base` for every line.
#[must_use]
pub(crate) fn highlight(
    code: &str,
    lang: Option<&str>,
    theme: &Theme,
    base: Style,
) -> Vec<Vec<Span<'static>>> {
    let Some(syntax) = lang.and_then(syntax_for) else {
        return code
            .lines()
            .map(|l| vec![Span::styled(l.to_owned(), base)])
            .collect();
    };
    let mut state = ParseState::new(syntax);
    let mut stack = ScopeStack::new();
    let mut out = Vec::new();
    for line in code.lines() {
        let with_newline = format!("{line}\n");
        let Ok(ops) = state.parse_line(&with_newline, syntaxes()) else {
            out.push(vec![Span::styled(line.to_owned(), base)]);
            continue;
        };
        let mut spans = Vec::new();
        for (region, op) in ScopeRegionIterator::new(&ops, &with_newline) {
            let _ = stack.apply(op);
            let region = region.trim_end_matches('\n');
            if region.is_empty() {
                continue;
            }
            spans.push(Span::styled(
                region.to_owned(),
                style_for(&stack, theme, base),
            ));
        }
        out.push(spans);
    }
    out
}
