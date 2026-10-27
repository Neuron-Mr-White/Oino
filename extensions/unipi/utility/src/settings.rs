//! Settings namespaces `utility` and `image` (unipi `src/settings.ts`, `src/image/settings.ts`).
//!
//! Values arrive resolved from the host (`defaults ⊕ global ⊕ project`); [`UtilSettings::from_json`]
//! and [`ImageConfig::from_json`] are unipi's never-failing normalisers (wrong-typed fields fall
//! back to defaults).

use serde_json::{Map, Value, json};
use unipi_kit::jev;
use unipi_kit::settings::{Field, Kind, Namespace, Section, choice, set_field};

/// Namespace of the utility settings.
pub const NAMESPACE: &str = "utility";
/// Namespace of the image tool settings.
pub const IMAGE_NAMESPACE: &str = "image";

/// `answer.method`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AnswerMethod {
    /// Fixed input under the scrollable reply.
    #[default]
    Reply,
    /// Q/A template in the editor.
    Questions,
    /// Browser form on a loopback port.
    Web,
}

impl AnswerMethod {
    /// Parses a method name (`editor` is the old name of `questions`).
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "reply" => Some(Self::Reply),
            "questions" | "editor" => Some(Self::Questions),
            "web" => Some(Self::Web),
            _ => None,
        }
    }
}

/// `render.style`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RenderStyle {
    /// One collapsed line per tool (unipi default).
    #[default]
    Simple,
    /// The frontend's own rendering.
    Regular,
    /// Highlighted commands, diffs, test summaries.
    Advanced,
}

impl RenderStyle {
    /// Parses a style name.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "simple" => Some(Self::Simple),
            "regular" => Some(Self::Regular),
            "advanced" => Some(Self::Advanced),
            _ => None,
        }
    }

    /// Wire name (also the `ui.density` value).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Simple => "simple",
            Self::Regular => "regular",
            Self::Advanced => "advanced",
        }
    }
}

/// Automatic session naming.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenameSettings {
    /// Rename after confirmed rounds that start or change the topic.
    pub auto: bool,
    /// `""` = the session model, else `provider/model-id`.
    pub model: String,
}

/// `/answer` defaults.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AnswerSettings {
    /// Default method.
    pub method: AnswerMethod,
    /// Questions hint above the editor.
    pub hint: bool,
    /// Web form port (0 = any; 47321 is tried first over SSH).
    pub port: u16,
}

/// Pasted / dropped file handling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AttachmentSettings {
    /// Turn pasted paths into tokens.
    pub enabled: bool,
    /// Inline previews.
    pub preview: bool,
}

/// The whole `utility` namespace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UtilSettings {
    /// Naming.
    pub rename: RenameSettings,
    /// Answer.
    pub answer: AnswerSettings,
    /// Attachments.
    pub attachments: AttachmentSettings,
    /// Tool rendering.
    pub render: RenderStyle,
    /// Debug trail to `/logs/rename.log`.
    pub debug: bool,
}

impl Default for UtilSettings {
    fn default() -> Self {
        Self::from_json(&Value::Null)
    }
}

fn obj<'a>(v: &'a Value, key: &str) -> &'a Value {
    v.get(key).filter(|x| x.is_object()).unwrap_or(&Value::Null)
}

fn bool_or(v: Option<&Value>, d: bool) -> bool {
    v.and_then(Value::as_bool).unwrap_or(d)
}

impl UtilSettings {
    /// unipi `normalizeSettings`: never fails.
    #[must_use]
    pub fn from_json(raw: &Value) -> Self {
        let p = expand_dotted(raw);
        let rename = obj(&p, "rename");
        let answer = obj(&p, "answer");
        let att = obj(&p, "attachments");
        let render = obj(&p, "render");
        let model = rename.get("model").and_then(Value::as_str).unwrap_or("");
        let port = answer
            .get("port")
            .and_then(Value::as_f64)
            .filter(|p| (0.0..=65_535.0).contains(p))
            .map_or(0, |p| {
                #[allow(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "range-checked above"
                )]
                let port = p.floor() as u16;
                port
            });
        Self {
            rename: RenameSettings {
                auto: bool_or(rename.get("auto"), true),
                model: if model == "inherit" {
                    String::new()
                } else {
                    model.to_owned()
                },
            },
            answer: AnswerSettings {
                method: answer
                    .get("method")
                    .and_then(Value::as_str)
                    .and_then(AnswerMethod::parse)
                    .unwrap_or_default(),
                hint: bool_or(answer.get("hint"), true),
                port,
            },
            attachments: AttachmentSettings {
                enabled: bool_or(att.get("enabled"), true),
                preview: bool_or(att.get("preview"), true),
            },
            render: render
                .get("style")
                .and_then(Value::as_str)
                .and_then(RenderStyle::parse)
                .unwrap_or_default(),
            debug: p.get("debug").and_then(Value::as_bool).unwrap_or(false),
        }
    }
}

/// Values with dotted keys (`"rename.auto": false`) expanded into nested objects.
#[must_use]
pub fn expand_dotted(v: &Value) -> Value {
    let Value::Object(map) = v else {
        return v.clone();
    };
    let mut nested = Map::new();
    let mut flat = Vec::new();
    for (k, val) in map {
        if k.contains('.') {
            flat.push((k.clone(), val.clone()));
        } else {
            nested.insert(k.clone(), expand_dotted(val));
        }
    }
    let mut out = Value::Object(nested);
    for (k, val) in flat {
        out = set_field(&out, &k, val);
    }
    out
}

/// Defaults of the `utility` namespace.
#[must_use]
pub fn defaults() -> Value {
    json!({
        "rename": {"auto": true, "model": "", "herdrSync": true},
        "answer": {"method": "reply", "hint": true, "port": 0},
        "attachments": {"enabled": true, "preview": true},
        "render": {"style": "simple"},
        "decisionModel": jev::consumer_defaults(),
        "debug": false,
    })
}

/// The `utility` namespace schema (unipi `registerSettings({namespace: "utility"})`).
#[must_use]
pub fn namespace(extension_id: &str) -> Namespace {
    Namespace::new(NAMESPACE, "Utility", extension_id)
        .defaults(defaults())
        .section(
            Section::new("rename", "Session name")
                .description("Automatic naming after rounds that start or change the topic")
                .field(Field::boolean("rename.auto", "Auto-rename").description(
                    "Name the session when a real request starts or changes the topic; greetings and short replies are skipped.",
                ))
                .field(
                    Field::model("rename.model", "Naming model")
                        .with_kind(|k| {
                            if let Kind::Model { filter, empty_option, .. } = k {
                                *filter = Some(json!({"capability": "text"}));
                                *empty_option = Some("inherit (session model)".into());
                            }
                        })
                        .empty_label("inherit (session model)")
                        .description("Model for the naming side session; empty uses the session model."),
                )
                .field(
                    Field::action("rename.now", "Rename now", "rename-now")
                        .description("Name the session from the recent requests right away."),
                ),
        )
        .section(
            Section::new("answer", "Answer")
                .description("/answer — answer the last reply's questions without scrolling")
                .field(
                    Field::enumeration(
                        "answer.method",
                        "Default method",
                        vec![
                            choice("reply", "reply", "fixed input under the scrollable reply"),
                            choice("questions", "questions", "one answer field per question"),
                            choice("web", "web form", "form in the browser"),
                        ],
                    )
                    .description("How /answer collects your answers; the reply|questions|web argument overrides it per use."),
                )
                .field(Field::boolean("answer.hint", "Questions hint").description(
                    "After a reply that asks questions, show a hint above the editor.",
                ))
                .field(
                    Field::number("answer.port", "Web form port")
                        .with_kind(|k| {
                            if let Kind::Number { min, max, zero_label, .. } = k {
                                *min = Some(0.0);
                                *max = Some(65_535.0);
                                *zero_label = Some("any free port (47321 over SSH)".into());
                            }
                        })
                        .description("Fix it to keep one ssh -L forward working."),
                ),
        )
        .section(
            Section::new("render", "Response formatting")
                .description("How tool calls look in the transcript")
                .field(
                    Field::enumeration(
                        "render.style",
                        "Style",
                        vec![
                            choice("simple", "simple", "one collapsed line per tool"),
                            choice("regular", "regular", "the frontend's own tool rendering"),
                            choice("advanced", "advanced", "highlighted commands, diffs, test summaries"),
                        ],
                    )
                    .description("How tool calls look in the transcript; expanding a row shows the full output in every style."),
                ),
        )
        .section(
            Section::new("attachments", "Attachments")
                .description("Pasted, dropped or Ctrl+V'd files")
                .field(Field::boolean("attachments.enabled", "Attach pasted files").description(
                    "Turn pasted image/file paths into [Image #N] / [File #N]; images are sent as images.",
                ))
                .field(Field::boolean("attachments.preview", "Inline previews").description(
                    "Small previews above the editor and in the transcript where the frontend can draw images.",
                )),
        )
        .section(
            Section::new("debug", "Diagnostics")
                .advanced(true)
                .field(Field::boolean("debug", "Debug log").description(
                    "Write the auto-rename decision trail to the extension log folder (rename.log).",
                )),
        )
        .section(jev::consumer_section(Some("Session name — Decision model"), true))
}

// ── image ────────────────────────────────────────────────────────────────

/// Built-in analyst prompt of `image_recognize`.
pub const DEFAULT_RECOGNIZE_SYSTEM_PROMPT: &str = "You are a precise image analyst assisting a software engineer. Describe what is actually visible — never speculate about what is not shown. For screenshots, transcribe visible text, UI structure, and any errors verbatim. For diagrams, describe the components and their relationships. For photographs, describe the subject, setting, and notable detail. Be specific and concise; lead with the single most important observation.";

/// Default text → image model (FLUX.2 klein 4B does both generation and edits).
pub const DEFAULT_GENERATE_MODEL: &str = "openrouter/black-forest-labs/flux.2-klein-4b";
/// Default image edit model.
pub const DEFAULT_EDIT_MODEL: &str = "openrouter/black-forest-labs/flux.2-klein-4b";
/// Default output folder: the extension's global state folder (`/ext-global/images`).
pub const DEFAULT_OUTPUT_DIR: &str = "";

/// Wire shape of a custom image endpoint.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ImageApi {
    /// `POST {base}/images/generations`.
    #[default]
    OpenAiImages,
    /// OpenRouter chat completions with image modalities.
    OpenRouter,
    /// `fal.run/<model>`.
    Fal,
}

impl ImageApi {
    /// Parses an api name (unknown → OpenAI images, unipi `loadConfig`).
    #[must_use]
    pub fn parse(s: &str) -> Self {
        match s {
            "openrouter" => Self::OpenRouter,
            "fal" => Self::Fal,
            _ => Self::OpenAiImages,
        }
    }
}

/// A generate/edit endpoint.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Endpoint {
    /// Custom base URL (`""` = route by provider).
    pub base_url: String,
    /// Key of the custom endpoint.
    pub api_key: String,
    /// Wire shape.
    pub api: ImageApi,
}

/// `generate` / `edit` section.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GenerateSettings {
    /// Tool registered.
    pub enabled: bool,
    /// `provider/model-id`.
    pub model: String,
    /// Output folder (host path; `""` = default).
    pub output_dir: String,
    /// Write results to disk.
    pub save_to_disk: bool,
    /// Custom endpoint.
    pub endpoint: Endpoint,
}

/// `recognize` section.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecognizeSettings {
    /// Tool registered.
    pub enabled: bool,
    /// `""` = the session model.
    pub model: String,
    /// Analyst prompt.
    pub system_prompt: String,
    /// Custom chat endpoint.
    pub base_url: String,
    /// Its key.
    pub api_key: String,
}

/// The `image` namespace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageConfig {
    /// Text → image.
    pub generate: GenerateSettings,
    /// Image + text → image.
    pub edit: GenerateSettings,
    /// Image → text.
    pub recognize: RecognizeSettings,
    /// Stored OpenRouter key.
    pub openrouter_key: String,
    /// Stored fal key.
    pub fal_key: String,
}

impl Default for ImageConfig {
    fn default() -> Self {
        Self::from_json(&Value::Null)
    }
}

fn s_or(v: &Value, key: &str, d: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or(d).to_owned()
}

fn endpoint(v: &Value) -> Endpoint {
    Endpoint {
        base_url: s_or(v, "baseUrl", ""),
        api_key: s_or(v, "apiKey", ""),
        api: ImageApi::parse(&s_or(v, "api", "openai-images")),
    }
}

impl ImageConfig {
    /// unipi `loadConfig`: sections merged over defaults, wrong-typed fields ignored.
    #[must_use]
    pub fn from_json(raw: &Value) -> Self {
        let all = expand_dotted(raw);
        let gen_s = obj(&all, "generate");
        let edit_s = obj(&all, "edit");
        let rec_s = obj(&all, "recognize");
        let keys = obj(&all, "keys");
        Self {
            generate: GenerateSettings {
                enabled: bool_or(gen_s.get("enabled"), true),
                model: s_or(gen_s, "model", DEFAULT_GENERATE_MODEL),
                output_dir: s_or(gen_s, "outputDir", DEFAULT_OUTPUT_DIR),
                save_to_disk: bool_or(gen_s.get("saveToDisk"), true),
                endpoint: endpoint(gen_s),
            },
            edit: GenerateSettings {
                enabled: bool_or(edit_s.get("enabled"), true),
                model: s_or(edit_s, "model", DEFAULT_EDIT_MODEL),
                output_dir: String::new(),
                save_to_disk: true,
                endpoint: endpoint(edit_s),
            },
            recognize: RecognizeSettings {
                enabled: bool_or(rec_s.get("enabled"), true),
                model: s_or(rec_s, "model", ""),
                system_prompt: Some(s_or(rec_s, "systemPrompt", DEFAULT_RECOGNIZE_SYSTEM_PROMPT))
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or_else(|| DEFAULT_RECOGNIZE_SYSTEM_PROMPT.to_owned()),
                base_url: s_or(rec_s, "baseUrl", ""),
                api_key: s_or(rec_s, "apiKey", ""),
            },
            openrouter_key: s_or(keys, "openrouter", ""),
            fal_key: s_or(keys, "fal", ""),
        }
    }
}

/// Defaults of the `image` namespace.
#[must_use]
pub fn image_defaults() -> Value {
    let none = json!({"baseUrl": "", "apiKey": "", "api": "openai-images"});
    let mut g = json!({"enabled": true, "model": DEFAULT_GENERATE_MODEL, "outputDir": DEFAULT_OUTPUT_DIR, "saveToDisk": true});
    let mut e = json!({"enabled": true, "model": DEFAULT_EDIT_MODEL});
    for (k, v) in none.as_object().into_iter().flatten() {
        g[k] = v.clone();
        e[k] = v.clone();
    }
    json!({
        "generate": g,
        "edit": e,
        "recognize": {"enabled": true, "model": "", "systemPrompt": DEFAULT_RECOGNIZE_SYSTEM_PROMPT, "baseUrl": "", "apiKey": ""},
        "keys": {"openrouter": "", "fal": ""},
    })
}

fn api_options() -> Vec<unipi_kit::settings::Choice> {
    vec![
        choice(
            "openai-images",
            "OpenAI images",
            "POST /images/generations with the model id as typed",
        ),
        choice(
            "openrouter",
            "OpenRouter chat",
            "chat completions that return image modalities",
        ),
        choice("fal", "fal", "fal.run/<model> REST endpoint"),
    ]
}

fn endpoint_page(prefix: &str, title: &str, with_api: bool) -> Field {
    let mut s = Section::new(&format!("{prefix}-endpoint"), title)
        .field(
            Field::string(&format!("{prefix}.baseUrl"), "Base URL")
                .empty_label("none (use the model's provider)")
                .description("OpenAI-compatible base URL the model id is sent to."),
        )
        .field(
            Field::secret(&format!("{prefix}.apiKey"), "API key")
                .empty_label("none")
                .description(
                    "Key for the custom endpoint; without it the request goes out unsigned.",
                ),
        );
    if with_api {
        s = s.field(
            Field::enumeration(&format!("{prefix}.api"), "API", api_options())
                .description("Wire shape the custom endpoint speaks."),
        );
    }
    Field::page(&format!("{prefix}.endpoint"), "Custom endpoint…", vec![s])
        .description("Send the model id as typed to your own gateway instead of its provider.")
}

fn model_field(key: &str, filter: Value) -> Field {
    Field::model(key, "Model")
        .with_kind(|k| {
            if let Kind::Model { filter: f, .. } = k {
                *f = Some(filter);
            }
        })
        .dynamic(crate::image::MODELS_SERVICE)
}

/// The `image` namespace schema (unchanged keys from the former `@pi-unipi/image`).
#[must_use]
pub fn image_namespace(extension_id: &str) -> Namespace {
    Namespace::new(IMAGE_NAMESPACE, "Image", extension_id)
        .defaults(image_defaults())
        .section(
            Section::new("generate", "Generate")
                .description("Text → image (image_generate)")
                .field(Field::boolean("generate.enabled", "Enabled").description(
                    "Give the agent the image_generate tool. Applies from the next session start.",
                ))
                .field(model_field("generate.model", json!({"output": ["image"]})).description("Models whose output includes an image."))
                .field(
                    Field::string("generate.outputDir", "Save to folder")
                        .empty_label("the utility's image folder")
                        .description("Folder generated images are written to (inside the project or the utility's state folder). ~ expands to your home."),
                )
                .field(Field::boolean("generate.saveToDisk", "Save to disk").description(
                    "Write results to the folder; off hands them to the agent only.",
                ))
                .field(endpoint_page("generate", "Generate endpoint", true)),
        )
        .section(
            Section::new("edit", "Edit")
                .description("Image + text → image (image_edit)")
                .field(Field::boolean("edit.enabled", "Enabled").description(
                    "Give the agent the image_edit tool. Applies from the next session start.",
                ))
                .field(model_field("edit.model", json!({"input": ["image"], "output": ["image"]})).description("Models that take an image and return an image."))
                .field(endpoint_page("edit", "Edit endpoint", true)),
        )
        .section(
            Section::new("recognize", "Recognize")
                .description("Image → text (image_recognize; hidden while the session model can see images)")
                .field(Field::boolean("recognize.enabled", "Enabled").description(
                    "Give text-only models the image_recognize tool. Applies from the next session start.",
                ))
                .field(
                    Field::model("recognize.model", "Model")
                        .with_kind(|k| {
                            if let Kind::Model { filter, empty_option, .. } = k {
                                *filter = Some(json!({"input": ["image"]}));
                                *empty_option = Some("inherit (session model)".into());
                            }
                        })
                        .empty_label("inherit (session model)")
                        .description("Vision model that describes images; empty uses the session model."),
                )
                .field(
                    Field::string("recognize.systemPrompt", "System prompt")
                        .empty_label("built-in analyst prompt")
                        .description("Tells the vision model how to describe screenshots and diagrams."),
                )
                .field(endpoint_page("recognize", "Recognize endpoint", false)),
        )
        .section(
            Section::new("keys", "Keys")
                .description("Used when the model's provider is OpenRouter or fal")
                .advanced(true)
                .field(
                    Field::secret("keys.openrouter", "OpenRouter key")
                        .empty_label("Oino login or OPENROUTER_API_KEY")
                        .description("Stored key wins over the environment for OpenRouter models."),
                )
                .field(
                    Field::secret("keys.fal", "fal key")
                        .empty_label("FAL_KEY / FAL_API_KEY")
                        .description("Stored key wins over the environment for fal models."),
                ),
        )
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
    fn defaults_normalise() {
        let s = UtilSettings::from_json(&defaults());
        assert!(s.rename.auto);
        assert_eq!(s.rename.model, "");
        assert_eq!(s.answer.method, AnswerMethod::Reply);
        assert!(s.answer.hint);
        assert_eq!(s.answer.port, 0);
        assert!(s.attachments.enabled && s.attachments.preview);
        assert_eq!(s.render, RenderStyle::Simple);
        assert_eq!(UtilSettings::default(), s);
    }

    #[test]
    fn rename_model_inherit_is_default() {
        assert_eq!(
            UtilSettings::from_json(&json!({"rename": {"model": "inherit"}}))
                .rename
                .model,
            ""
        );
    }

    #[test]
    fn answer_and_render_values() {
        let s = UtilSettings::from_json(
            &json!({"answer": {"method": "editor", "port": 70000}, "render": {"style": "fancy"}}),
        );
        assert_eq!(s.answer.method, AnswerMethod::Questions);
        assert_eq!(s.answer.port, 0);
        assert_eq!(s.render, RenderStyle::Simple);
        let s = UtilSettings::from_json(
            &json!({"answer.method": "web", "answer.port": 8080.7, "render.style": "advanced"}),
        );
        assert_eq!(s.answer.method, AnswerMethod::Web);
        assert_eq!(s.answer.port, 8080);
        assert_eq!(s.render, RenderStyle::Advanced);
        assert_eq!(RenderStyle::Regular.as_str(), "regular");
    }

    #[test]
    fn image_config_merges_and_validates() {
        let c = ImageConfig::from_json(
            &json!({"generate": {"model": "fal/x", "api": "bogus", "enabled": "yes"}, "keys": {"fal": "k"}}),
        );
        assert_eq!(c.generate.model, "fal/x");
        assert_eq!(c.generate.endpoint.api, ImageApi::OpenAiImages);
        assert!(
            c.generate.enabled,
            "wrong-typed value falls back to the default"
        );
        assert_eq!(c.fal_key, "k");
        assert_eq!(c.edit.model, DEFAULT_EDIT_MODEL);
        assert_eq!(c.recognize.system_prompt, DEFAULT_RECOGNIZE_SYSTEM_PROMPT);
        let d = ImageConfig::from_json(&image_defaults());
        assert_eq!(d, ImageConfig::default());
        assert_eq!(ImageApi::parse("openrouter"), ImageApi::OpenRouter);
        assert_eq!(ImageApi::parse("fal"), ImageApi::Fal);
    }

    #[test]
    fn schemas_cover_the_unipi_keys() {
        let ns = namespace("unipi.utility");
        for key in [
            "rename.auto",
            "rename.model",
            "rename.now",
            "answer.method",
            "answer.hint",
            "answer.port",
            "render.style",
            "attachments.enabled",
            "attachments.preview",
            "decisionModel.source",
        ] {
            assert!(ns.field(key).is_some(), "{key}");
        }
        let img = image_namespace("unipi.utility");
        for key in [
            "generate.enabled",
            "generate.model",
            "generate.outputDir",
            "generate.saveToDisk",
            "generate.baseUrl",
            "generate.apiKey",
            "generate.api",
            "edit.enabled",
            "edit.model",
            "edit.baseUrl",
            "recognize.enabled",
            "recognize.model",
            "recognize.systemPrompt",
            "recognize.baseUrl",
            "recognize.apiKey",
            "keys.openrouter",
            "keys.fal",
        ] {
            assert!(img.field(key).is_some(), "{key}");
        }
        assert_eq!(ns.to_json()["namespace"], "utility");
        assert_eq!(img.to_json()["namespace"], "image");
    }
}
