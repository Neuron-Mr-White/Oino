//! Property tests for the template argument parser and substitution.
#![allow(clippy::unwrap_used, reason = "proptest bodies")]

use oino_resources::templates::{parse_command_args, substitute_args};
use proptest::prelude::*;

proptest! {
    #[test]
    fn text_without_dollar_is_unchanged(content in "[^$]{0,64}", args in prop::collection::vec(".{0,8}", 0..4)) {
        prop_assert_eq!(substitute_args(&content, &args), content);
    }

    #[test]
    fn substitution_never_panics(content in "[$@{}:0-9A-Za-z -]{0,48}", args in prop::collection::vec("[ -~]{0,6}", 0..5)) {
        let _ = substitute_args(&content, &args);
    }

    #[test]
    fn plain_words_round_trip(words in prop::collection::vec("[a-z0-9_./-]{1,8}", 0..6), sep in "[ \t]{1,3}") {
        prop_assert_eq!(parse_command_args(&words.join(&sep)), words);
    }

    #[test]
    fn quoted_words_keep_inner_whitespace(word in "[a-z ]{0,10}") {
        prop_assert_eq!(parse_command_args(&format!("\"{word}\"")), vec![word]);
    }
}
