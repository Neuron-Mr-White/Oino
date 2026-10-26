//! pi `file-processor.ts` + `initial-message.ts`: `@file` arguments become text blocks and
//! image attachments of the first message; piped stdin is prepended.

use std::fmt::Write as _;
use std::path::Path;

use base64::Engine as _;
use oino_ai::ImageContent;

/// Text and images collected from `@file` arguments.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FileContent {
    /// `<file name="…">…</file>` blocks.
    pub text: String,
    /// Image attachments.
    pub images: Vec<ImageContent>,
}

/// pi `processFileArguments`; errors carry pi's text (the caller prints it and exits 1).
///
/// # Errors
///
/// `Error: File not found: {path}` and `Error: Could not read file {path}: {message}`.
pub fn process_file_arguments(file_args: &[String], cwd: &Path) -> Result<FileContent, String> {
    let mut content = FileContent::default();
    let cwd_text = cwd.display().to_string();
    for file_arg in file_args {
        let absolute = oino_tools::path_utils::resolve_to_cwd(file_arg, &cwd_text)
            .unwrap_or_else(|_| cwd.join(file_arg).display().to_string());
        let path = Path::new(&absolute);
        let Ok(metadata) = std::fs::metadata(path) else {
            return Err(format!("Error: File not found: {absolute}"));
        };
        if metadata.len() == 0 {
            continue;
        }
        let bytes = std::fs::read(path)
            .map_err(|e| format!("Error: Could not read file {absolute}: {e}"))?;
        let sniff = &bytes[..bytes.len().min(oino_tools::image::IMAGE_TYPE_SNIFF_BYTES)];
        if let Some(mime_type) = oino_tools::image::detect_supported_image_mime_type(sniff) {
            content.images.push(ImageContent {
                data: base64::engine::general_purpose::STANDARD.encode(&bytes),
                mime_type: mime_type.to_owned(),
            });
            let _ = writeln!(content.text, "<file name=\"{absolute}\"></file>");
        } else {
            let text = String::from_utf8(bytes)
                .map_err(|e| format!("Error: Could not read file {absolute}: {e}"))?;
            let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
            let _ = writeln!(content.text, "<file name=\"{absolute}\">\n{text}\n</file>");
        }
    }
    Ok(content)
}

/// The first prompt of a non-interactive run (pi `buildInitialMessage`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InitialMessage {
    /// Text (stdin + files + first message), `None` when all are absent.
    pub text: Option<String>,
    /// Image attachments.
    pub images: Vec<ImageContent>,
}

/// pi `buildInitialMessage`: consumes the first positional message.
#[must_use]
pub fn build_initial_message(
    messages: &mut Vec<String>,
    files: FileContent,
    stdin: Option<String>,
) -> InitialMessage {
    let mut parts = Vec::new();
    if let Some(stdin) = stdin {
        parts.push(stdin);
    }
    if !files.text.is_empty() {
        parts.push(files.text);
    }
    if !messages.is_empty() {
        parts.push(messages.remove(0));
    }
    InitialMessage {
        text: (!parts.is_empty()).then(|| parts.concat()),
        images: files.images,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_become_tagged_text_and_images() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.md"), "\u{feff}hello").unwrap();
        std::fs::write(dir.path().join("empty.txt"), "").unwrap();
        std::fs::write(
            dir.path().join("p.gif"),
            b"GIF89a\x01\x00\x01\x00\x00\x00\x00;",
        )
        .unwrap();
        let files = process_file_arguments(
            &["a.md".into(), "empty.txt".into(), "p.gif".into()],
            dir.path(),
        )
        .unwrap();
        let a = dir.path().join("a.md").display().to_string();
        let p = dir.path().join("p.gif").display().to_string();
        assert_eq!(
            files.text,
            format!("<file name=\"{a}\">\nhello\n</file>\n<file name=\"{p}\"></file>\n")
        );
        assert_eq!(files.images[0].mime_type, "image/gif");
        let missing = process_file_arguments(&["nope".into()], dir.path()).unwrap_err();
        assert!(missing.starts_with("Error: File not found: "));
    }

    #[test]
    fn initial_message_joins_stdin_files_and_first_message() {
        let mut messages = vec!["first".to_owned(), "second".to_owned()];
        let initial = build_initial_message(
            &mut messages,
            FileContent {
                text: "<f/>\n".into(),
                images: Vec::new(),
            },
            Some("piped".into()),
        );
        assert_eq!(initial.text.as_deref(), Some("piped<f/>\nfirst"));
        assert_eq!(messages, vec!["second"]);
        let mut none = Vec::new();
        assert_eq!(
            build_initial_message(&mut none, FileContent::default(), None).text,
            None
        );
    }
}
