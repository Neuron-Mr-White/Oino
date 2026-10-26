//! Image tools (unipi `src/image/*`): `image_generate`, `image_edit`, `image_recognize`.
//!
//! Registered once at init from the `enabled` flags (static schemas keep the provider prefix
//! cache intact). Failures are tool errors with a message that says what to set.
//! `image_recognize` is hidden while the session model reads images itself and comes back for
//! text-only models (`session_start`, `model_select`).

pub mod models;
pub mod recognize;
pub mod source;
pub mod transports;

use std::collections::HashMap;

use oino_extension::prelude::*;
use serde::Deserialize;

use self::models::{Cache, Cred, catalog, resolve_route, resolve_vision_model, split_ref};
use self::transports::ImagePayload;
use crate::hostfs::{self, Roots};
use crate::settings::{Endpoint, ImageConfig};

/// `image_generate`.
pub const GENERATE: &str = "image_generate";
/// `image_edit`.
pub const EDIT: &str = "image_edit";
/// `image_recognize`.
pub const RECOGNIZE: &str = "image_recognize";
/// Service listing image models for the settings hub pickers.
pub const MODELS_SERVICE: &str = "image.models";
/// Default output folder (guest path).
pub const DEFAULT_OUTPUT: &str = "/ext-global/images";

/// `image_generate` arguments.
#[derive(Debug, Deserialize)]
pub struct GenerateArgs {
    /// Image description.
    pub prompt: String,
    /// `provider/model-id` override.
    #[serde(default)]
    pub model: Option<String>,
}

/// `image_edit` arguments.
#[derive(Debug, Deserialize)]
pub struct EditArgs {
    /// What to change.
    pub prompt: String,
    /// Source image.
    pub image: String,
    /// Model override.
    #[serde(default)]
    pub model: Option<String>,
}

/// `image_recognize` arguments.
#[derive(Debug, Deserialize)]
pub struct RecognizeArgs {
    /// Image.
    pub image: String,
    /// Question.
    #[serde(default)]
    pub prompt: Option<String>,
    /// Vision model override.
    #[serde(default)]
    pub model: Option<String>,
}

/// Tool spec: `(name, label, description, snippet, guidelines, parameters)`.
pub type ToolDef = (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    Vec<&'static str>,
    Value,
);

/// The three image tools.
#[must_use]
pub fn tool_defs() -> Vec<ToolDef> {
    vec![
        (
            GENERATE,
            "Generate Image",
            "Generate an image from a text prompt. Returned inline and saved to disk.",
            "Generate an image from a text prompt.",
            vec![
                "Write a detailed prompt — subject, style, composition and lighting all help.",
                "Describe what you DO want; negation is unreliable in image models.",
                "To change an existing image use image_edit instead.",
                "Omit model to use the one configured in /unipi:settings. Images cost money; do not regenerate unasked.",
            ],
            json!({"type": "object", "required": ["prompt"], "properties": {
                "prompt": {"type": "string", "description": "Description of the image. Be specific."},
                "model": {"type": "string", "description": "Override as \"provider/model-id\", e.g. \"openrouter/black-forest-labs/flux.2-klein-4b\" or \"fal/fal-ai/flux-2/klein/4b\"."},
            }}),
        ),
        (
            EDIT,
            "Edit Image",
            "Edit an existing image with a text instruction. Returned inline and saved to disk.",
            "Edit an existing image with a text instruction.",
            vec![
                "Pass the source image as a file path (preferred), data: URL or base64.",
                "Say what to change and what to keep; unmentioned details may change.",
            ],
            json!({"type": "object", "required": ["prompt", "image"], "properties": {
                "prompt": {"type": "string", "description": "What to change."},
                "image": {"type": "string", "description": "Source image: local file path, data: URL, or base64."},
                "model": {"type": "string", "description": "Override as \"provider/model-id\" (must accept an image and output one)."},
            }}),
        ),
        (
            RECOGNIZE,
            "Recognize Image",
            "Analyze an image and answer questions about it using a vision model.",
            "Analyze an image and answer questions about it.",
            vec![
                "Use image_recognize to read screenshots, diagrams, mockups and photos.",
                "Pass a local file path when possible — cheaper than inlining base64.",
                "Ask a specific question in `prompt` to focus the analysis.",
            ],
            json!({"type": "object", "required": ["image"], "properties": {
                "image": {"type": "string", "description": "Local file path, data: URL, or base64 (PNG, JPEG, GIF, WebP)."},
                "prompt": {"type": "string", "description": "What to ask about the image. Defaults to a general description."},
                "model": {"type": "string", "description": "Vision model override (must accept image input)."},
            }}),
        ),
    ]
}

/// URL-safe slug of a prompt (≤ 40 chars, `image` when empty).
#[must_use]
pub fn slugify(prompt: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for c in prompt.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
            dash = false;
        } else if !dash && !out.is_empty() {
            out.push('-');
            dash = true;
        }
    }
    let s: String = out.trim_end_matches('-').chars().take(40).collect();
    let s = s.trim_end_matches('-').to_owned();
    if s.is_empty() { "image".into() } else { s }
}

fn extension(mime: &str) -> &'static str {
    match mime {
        "image/jpeg" => ".jpg",
        "image/webp" => ".webp",
        "image/gif" => ".gif",
        _ => ".png",
    }
}

/// File names for saved images (`2026-09-28_01-02-03-a-red-cube.png`, then `-2`, `-3`, …).
#[must_use]
pub fn file_names(prompt: &str, images: &[ImagePayload], now_ms: i64) -> Vec<String> {
    let stamp: String = unipi_kit::fmt::iso_from_ms(now_ms)
        .replace([':', '.'], "-")
        .replacen('T', "_", 1)
        .chars()
        .take(19)
        .collect();
    let slug = slugify(prompt);
    images
        .iter()
        .enumerate()
        .map(|(i, img)| {
            let n = if i == 0 {
                String::new()
            } else {
                format!("-{}", i + 1)
            };
            format!("{stamp}-{slug}{n}{}", extension(&img.mime_type))
        })
        .collect()
}

/// Output folder (guest path): the setting when it maps to a writable preopen, else the
/// default `/ext-global/images`.
#[must_use]
pub fn output_dir(
    setting: &str,
    roots: &Roots,
    home: Option<&str>,
    writable: &dyn Fn(&str) -> bool,
) -> String {
    let s = setting.trim();
    if s.is_empty() {
        return DEFAULT_OUTPUT.into();
    }
    let host = hostfs::absolute(s, "/", home);
    roots
        .to_guest(&host)
        .map(|g| g.to_string_lossy().into_owned())
        .filter(|g| writable(g))
        .unwrap_or_else(|| DEFAULT_OUTPUT.into())
}

/// Saves images; a failed write never loses the image (it stays inline). Returns host paths.
fn save_images(dir: &str, prompt: &str, images: &[ImagePayload], roots: &Roots) -> Vec<String> {
    use base64::Engine as _;
    let _ = std::fs::create_dir_all(dir);
    file_names(prompt, images, timers::now_ms())
        .into_iter()
        .zip(images)
        .filter_map(|(name, img)| {
            let guest = std::path::Path::new(dir).join(name);
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(&img.data)
                .ok()?;
            std::fs::write(&guest, bytes).ok()?;
            Some(roots.to_host(&guest).map_or_else(
                || guest.to_string_lossy().into_owned(),
                |h| h.to_string_lossy().into_owned(),
            ))
        })
        .collect()
}

fn env(name: &str) -> Option<String> {
    host::env(name).ok().flatten().filter(|v| !v.is_empty())
}

fn secret(name: &str) -> Option<String> {
    secrets::get(&format!("{}/{name}", crate::settings::IMAGE_NAMESPACE))
        .ok()
        .flatten()
        .or_else(|| secrets::get(name).ok().flatten())
        .filter(|v| !v.is_empty())
}

/// Fills secret settings fields from the secret store.
#[must_use]
pub fn with_secrets(mut c: ImageConfig) -> ImageConfig {
    let fill = |slot: &mut String, key: &str| {
        if slot.is_empty()
            && let Some(v) = secret(key)
        {
            *slot = v;
        }
    };
    fill(&mut c.openrouter_key, "keys.openrouter");
    fill(&mut c.fal_key, "keys.fal");
    fill(&mut c.generate.endpoint.api_key, "generate.apiKey");
    fill(&mut c.edit.endpoint.api_key, "edit.apiKey");
    fill(&mut c.recognize.api_key, "recognize.apiKey");
    c
}

/// fal key: setting → `FAL_KEY` → `FAL_API_KEY`.
#[must_use]
pub fn fal_key(c: &ImageConfig) -> Option<String> {
    Some(c.fal_key.clone())
        .filter(|k| !k.is_empty())
        .or_else(|| env("FAL_KEY"))
        .or_else(|| env("FAL_API_KEY"))
}

async fn host_cred(provider: &str) -> Option<Cred> {
    let c = oino_extension::models::credential_for(provider)
        .await
        .ok()
        .flatten()?;
    (!c.headers.is_empty()).then_some(Cred::Headers(c.headers))
}

async fn cred_for(provider: &str, c: &ImageConfig) -> Option<Cred> {
    match provider {
        "openrouter" => match Some(c.openrouter_key.clone()).filter(|k| !k.is_empty()) {
            Some(k) => Some(Cred::Key(k)),
            None => match host_cred("openrouter").await {
                Some(h) => Some(h),
                None => env("OPENROUTER_API_KEY").map(Cred::Key),
            },
        },
        "fal" => fal_key(c).map(Cred::Key),
        other => match host_cred(other).await {
            Some(h) => Some(h),
            None => env(&models::provider_env_var(other)).map(Cred::Key),
        },
    }
}

/// `image_generate` / `image_edit`.
pub async fn run_generate(
    c: &ImageConfig,
    edit: bool,
    prompt: &str,
    model: Option<&str>,
    source: Option<ImagePayload>,
) -> Result<ToolResult> {
    let section = if edit { &c.edit } else { &c.generate };
    let over = model.map(str::trim).filter(|m| !m.is_empty());
    let reference = over.unwrap_or(&section.model).to_owned();
    let endpoint = if over.is_some() {
        Endpoint {
            base_url: String::new(),
            ..section.endpoint.clone()
        }
    } else {
        section.endpoint.clone()
    };
    let models = control::models().unwrap_or_default();
    let cache = Cache::read();
    let mut creds: HashMap<String, Cred> = HashMap::new();
    if endpoint.base_url.trim().is_empty()
        && let Some((provider, _)) = split_ref(&reference)
        && let Some(cred) = cred_for(provider, c).await
    {
        creds.insert(provider.to_owned(), cred);
    }
    let route = resolve_route(&reference, &endpoint, &models, &cache, &|p| {
        creds.get(p).cloned()
    })
    .map_err(Error::msg)?;
    let images: Vec<ImagePayload> = source.into_iter().collect();
    let out = transports::generate(&route, prompt, &images)
        .await
        .map_err(Error::msg)?;
    let roots = Roots::current();
    let saved = if c.generate.save_to_disk {
        let dir = output_dir(
            &c.generate.output_dir,
            &roots,
            hostfs::home().as_deref(),
            &|g| std::fs::create_dir_all(g).is_ok(),
        );
        save_images(&dir, prompt, &out.images, &roots)
    } else {
        Vec::new()
    };
    let n = out.images.len();
    let mut summary = vec![format!(
        "{} {n} image{} with {}.",
        if edit { "Edited" } else { "Generated" },
        if n == 1 { "" } else { "s" },
        route.label
    )];
    if !saved.is_empty() {
        summary.push(format!(
            "Saved to:\n{}",
            saved
                .iter()
                .map(|p| format!("  {p}"))
                .collect::<Vec<_>>()
                .join("\n")
        ));
    } else if c.generate.save_to_disk {
        summary.push("Could not save to disk; the image is inline only.".into());
    }
    if !out.text.is_empty() {
        summary.push(out.text.clone());
    }
    let mut content = vec![json!({"type": "text", "text": summary.join("\n")})];
    content.extend(
        out.images
            .iter()
            .map(|i| json!({"type": "image", "data": i.data, "mimeType": i.mime_type})),
    );
    Ok(ToolResult {
        content,
        details: Some(json!({"model": route.label, "count": n, "paths": saved})),
        ..ToolResult::default()
    })
}

/// Loads an `image` argument against the call's cwd.
pub fn load(input: &str, cwd: &str) -> Result<source::LoadedImage> {
    let roots = Roots::current();
    source::load_image(input, cwd, hostfs::home().as_deref(), &|p| {
        source::probe_host(&roots, p)
    })
    .map_err(Error::msg)
}

/// `image_recognize`.
pub async fn run_recognize(c: &ImageConfig, args: &RecognizeArgs, cwd: &str) -> Result<ToolResult> {
    let r = &c.recognize;
    let image = load(&args.image, cwd)?;
    let prompt = args
        .prompt
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .unwrap_or("Describe this image in detail.");
    let over = args
        .model
        .as_deref()
        .map(str::trim)
        .filter(|m| !m.is_empty());
    let requested = over.unwrap_or(r.model.trim());
    let (text, label) = if !r.base_url.trim().is_empty() && over.is_none() {
        if requested.is_empty() {
            return Err(Error::msg(
                "Set a model id for the custom recognition endpoint in /unipi:settings → Image.",
            ));
        }
        let base = r.base_url.trim();
        (
            recognize::via_endpoint(
                base,
                &r.api_key,
                requested,
                &image,
                prompt,
                &r.system_prompt,
            )
            .await
            .map_err(Error::msg)?,
            format!("{requested} @ {base}"),
        )
    } else {
        let session = control::model().and_then(|m| {
            Some(format!(
                "{}/{}",
                m.get("provider")?.as_str()?,
                m.get("id")?.as_str()?
            ))
        });
        let reference = if requested.is_empty() {
            session.unwrap_or_default()
        } else {
            requested.to_owned()
        };
        if reference.is_empty() {
            return Err(Error::msg(
                "No vision model configured — choose one in /unipi:settings → Image.",
            ));
        }
        let all = control::models().unwrap_or_default();
        let model = resolve_vision_model(&reference, &all).map_err(Error::msg)?;
        let label = format!(
            "{}/{}",
            model
                .get("provider")
                .and_then(Value::as_str)
                .unwrap_or_default(),
            model.get("id").and_then(Value::as_str).unwrap_or_default()
        );
        let reply = oino_extension::models::complete(&recognize::completion_request(
            &label,
            &image,
            prompt,
            &r.system_prompt,
        ))
        .await?;
        (
            recognize::completion_text(&reply).map_err(Error::msg)?,
            label,
        )
    };
    let path = image
        .path
        .as_ref()
        .map_or_else(String::new, |p| format!(" ({p})"));
    let mut details = json!({"model": label, "mimeType": image.mime_type});
    if let Some(p) = &image.path {
        details["path"] = json!(p);
    }
    Ok(ToolResult::text(format!("{text}\n\n— analyzed with {label}{path}")).with_details(details))
}

/// Recognize gating for `model` (`session_start`, `model_select`).
pub fn gate(model: Option<&Value>) {
    let model = model
        .cloned()
        .or_else(control::model)
        .unwrap_or(Value::Null);
    let active = control::active_tools();
    if let Some(next) = models::apply_recognize_gating(&active, &model, RECOGNIZE) {
        let _ = control::set_active_tools(&next);
    }
}

/// Refreshes the OpenRouter image list when stale (never fails).
pub async fn refresh_cache() -> bool {
    let now = timers::now_ms();
    if !Cache::read().stale(now) {
        return false;
    }
    let Ok(resp) = oino_extension::http::get(models::OPENROUTER_MODELS_URL)
        .timeout_ms(15_000)
        .send()
        .await
    else {
        return false;
    };
    if !(200..300).contains(&resp.status()) {
        return false;
    }
    let Ok(json) = resp.json::<Value>().await else {
        return false;
    };
    let list = models::parse_openrouter_image_models(&json);
    if list.is_empty() {
        return false;
    }
    oino_extension::fs::write_json(
        models::CACHE_FILE,
        &Cache {
            updated_at: now,
            models: list,
        },
    )
    .is_ok()
}

/// `image.models` service: picker options (image catalog + host models with modalities).
#[must_use]
pub fn model_options(c: &ImageConfig) -> Value {
    let mut out: Vec<Value> = catalog(&Cache::read(), fal_key(c).is_some())
        .into_iter()
        .map(|m| json!({"value": m.reference(), "label": m.name.clone().unwrap_or_else(|| m.reference()), "input": m.input, "output": m.output}))
        .collect();
    for m in control::models().unwrap_or_default() {
        let (Some(p), Some(id)) = (
            m.get("provider").and_then(Value::as_str),
            m.get("id").and_then(Value::as_str),
        ) else {
            continue;
        };
        let reference = format!("{p}/{id}");
        if out.iter().any(|o| o["value"] == reference.as_str()) {
            continue;
        }
        out.push(json!({"value": reference, "label": m.get("name").cloned().unwrap_or_else(|| json!(reference)), "input": m.get("input").cloned().unwrap_or_else(|| json!(["text"])), "output": ["text"]}));
    }
    Value::Array(out)
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
    fn slugs_and_file_names() {
        assert_eq!(slugify("A red cube!"), "a-red-cube");
        assert_eq!(slugify("!!!"), "image");
        assert_eq!(slugify(&"word ".repeat(20)).len(), 39);
        let png = ImagePayload {
            data: String::new(),
            mime_type: "image/png".into(),
        };
        let jpg = ImagePayload {
            data: String::new(),
            mime_type: "image/jpeg".into(),
        };
        // 2026-09-28T01:02:03Z
        let names = file_names("A red cube!", &[png, jpg], 1_790_557_323_000);
        assert_eq!(
            names,
            [
                "2026-09-28_01-02-03-a-red-cube.png",
                "2026-09-28_01-02-03-a-red-cube-2.jpg"
            ]
        );
    }

    #[test]
    fn output_folder_falls_back_to_the_state_folder() {
        let roots = Roots(vec![
            ("/ext-global".into(), "/h/.oino/state/u/global".into()),
            ("/project".into(), "/w".into()),
        ]);
        assert_eq!(output_dir("", &roots, None, &|_| true), DEFAULT_OUTPUT);
        assert_eq!(
            output_dir("/h/.oino/state/u/global/pics", &roots, None, &|_| true),
            "/ext-global/pics"
        );
        assert_eq!(
            output_dir("/w/assets", &roots, None, &|_| false),
            DEFAULT_OUTPUT
        );
        assert_eq!(
            output_dir("/elsewhere", &roots, None, &|_| true),
            DEFAULT_OUTPUT
        );
    }

    #[test]
    fn tool_definitions() {
        let defs = tool_defs();
        assert_eq!(
            defs.iter().map(|d| d.0).collect::<Vec<_>>(),
            [GENERATE, EDIT, RECOGNIZE]
        );
        assert_eq!(defs[1].5["required"], json!(["prompt", "image"]));
        assert_eq!(defs[0].4.len(), 4);
    }
}
