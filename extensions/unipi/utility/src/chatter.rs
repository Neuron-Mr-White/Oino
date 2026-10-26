//! Chatter detection (unipi core `jev/chatter.ts`): greetings, thanks, acknowledgements and
//! bare "continue"-style nudges carry no subject, so no naming decision is made from them.

use std::sync::LazyLock;

use regex::Regex;

static CHATTER: LazyLock<Option<Regex>> = LazyLock::new(|| {
    let forms = [
        "h(?:i|ello|ey|owdy)(?: there)?",
        "yo",
        "sup",
        "good (?:morning|afternoon|evening)",
        "thanks?(?: you)?(?: so much)?",
        "thx",
        "ty",
        "cheers",
        "nice",
        "great",
        "cool",
        "perfect",
        "awesome",
        "ok(?:ay)?",
        "k",
        "yes",
        "yep",
        "yeah",
        "no",
        "nope",
        "sure",
        "lgtm",
        "sounds good",
        "go(?: on| ahead)?",
        "continue",
        "proceed",
        "retry",
        "again",
        "do it",
        "next",
        "done",
    ];
    Regex::new(&format!(r"(?i)^(?:{})[\s.!?,]*$", forms.join("|"))).ok()
});

/// True for messages with no subject of their own (unipi `isChatter`).
#[must_use]
pub fn is_chatter(text: &str) -> bool {
    let t = text.trim();
    if t.chars().count() < 3 || t.starts_with('/') {
        return true;
    }
    CHATTER.as_ref().is_some_and(|re| re.is_match(t))
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::assert_is_empty,
    clippy::format_collect,
    clippy::cloned_ref_to_slice_refs,
    clippy::duration_suboptimal_units,
    clippy::too_many_lines,
    clippy::needless_pass_by_value,
    reason = "tests assert by unwrapping"
)]
mod tests {
    use super::*;

    #[test]
    fn greetings_acks_and_commands_are_chatter() {
        for t in [
            "hi",
            "Hello there!",
            "thanks",
            "ok",
            "go ahead",
            "continue",
            "/unipi:plan",
            "yes.",
            "Thank you so much!!",
            "k",
        ] {
            assert!(is_chatter(t), "{t}");
        }
        for t in [
            "fix the kanboard lane ordering",
            "hi, can you fix the build",
            "okay so the parser breaks",
        ] {
            assert!(!is_chatter(t), "{t}");
        }
    }
}
