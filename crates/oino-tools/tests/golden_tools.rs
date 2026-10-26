//! Golden tests: every built-in tool against pi 1.4.0's real implementation.
//!
//! Cases: `scripts/pi-oracle/core-runtime/tools/cases.json`; pi results:
//! `tests/fixtures/tools.json` (regenerate with `node run.mjs` in that directory). Both sides
//! normalise the case directory to `$CWD`, temp logs to `$TMPLOG`, wall time to `0`, image
//! bytes to `$SOURCE`/`$ENCODED` (+ decoded dimensions) and long strings to a digest.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stderr,
    reason = "test harness"
)]

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use oino_agent::{ToolInvocation, UpdateSink};
use oino_ai::{ImageResize, JsonObject, UserContent};
use oino_tools::{
    FileMutationQueue, ImageOptions, ModelImageInfo, SessionEnv, ShellOptions, ToolConfig,
    ToolContext, tool_by_name,
};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

const CASES: &str = include_str!("../../../scripts/pi-oracle/core-runtime/tools/cases.json");
const GOLDEN: &str = include_str!("fixtures/tools.json");

struct CaseContext {
    cwd: String,
    model: Option<ModelImageInfo>,
    session: SessionEnv,
}

impl ToolContext for CaseContext {
    fn cwd(&self) -> Option<String> {
        Some(self.cwd.clone())
    }

    fn model_images(&self) -> Option<ModelImageInfo> {
        self.model.clone()
    }

    fn session_env(&self) -> Option<SessionEnv> {
        Some(self.session.clone())
    }
}

fn pixels(width: u32, height: u32) -> image::RgbaImage {
    image::RgbaImage::from_fn(width, height, |x, y| {
        image::Rgba([
            u8::try_from((x * 7) % 256).unwrap(),
            u8::try_from((y * 13) % 256).unwrap(),
            u8::try_from((x + y) % 256).unwrap(),
            255,
        ])
    })
}

fn bmp(width: u32, height: u32) -> Vec<u8> {
    let row = (width * 3).div_ceil(4) * 4;
    let size = 54 + row * height;
    let mut buf = vec![0_u8; size as usize];
    buf[..2].copy_from_slice(b"BM");
    buf[2..6].copy_from_slice(&size.to_le_bytes());
    buf[10..14].copy_from_slice(&54_u32.to_le_bytes());
    buf[14..18].copy_from_slice(&40_u32.to_le_bytes());
    buf[18..22].copy_from_slice(&width.to_le_bytes());
    buf[22..26].copy_from_slice(&height.to_le_bytes());
    buf[26..28].copy_from_slice(&1_u16.to_le_bytes());
    buf[28..30].copy_from_slice(&24_u16.to_le_bytes());
    buf[34..38].copy_from_slice(&(row * height).to_le_bytes());
    let raw = pixels(width, height);
    for y in 0..height {
        for x in 0..width {
            let px = raw.get_pixel(x, height - 1 - y).0;
            let at = (54 + y * row + x * 3) as usize;
            buf[at] = px[2];
            buf[at + 1] = px[1];
            buf[at + 2] = px[0];
        }
    }
    buf
}

fn image_bytes(spec: &Value) -> Vec<u8> {
    let width = u32::try_from(spec["width"].as_u64().unwrap()).unwrap();
    let height = u32::try_from(spec["height"].as_u64().unwrap()).unwrap();
    match spec["image"].as_str().unwrap() {
        "bmp" => bmp(width, height),
        "jpeg" => {
            let mut out = Vec::new();
            let rgb = image::DynamicImage::ImageRgba8(pixels(width, height)).to_rgb8();
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 80)
                .encode_image(&rgb)
                .unwrap();
            out
        }
        _ => {
            let mut out = Cursor::new(Vec::new());
            pixels(width, height)
                .write_to(&mut out, image::ImageFormat::Png)
                .unwrap();
            out.into_inner()
        }
    }
}

/// Mirrors `content()` in run.mjs.
fn content(spec: &Value) -> Vec<u8> {
    if let Some(text) = spec.as_str() {
        return text.as_bytes().to_vec();
    }
    if let Some(b64) = spec["b64"].as_str() {
        return BASE64.decode(b64).unwrap();
    }
    if let Some(count) = spec["lines"].as_u64() {
        let prefix = spec["prefix"].as_str().unwrap_or("");
        let pad = spec["pad"]
            .as_u64()
            .map(|pad| usize::try_from(pad).unwrap());
        let lines: Vec<String> = (1..=count)
            .map(|i| match pad {
                Some(pad) => format!("{prefix}{i:0>pad$}"),
                None => format!("{prefix}{i}"),
            })
            .collect();
        return format!("{}\n", lines.join("\n")).into_bytes();
    }
    if let Some(repeat) = spec["repeat"].as_str() {
        return repeat
            .repeat(usize::try_from(spec["count"].as_u64().unwrap()).unwrap())
            .into_bytes();
    }
    if let Some(parts) = spec["concat"].as_array() {
        return parts.iter().flat_map(content).collect();
    }
    image_bytes(spec)
}

fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

fn fnv1a(text: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// Mirrors `compact()` in run.mjs.
fn compact(value: Value) -> Value {
    match value {
        Value::String(text) if utf16_len(&text) > 4096 => {
            let units: Vec<u16> = text.encode_utf16().collect();
            json!({
                "$long": text.len(),
                "head": String::from_utf16_lossy(&units[..200]),
                "tail": String::from_utf16_lossy(&units[units.len() - 200..]),
                "fnv": fnv1a(&text),
            })
        }
        Value::Array(items) => Value::Array(items.into_iter().map(compact).collect()),
        Value::Object(map) => {
            Value::Object(map.into_iter().map(|(k, v)| (k, compact(v))).collect())
        }
        other => other,
    }
}

fn image_dims(data: &str) -> Value {
    let bytes = BASE64.decode(data).unwrap();
    match image::load_from_memory(&bytes) {
        Ok(img) => json!({"width": img.width(), "height": img.height()}),
        Err(_) => Value::Null,
    }
}

fn setup(case: &Value, dir: &Path, golden: &Value) {
    if dir.exists() {
        std::fs::remove_dir_all(dir).unwrap();
    }
    std::fs::create_dir_all(dir).unwrap();
    for sub in case["dirs"].as_array().into_iter().flatten() {
        std::fs::create_dir_all(dir.join(sub.as_str().unwrap())).unwrap();
    }
    for (rel, spec) in case["files"].as_object().into_iter().flatten() {
        let path = dir.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let bytes = match golden["materialized"][rel].as_str() {
            Some(b64) => BASE64.decode(b64).unwrap(),
            None => content(spec),
        };
        std::fs::write(path, bytes).unwrap();
    }
    #[cfg(unix)]
    for (rel, target) in case["symlinks"].as_object().into_iter().flatten() {
        std::os::unix::fs::symlink(target.as_str().unwrap(), dir.join(rel)).unwrap();
    }
}

fn config(case: &Value, dir: &str, agent_dir: &str) -> ToolConfig {
    let ctx = &case["ctx"];
    let cwd = match ctx["cwd"].as_str() {
        Some(sub) => format!("{dir}/{sub}"),
        None => dir.to_owned(),
    };
    let model = ctx["model"].as_object().map(|model| {
        let accepts_images = model["input"]
            .as_array()
            .is_some_and(|input| input.iter().any(|m| m == "image"));
        let resize: Option<ImageResize> = model
            .get("inputLimits")
            .map(|limits| serde_json::from_value(limits["images"]["resize"].clone()).unwrap());
        ModelImageInfo {
            accepts_images,
            resize,
        }
    });
    let session = if ctx["session"].as_bool() == Some(true) {
        SessionEnv {
            session_id: "sess-1".into(),
            session_file: Some("/s/file.jsonl".into()),
            provider: Some("prov".into()),
            model: Some("mod-1".into()),
            reasoning_level: Some("high".into()),
        }
    } else {
        SessionEnv {
            session_id: "sess-1".into(),
            ..SessionEnv::default()
        }
    };
    let options = &case["options"];
    let mut config = ToolConfig::new(dir, agent_dir);
    config.context = Arc::new(CaseContext {
        cwd,
        model,
        session,
    });
    config.images = ImageOptions {
        auto_resize: options["autoResizeImages"].as_bool().unwrap_or(true),
        fallback_resize: options
            .get("resizeOptions")
            .map(|resize| serde_json::from_value(resize.clone()).unwrap()),
    };
    config.shell = ShellOptions {
        shell_path: options["shellPath"].as_str().map(str::to_owned),
        command_prefix: options["commandPrefix"].as_str().map(str::to_owned),
        expose_session_environment: true,
    };
    config.mutation_queue = FileMutationQueue::default();
    let mut env: Vec<(String, String)> = std::env::vars().collect();
    for (key, value) in case["env"].as_object().into_iter().flatten() {
        env.retain(|(name, _)| name != key);
        env.push((key.clone(), value.as_str().unwrap().to_owned()));
    }
    config.process_env = Some(env);
    config
}

async fn run_case(case: &Value, golden: &Value, root: &Path) -> Value {
    let name = case["name"].as_str().unwrap();
    let dir = root.join("cases").join(name);
    let agent_dir = root.join("agent");
    setup(case, &dir, golden);
    let dir_str = dir.to_string_lossy().into_owned();
    let agent_str = agent_dir.to_string_lossy().into_owned();
    let config = config(case, &dir_str, &agent_str);
    let tool = tool_by_name(case["tool"].as_str().unwrap(), config.clone()).unwrap();
    let raw: JsonObject =
        serde_json::from_str(&case["args"].to_string().replace("$CWD", &dir_str)).unwrap();
    let args = tool.prepare_arguments(raw);
    let cancel = CancellationToken::new();
    if case["abort"] == "before" {
        cancel.cancel();
    }
    if let Some(ms) = case["abortAfterMs"].as_u64() {
        let cancel = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
            cancel.cancel();
        });
    }
    let call = ToolInvocation {
        tool_call_id: format!("call-{name}"),
        args: args.clone(),
        parent_tool_call_id: None,
    };
    let mut record = match tool.execute(call, UpdateSink::discard(), cancel).await {
        Ok(result) => {
            let mut record = json!({
                "content": result.content,
                "details": result.details.unwrap_or(Value::Null),
                "structuredContent": result.structured_content.unwrap_or(Value::Null),
                "isError": result.is_error,
            });
            let dims: Vec<Value> = result
                .content
                .iter()
                .filter_map(|block| match block {
                    UserContent::Image(image) => Some(image_dims(&image.data)),
                    UserContent::Text(_) => None,
                })
                .collect();
            if !dims.is_empty() {
                record["imageDims"] = Value::Array(dims);
            }
            record
        }
        Err(error) => json!({"error": error.0}),
    };
    mark_images(&mut record, &config, &args);
    if let Some(seconds) = record
        .get_mut("structuredContent")
        .and_then(|structured| structured.get_mut("wall_time_seconds"))
    {
        *seconds = json!(0);
    }
    let mut files = serde_json::Map::new();
    for rel in case["check"].as_array().into_iter().flatten() {
        let path = dir.join(rel.as_str().unwrap());
        let value =
            std::fs::read(&path).map_or(Value::Null, |bytes| Value::String(BASE64.encode(bytes)));
        files.insert(rel.as_str().unwrap().to_owned(), value);
    }
    let text = json!({"result": record, "files": files})
        .to_string()
        .replace(&json_inner(&dir_str), "$CWD")
        .replace(&json_inner(&format!("{agent_str}/tmp")), "$TMP");
    let text = regex::Regex::new(r"\$TMP/oino-bash-[0-9a-f]{16}\.log")
        .unwrap()
        .replace_all(&text, "$$TMPLOG")
        .into_owned();
    compact(serde_json::from_str(&text).unwrap())
}

fn json_inner(text: &str) -> String {
    let quoted = Value::String(text.to_owned()).to_string();
    quoted[1..quoted.len() - 1].to_owned()
}

fn mark_images(record: &mut Value, config: &ToolConfig, args: &JsonObject) {
    let has_image = record["content"]
        .as_array()
        .is_some_and(|blocks| blocks.iter().any(|block| block["type"] == "image"));
    if !has_image {
        return;
    }
    let path = args["path"].as_str().unwrap().trim_start_matches('@');
    let cwd = config.context.cwd().unwrap();
    let source = BASE64.encode(std::fs::read(Path::new(&cwd).join(path)).unwrap());
    let marker = |data: &mut Value| {
        let same = data.as_str() == Some(source.as_str());
        *data = json!(if same { "$SOURCE" } else { "$ENCODED" });
    };
    for block in record["content"].as_array_mut().unwrap() {
        if block["type"] == "image" {
            marker(&mut block["data"]);
        }
    }
    if record["structuredContent"]["type"] == "image" {
        marker(&mut record["structuredContent"]["data"]);
    }
}

/// Sorts the result lines of the first text block (notices after `\n\n[` stay last).
fn sort_lines(record: &mut Value) {
    let Some(text) = record["result"]["content"][0]["text"].as_str() else {
        return;
    };
    let (body, notice) = match text.find("\n\n[") {
        Some(at) => (&text[..at], &text[at..]),
        None => (text, ""),
    };
    let mut lines: Vec<&str> = body.split('\n').collect();
    lines.sort_unstable();
    record["result"]["content"][0]["text"] = json!(format!("{}{notice}", lines.join("\n")));
}

fn count_lines(record: &mut Value) {
    let Some(text) = record["result"]["content"][0]["text"].as_str() else {
        return;
    };
    let (body, notice) = match text.find("\n\n[") {
        Some(at) => (&text[..at], &text[at..]),
        None => (text, ""),
    };
    let count = body.split('\n').count();
    record["result"]["content"][0]["text"] = json!(format!("<{count} lines>{notice}"));
}

fn comparable(mut record: Value, mode: Option<&str>) -> Value {
    match mode {
        Some("sortedLines") => sort_lines(&mut record),
        Some("count") => count_lines(&mut record),
        _ => {}
    }
    record
}

fn root_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("golden-tools")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tools_match_pi() {
    let cases: Vec<Value> = serde_json::from_str(CASES).unwrap();
    let golden: Vec<Value> = serde_json::from_str(GOLDEN).unwrap();
    assert_eq!(cases.len(), golden.len(), "regenerate tools.json");
    let root = root_dir();
    let filter = std::env::var("GOLDEN_CASE").ok();
    let mut failures = Vec::new();
    let mut compared = 0;
    for (case, expected) in cases.iter().zip(&golden) {
        let name = case["name"].as_str().unwrap();
        assert_eq!(expected["name"], name);
        if filter
            .as_deref()
            .is_some_and(|filter| !name.contains(filter))
        {
            continue;
        }
        if cfg!(windows) && case["tool"] == "bash" {
            continue;
        }
        compared += 1;
        let actual = run_case(case, expected, &root).await;
        let mode = case["compare"].as_str();
        let expected = comparable(
            json!({"result": expected["result"], "files": expected["files"]}),
            mode,
        );
        let actual = comparable(actual, mode);
        if actual != expected {
            failures.push(format!(
                "case {name}\n  pi:   {}\n  oino: {}",
                serde_json::to_string(&expected).unwrap(),
                serde_json::to_string(&actual).unwrap()
            ));
        }
    }
    assert!(compared > 0, "no case selected");
    eprintln!("compared {compared} golden cases");
    assert!(
        failures.is_empty(),
        "{} golden mismatches:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn metadata_matches_pi() {
    let meta: Value = serde_json::from_str(include_str!("fixtures/meta.json")).unwrap();
    let config = ToolConfig::new("/cwd", "/agent");
    for (name, expected) in meta.as_object().unwrap() {
        let info = oino_tools::BuiltinToolName::all()
            .into_iter()
            .chain([oino_tools::BuiltinToolName::Powershell])
            .find(|tool| tool.as_str() == name)
            .map(|tool| oino_tools::builtin_tool_info(tool, true))
            .unwrap();
        let guidelines = info.prompt_guidelines.as_ref().map(|lines| {
            lines
                .iter()
                .map(|line| {
                    // D13: pi's PI_* guideline.
                    line.replace(
                        "You can inspect OINO_* environment variables (also exported as PI_*)",
                        "You can inspect PI_* environment variables",
                    )
                })
                .collect::<Vec<_>>()
        });
        let actual = json!({
            "name": info.name.as_str(),
            "label": info.label,
            "description": info.description,
            "promptSnippet": info.prompt_snippet,
            "promptGuidelines": guidelines,
            "parameters": info.parameters,
            "outputSchema": info.output_schema,
            "constrainedSampling": info.constrained_sampling,
            "executionMode": Value::Null,
            "hasPrepareArguments": name == "edit",
        });
        assert_eq!(&actual, expected, "metadata of {name}");
        assert_eq!(
            serde_json::to_string(&info.parameters).unwrap(),
            serde_json::to_string(&expected["parameters"]).unwrap(),
            "key order of {name} parameters"
        );
    }
    // Every registry tool exposes the same declaration as its info.
    for tool in oino_tools::builtin_tools(&config) {
        let meta = tool.meta();
        let info = oino_tools::builtin_tool_info(
            oino_tools::BuiltinToolName::parse(&meta.name).unwrap(),
            true,
        );
        assert_eq!(meta.description, info.description);
        assert_eq!(meta.parameters, info.parameters);
    }
}
