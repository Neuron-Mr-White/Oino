//! Image generation transports (unipi `src/image/transports.ts`): three wire formats, one
//! result shape. Requests are built and responses parsed purely ([`build`], [`parse`]); the
//! async [`generate`] runs them over host HTTP.
//!
//! | api | request | images |
//! |---|---|---|
//! | `openrouter` | `POST {base}/chat/completions` with `modalities` | data URLs in `choices[0].message.images` |
//! | `fal` | `POST https://fal.run/<model>`, `sync_mode` (edits: `image_urls`) | `images[].url` |
//! | `openai-images` | `POST {base}/images/generations` (edits: `image` array) | `data[].b64_json` / `url` |

use base64::Engine as _;
use serde_json::{Value, json};

use super::models::{Cred, Route};
use crate::settings::ImageApi;

/// OpenRouter API base.
pub const OPENROUTER_BASE: &str = "https://openrouter.ai/api/v1";
/// fal REST base.
pub const FAL_BASE: &str = "https://fal.run";
/// Generation time box.
pub const TIMEOUT_MS: u32 = 240_000;

/// One image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImagePayload {
    /// Base64.
    pub data: String,
    /// Media type.
    pub mime_type: String,
}

/// An image in a response: inline or to download.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImageRef {
    /// Inline data.
    Inline(ImagePayload),
    /// Remote URL.
    Url(String),
}

/// A prepared HTTP call.
#[derive(Clone, Debug, PartialEq)]
pub struct Call {
    /// URL.
    pub url: String,
    /// Headers.
    pub headers: Vec<(String, String)>,
    /// JSON body.
    pub body: Value,
}

/// Parsed response.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Parsed {
    /// Images (some may still need downloading).
    pub images: Vec<ImageRef>,
    /// Model text / revised prompts.
    pub text: String,
}

/// Final result.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GenerateResponse {
    /// Images.
    pub images: Vec<ImagePayload>,
    /// Model text.
    pub text: String,
}

fn data_url(img: &ImagePayload) -> String {
    format!("data:{};base64,{}", img.mime_type, img.data)
}

/// `data:image/png;base64,…` → payload.
#[must_use]
pub fn parse_data_url(url: &str) -> Option<ImagePayload> {
    let rest = url.strip_prefix("data:")?;
    let (meta, data) = rest.split_once(',')?;
    if data.is_empty() {
        return None;
    }
    let mime = meta
        .split(';')
        .next()
        .filter(|m| !m.is_empty())
        .unwrap_or("image/png");
    Some(ImagePayload {
        data: data.to_owned(),
        mime_type: mime.to_owned(),
    })
}

fn join(base: &str, suffix: &str) -> String {
    format!(
        "{}/{}",
        base.trim_end_matches('/'),
        suffix.trim_start_matches('/')
    )
}

fn auth_headers(cred: &Cred, fal: bool) -> Vec<(String, String)> {
    let mut h = vec![("content-type".to_owned(), "application/json".to_owned())];
    match cred {
        Cred::Key(k) if fal => h.push(("authorization".into(), format!("Key {k}"))),
        Cred::Key(k) => h.push(("authorization".into(), format!("Bearer {k}"))),
        Cred::Headers(hs) => h.extend(hs.iter().cloned()),
    }
    h
}

fn cred_empty(c: &Cred) -> bool {
    match c {
        Cred::Key(k) => k.is_empty(),
        Cred::Headers(h) => h.is_empty(),
    }
}

/// Builds the request.
///
/// # Errors
///
/// Empty prompt, missing key or endpoint.
pub fn build(route: &Route, prompt: &str, images: &[ImagePayload]) -> Result<Call, String> {
    if prompt.trim().is_empty() {
        return Err("A non-empty prompt is required.".into());
    }
    if cred_empty(&route.cred) {
        return Err("No API key for this image model.".into());
    }
    match route.api {
        ImageApi::OpenRouter => {
            let mut content = vec![json!({"type": "text", "text": prompt})];
            content.extend(
                images
                    .iter()
                    .map(|i| json!({"type": "image_url", "image_url": {"url": data_url(i)}})),
            );
            let output = route
                .output
                .clone()
                .filter(|o| o.iter().any(|m| m == "image"))
                .unwrap_or_else(|| vec!["image".into()]);
            let base = if route.base_url.is_empty() {
                OPENROUTER_BASE
            } else {
                &route.base_url
            };
            Ok(Call {
                url: join(base, "chat/completions"),
                headers: auth_headers(&route.cred, false),
                body: json!({"model": route.model_id, "messages": [{"role": "user", "content": content}], "modalities": output}),
            })
        }
        ImageApi::Fal => {
            let mut body = json!({"prompt": prompt, "sync_mode": true, "num_images": 1});
            if !images.is_empty() {
                body["image_urls"] = json!(images.iter().map(data_url).collect::<Vec<_>>());
            }
            let base = if route.base_url.is_empty() {
                FAL_BASE
            } else {
                &route.base_url
            };
            Ok(Call {
                url: join(base, &route.model_id),
                headers: auth_headers(&route.cred, true),
                body,
            })
        }
        ImageApi::OpenAiImages => {
            if route.base_url.is_empty() {
                return Err(format!(
                    "No endpoint for {}: set a custom endpoint in /settings → Image.",
                    route.model_id
                ));
            }
            let mut body = json!({"model": route.model_id, "prompt": prompt});
            if !images.is_empty() {
                body["image"] = json!(images.iter().map(data_url).collect::<Vec<_>>());
            }
            Ok(Call {
                url: join(&route.base_url, "images/generations"),
                headers: auth_headers(&route.cred, false),
                body,
            })
        }
    }
}

/// Provider error text (`<status>: <detail>`).
#[must_use]
pub fn read_error(status: u16, body: &[u8]) -> String {
    let text = String::from_utf8_lossy(body);
    let mut detail: String = text.chars().take(400).collect();
    if let Ok(v) = serde_json::from_str::<Value>(&text) {
        if let Some(s) = v.get("error").and_then(Value::as_str) {
            s.clone_into(&mut detail);
        } else if let Some(s) = v
            .get("error")
            .and_then(|e| e.get("message"))
            .and_then(Value::as_str)
        {
            s.clone_into(&mut detail);
        } else if let Some(s) = v.get("message").and_then(Value::as_str) {
            s.clone_into(&mut detail);
        } else if let Some(d) = v.get("detail").filter(|d| !d.is_null()) {
            detail = d.as_str().map_or_else(
                || d.to_string().chars().take(400).collect(),
                ToOwned::to_owned,
            );
        }
    }
    if detail.is_empty() {
        status.to_string()
    } else {
        format!("{status}: {detail}")
    }
}

fn push_url(images: &mut Vec<ImageRef>, url: &str) {
    images
        .push(parse_data_url(url).map_or_else(|| ImageRef::Url(url.to_owned()), ImageRef::Inline));
}

/// Parses a response.
///
/// # Errors
///
/// HTTP errors and provider-reported errors.
pub fn parse(api: ImageApi, status: u16, body: &[u8], prompt: &str) -> Result<Parsed, String> {
    if !(200..300).contains(&status) {
        return Err(read_error(status, body));
    }
    let v: Value = serde_json::from_slice(body)
        .map_err(|e| format!("The image endpoint returned invalid JSON: {e}"))?;
    if let Some(m) = v
        .get("error")
        .and_then(|e| e.get("message"))
        .and_then(Value::as_str)
    {
        return Err(m.to_owned());
    }
    let mut out = Parsed::default();
    match api {
        ImageApi::OpenRouter => {
            let msg = v.pointer("/choices/0/message");
            for item in msg
                .and_then(|m| m.get("images"))
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                if let Some(u) = item.pointer("/image_url/url").and_then(Value::as_str) {
                    push_url(&mut out.images, u);
                }
            }
            msg.and_then(|m| m.get("content"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .clone_into(&mut out.text);
        }
        ImageApi::Fal => {
            let list: Vec<&Value> = match v.get("images").and_then(Value::as_array) {
                Some(a) => a.iter().collect(),
                None => v.get("image").into_iter().collect(),
            };
            for item in list {
                if let Some(u) = item.get("url").and_then(Value::as_str) {
                    push_url(&mut out.images, u);
                }
            }
            v.get("description")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .clone_into(&mut out.text);
        }
        ImageApi::OpenAiImages => {
            let mut notes = Vec::new();
            for item in v
                .get("data")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let declared = item
                    .get("media_type")
                    .or_else(|| item.get("mime_type"))
                    .and_then(Value::as_str);
                if let Some(b) = item.get("b64_json").and_then(Value::as_str) {
                    out.images.push(ImageRef::Inline(ImagePayload {
                        data: b.to_owned(),
                        mime_type: declared.unwrap_or("image/png").to_owned(),
                    }));
                } else if let Some(u) = item.get("url").and_then(Value::as_str) {
                    push_url(&mut out.images, u);
                }
                if let Some(r) = item
                    .get("revised_prompt")
                    .and_then(Value::as_str)
                    .filter(|r| *r != prompt)
                {
                    notes.push(format!("Revised prompt: {r}"));
                }
            }
            out.text = notes.join("\n");
        }
    }
    Ok(out)
}

/// The no-image error.
#[must_use]
pub fn no_image_error(text: &str) -> String {
    if text.is_empty() {
        "The model returned no image.".into()
    } else {
        format!(
            "The model returned no image. It said: {}",
            text.chars().take(300).collect::<String>()
        )
    }
}

/// Runs one generation/edit over host HTTP; fails unless at least one image came back.
///
/// # Errors
///
/// A message worth showing the agent.
pub async fn generate(
    route: &Route,
    prompt: &str,
    images: &[ImagePayload],
) -> Result<GenerateResponse, String> {
    let call = build(route, prompt, images)?;
    let mut req = oino_extension::http::post(&call.url).timeout_ms(TIMEOUT_MS);
    for (k, v) in &call.headers {
        req = req.header(k, v);
    }
    let resp = req
        .body(call.body.to_string())
        .send()
        .await
        .map_err(|e| format!("Image request failed: {e}"))?;
    let status = resp.status();
    let body = resp.bytes().await;
    let parsed = parse(route.api, status, &body, prompt)?;
    let mut out = GenerateResponse {
        images: Vec::new(),
        text: parsed.text,
    };
    for r in parsed.images {
        match r {
            ImageRef::Inline(p) => out.images.push(p),
            ImageRef::Url(u) => out.images.push(download(&u).await?),
        }
    }
    if out.images.is_empty() {
        return Err(no_image_error(&out.text));
    }
    Ok(out)
}

async fn download(url: &str) -> Result<ImagePayload, String> {
    let resp = oino_extension::http::get(url)
        .timeout_ms(TIMEOUT_MS)
        .send()
        .await
        .map_err(|e| format!("Could not download the generated image: {e}"))?;
    if !(200..300).contains(&resp.status()) {
        return Err(format!(
            "Could not download the generated image: {}",
            resp.status()
        ));
    }
    let mime = resp
        .header("content-type")
        .and_then(|c| c.split(';').next())
        .filter(|m| !m.is_empty())
        .unwrap_or("image/png")
        .to_owned();
    let bytes = resp.bytes().await;
    Ok(ImagePayload {
        data: base64::engine::general_purpose::STANDARD.encode(bytes),
        mime_type: mime,
    })
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

    const PNG: &str = "ZmFrZS1wbmc=";

    fn route(api: ImageApi, base: &str, key: &str, model: &str) -> Route {
        Route {
            api,
            base_url: base.into(),
            cred: Cred::Key(key.into()),
            model_id: model.into(),
            label: model.into(),
            output: Some(vec!["image".into()]),
        }
    }

    fn header<'a>(c: &'a Call, k: &str) -> Option<&'a str> {
        c.headers
            .iter()
            .find(|(h, _)| h == k)
            .map(|(_, v)| v.as_str())
    }

    #[test]
    fn openrouter_chat_completions_with_modalities() {
        let c = build(
            &route(
                ImageApi::OpenRouter,
                "",
                "k",
                "black-forest-labs/flux.2-klein-4b",
            ),
            "a cube",
            &[],
        )
        .unwrap();
        assert_eq!(c.url, "https://openrouter.ai/api/v1/chat/completions");
        assert_eq!(c.body["modalities"], json!(["image"]));
        assert_eq!(header(&c, "authorization"), Some("Bearer k"));
        let body = json!({"choices": [{"message": {"content": "", "images": [{"image_url": {"url": format!("data:image/png;base64,{PNG}")}}]}}]}).to_string();
        let p = parse(ImageApi::OpenRouter, 200, body.as_bytes(), "a cube").unwrap();
        assert_eq!(
            p.images,
            vec![ImageRef::Inline(ImagePayload {
                data: PNG.into(),
                mime_type: "image/png".into()
            })]
        );
    }

    #[test]
    fn openrouter_edit_sends_the_source_as_image_url() {
        let src = ImagePayload {
            data: PNG.into(),
            mime_type: "image/png".into(),
        };
        let c = build(
            &route(ImageApi::OpenRouter, "", "k", "m"),
            "make it blue",
            &[src],
        )
        .unwrap();
        let types: Vec<&str> = c.body["messages"][0]["content"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["type"].as_str().unwrap())
            .collect();
        assert_eq!(types, ["text", "image_url"]);
        let mut text_only = route(ImageApi::OpenRouter, "", "k", "m");
        text_only.output = Some(vec!["text".into()]);
        assert_eq!(
            build(&text_only, "p", &[]).unwrap().body["modalities"],
            json!(["image"])
        );
    }

    #[test]
    fn fal_key_auth_sync_mode_and_image_urls() {
        let src = ImagePayload {
            data: PNG.into(),
            mime_type: "image/png".into(),
        };
        let c = build(
            &route(ImageApi::Fal, "", "fk", "fal-ai/flux-2/klein/4b/edit"),
            "p",
            &[src],
        )
        .unwrap();
        assert_eq!(c.url, "https://fal.run/fal-ai/flux-2/klein/4b/edit");
        assert_eq!(header(&c, "authorization"), Some("Key fk"));
        assert_eq!(c.body["sync_mode"], true);
        assert_eq!(
            c.body["image_urls"],
            json!([format!("data:image/png;base64,{PNG}")])
        );
        let body = json!({"images": [{"url": format!("data:image/jpeg;base64,{PNG}"), "content_type": "image/jpeg"}]}).to_string();
        let p = parse(ImageApi::Fal, 200, body.as_bytes(), "p").unwrap();
        assert!(matches!(&p.images[0], ImageRef::Inline(i) if i.mime_type == "image/jpeg"));
        let single = parse(
            ImageApi::Fal,
            200,
            br#"{"image": {"url": "https://cdn/x.png"}, "description": "d"}"#,
            "p",
        )
        .unwrap();
        assert_eq!(
            single.images,
            vec![ImageRef::Url("https://cdn/x.png".into())]
        );
        assert_eq!(single.text, "d");
    }

    #[test]
    fn openai_images_b64_and_revised_prompts() {
        let c = build(
            &route(ImageApi::OpenAiImages, "https://gw/v1/", "k", "flux"),
            "p",
            &[],
        )
        .unwrap();
        assert_eq!(c.url, "https://gw/v1/images/generations");
        let body = json!({"data": [{"b64_json": PNG, "media_type": "image/webp", "revised_prompt": "better"}]}).to_string();
        let p = parse(ImageApi::OpenAiImages, 200, body.as_bytes(), "p").unwrap();
        assert!(matches!(&p.images[0], ImageRef::Inline(i) if i.mime_type == "image/webp"));
        assert!(p.text.contains("Revised prompt: better"));
        assert!(
            build(&route(ImageApi::OpenAiImages, "", "k", "flux"), "p", &[])
                .unwrap_err()
                .contains("No endpoint")
        );
    }

    #[test]
    fn errors() {
        assert_eq!(
            parse(ImageApi::Fal, 404, br#"{"detail": "bad model"}"#, "p").unwrap_err(),
            "404: bad model"
        );
        assert_eq!(
            read_error(500, br#"{"error": {"message": "boom"}}"#),
            "500: boom"
        );
        assert_eq!(read_error(502, b""), "502");
        assert_eq!(
            read_error(400, br#"{"detail": [{"msg": "x"}]}"#),
            "400: [{\"msg\":\"x\"}]"
        );
        let said = parse(
            ImageApi::OpenRouter,
            200,
            br#"{"choices": [{"message": {"content": "Here is your image!"}}]}"#,
            "p",
        )
        .unwrap();
        assert!(said.images.is_empty());
        assert_eq!(
            no_image_error(&said.text),
            "The model returned no image. It said: Here is your image!"
        );
        assert!(
            build(&route(ImageApi::OpenRouter, "", "", "m"), "p", &[])
                .unwrap_err()
                .contains("No API key")
        );
        assert!(
            build(&route(ImageApi::OpenRouter, "", "k", "m"), "  ", &[])
                .unwrap_err()
                .contains("non-empty prompt")
        );
        assert_eq!(
            parse(
                ImageApi::OpenRouter,
                200,
                br#"{"error": {"message": "quota"}}"#,
                "p"
            )
            .unwrap_err(),
            "quota"
        );
    }

    #[test]
    fn host_credential_headers_pass_through() {
        let mut r = route(ImageApi::OpenRouter, "", "", "m");
        r.cred = Cred::Headers(vec![("authorization".into(), "Bearer host".into())]);
        assert_eq!(
            header(&build(&r, "p", &[]).unwrap(), "authorization"),
            Some("Bearer host")
        );
        assert_eq!(
            parse_data_url("data:;base64,AAA")
                .map(|p| p.mime_type)
                .as_deref(),
            Some("image/png")
        );
        assert_eq!(parse_data_url("https://x"), None);
    }
}
