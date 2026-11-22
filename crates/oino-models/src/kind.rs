//! Model kinds (F-02/F-04): `chat` vs `image` / `speech` / `embedding` / … so the UI can
//! group non-chat models away and the model cycle never steps on one.
//!
//! The catalogue marks many chat models with `"type": "chat"` ([`Model::kind`]) but carries
//! no marker at all for the rest — an image model looks like any other entry
//! (`minimax/image-01`). The kind is therefore derived: an explicit `chat` marker wins,
//! then the id/name vocabulary. Word-boundary tokens, the same shapes the UI's own
//! fallback matches (`image|img|imagen|dall-e|flux|speech|tts|…`), so the two layers agree.

use oino_ai::Model;

/// The derived kind of a model. `"chat"` is the only kind a session may run.
#[must_use]
pub fn model_kind(model: &Model) -> &'static str {
    if model.kind.is_some() {
        return "chat";
    }
    let hay = format!("{} {}", model.id, model.name).to_lowercase();
    let kind_of: [(&str, &str); 13] = [
        ("embedding", "embedding"),
        ("embed", "embedding"),
        ("rerank", "rerank"),
        ("moderation", "moderation"),
        ("speech", "speech"),
        ("tts", "speech"),
        ("stt", "speech"),
        ("whisper", "speech"),
        ("audio", "speech"),
        ("transcribe", "speech"),
        ("transcription", "speech"),
        ("imagen", "image"),
        ("dalle", "image"),
    ];
    for (token, kind) in kind_of {
        if has_token(&hay, token) {
            return kind;
        }
    }
    // "image-01", "mai-image-2.6", "gpt-image-2.5", "flux-1", "sora-2", "veo-3".
    for token in ["image", "img", "flux", "sora", "veo", "video"] {
        if has_token(&hay, token) {
            return if token == "video" { "video" } else { "image" };
        }
    }
    "chat"
}

/// Whether `hay` contains `token` as a word: delimited by start/end or one of
/// space `/ _ . : -` (the separators model ids actually use), optionally followed by
/// digits (`image-01`, `whisper-2`). Substrings inside a word never match, so
/// `influx` is not `flux` and `cambridge` is not a camera.
#[must_use]
fn has_token(hay: &str, token: &str) -> bool {
    hay.match_indices(token).any(|(at, _)| {
        let before_ok = at == 0
            || hay[..at]
                .chars()
                .next_back()
                .is_some_and(|c| matches!(c, ' ' | '/' | '_' | '.' | ':' | '-'));
        let after = &hay[at + token.len()..];
        let after_ok = after.chars().next().is_none_or(|c| {
            c.is_ascii_digit() || matches!(c, ' ' | '/' | '_' | '.' | ':' | '-' | '@')
        });
        before_ok && after_ok
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(id: &str, name: &str) -> Model {
        serde_json::from_value(serde_json::json!({
            "id": id, "name": name, "api": "openai-completions", "provider": "p",
            "baseUrl": "", "input": ["text"],
            "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0},
            "contextWindow": 128_000, "maxTokens": 16_384
        }))
        .unwrap()
    }

    #[test]
    fn chat_models_stay_chat() {
        assert_eq!(
            model_kind(&model("claude-opus-5-5", "Claude Opus 5.5")),
            "chat"
        );
        assert_eq!(model_kind(&model("gpt-5.2", "gpt-5.2")), "chat");
        assert_eq!(
            model_kind(&model("deepseek-flash", "DeepSeek Flash")),
            "chat"
        );
        // Vision input does not make a model non-chat ("vision" is not a token here).
        assert_eq!(model_kind(&model("gpt-4-vision", "GPT-4 Vision")), "chat");
        // The explicit catalogue marker wins even over a suspicious id.
        let mut marked = model("image-01", "Image 01");
        marked.kind = Some(oino_ai::ModelKindChat::Chat);
        assert_eq!(model_kind(&marked), "chat");
    }

    #[test]
    fn non_chat_families_are_named() {
        assert_eq!(model_kind(&model("minimax/image-01", "image-01")), "image");
        assert_eq!(
            model_kind(&model("minimax/image-01-live", "image-01-live")),
            "image"
        );
        assert_eq!(
            model_kind(&model(
                "openrouter/openai/gpt-image-2.5-sunburst",
                "gpt-image-2.5"
            )),
            "image"
        );
        assert_eq!(
            model_kind(&model("minimax/speech-2.8-hd", "speech-2.8-hd")),
            "speech"
        );
        assert_eq!(model_kind(&model("openai/whisper-2", "Whisper")), "speech");
        assert_eq!(
            model_kind(&model("openai/text-embedding-3", "embedding-3")),
            "embedding"
        );
        assert_eq!(model_kind(&model("co/rerank-v2", "rerank")), "rerank");
        assert_eq!(model_kind(&model("p/flux-1", "FLUX.1")), "image");
    }

    #[test]
    fn tokens_never_match_inside_words() {
        // "influx" must not read as flux.
        assert_eq!(model_kind(&model("influx-data", "Influx data")), "chat");
        // "images" (plural) is not an image generator, but "image-gen" is.
        assert_eq!(model_kind(&model("p/images-view", "Images view")), "chat");
        assert_eq!(model_kind(&model("p/image-gen", "image gen")), "image");
    }
}
