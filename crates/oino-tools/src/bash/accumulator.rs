//! Bounded streaming output with an on-demand temp file (pi `tools/output-accumulator.ts`).

use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};

use crate::env::FsError;
use crate::node_path;
use crate::truncate::{
    DEFAULT_MAX_BYTES, DEFAULT_MAX_LINES, TruncatedBy, TruncationOptions, TruncationResult,
    truncate_tail,
};

/// Streaming UTF-8 decoder with WHATWG `TextDecoder` replacement semantics.
#[derive(Debug, Default)]
pub(crate) struct Utf8Decoder {
    pending: Vec<u8>,
}

impl Utf8Decoder {
    /// `decode(chunk, {stream: true})`: incomplete trailing sequences are held back.
    pub(crate) fn decode(&mut self, chunk: &[u8]) -> String {
        self.pending.extend_from_slice(chunk);
        let mut out = String::new();
        let mut rest: &[u8] = &self.pending;
        loop {
            match std::str::from_utf8(rest) {
                Ok(valid) => {
                    out.push_str(valid);
                    rest = &[];
                    break;
                }
                Err(error) => {
                    let (valid, after) = rest.split_at(error.valid_up_to());
                    out.push_str(&String::from_utf8_lossy(valid));
                    let Some(len) = error.error_len() else {
                        rest = after;
                        break;
                    };
                    out.push(char::REPLACEMENT_CHARACTER);
                    rest = &after[len..];
                }
            }
        }
        self.pending = rest.to_vec();
        out
    }

    /// `decode()`: flushes held-back bytes as replacement characters.
    pub(crate) fn finish(&mut self) -> String {
        let out = String::from_utf8_lossy(&self.pending).into_owned();
        self.pending.clear();
        out
    }
}

/// Creates `<dir>/<prefix>-<16 hex>.log` exclusively with mode 0600 (pi
/// `createOutputFileStream`).
pub(crate) fn create_output_file(dir: &str, prefix: &str) -> Result<(String, File), FsError> {
    std::fs::create_dir_all(dir).map_err(|error| FsError::from_io(&error, "mkdir", Some(dir)))?;
    let random = uuid::Uuid::new_v4().simple().to_string();
    let path = node_path::join(dir, &format!("{prefix}-{}.log", &random[..16]));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options
        .open(&path)
        .map_err(|error| FsError::from_io(&error, "open", Some(&path)))?;
    Ok((path, file))
}

/// pi `OutputSnapshot`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OutputSnapshot {
    pub(crate) content: String,
    pub(crate) truncation: TruncationResult,
    pub(crate) full_output_path: Option<String>,
}

/// pi `FullOutput`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FullOutput {
    pub(crate) content: String,
    pub(crate) truncated: bool,
}

/// pi `OutputAccumulator` with default limits.
#[derive(Debug)]
pub(crate) struct OutputAccumulator {
    max_lines: u64,
    max_bytes: u64,
    max_rolling_bytes: usize,
    temp_dir: String,
    temp_prefix: &'static str,
    decoder: Utf8Decoder,
    raw_chunks: Vec<Vec<u8>>,
    tail_text: String,
    tail_starts_at_line_boundary: bool,
    total_raw_bytes: u64,
    total_decoded_bytes: u64,
    completed_lines: u64,
    total_lines: u64,
    current_line_bytes: u64,
    has_open_line: bool,
    finished: bool,
    temp_path: Option<String>,
    temp_file: Option<BufWriter<File>>,
    temp_error: Option<FsError>,
}

fn count(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

impl OutputAccumulator {
    /// An accumulator whose temp file goes to `temp_dir/<temp_prefix>-*.log`.
    pub(crate) fn new(temp_dir: String, temp_prefix: &'static str) -> Self {
        let max_bytes = DEFAULT_MAX_BYTES;
        Self {
            max_lines: DEFAULT_MAX_LINES,
            max_bytes,
            max_rolling_bytes: usize::try_from(max_bytes * 2).unwrap_or(usize::MAX).max(1),
            temp_dir,
            temp_prefix,
            decoder: Utf8Decoder::default(),
            raw_chunks: Vec::new(),
            tail_text: String::new(),
            tail_starts_at_line_boundary: true,
            total_raw_bytes: 0,
            total_decoded_bytes: 0,
            completed_lines: 0,
            total_lines: 0,
            current_line_bytes: 0,
            has_open_line: false,
            finished: false,
            temp_path: None,
            temp_file: None,
            temp_error: None,
        }
    }

    /// pi `append`.
    pub(crate) fn append(&mut self, data: &[u8]) {
        if self.finished {
            return;
        }
        self.total_raw_bytes += count(data.len());
        let text = self.decoder.decode(data);
        self.append_decoded(&text);
        if self.temp_file.is_some() || self.should_use_temp_file() {
            self.ensure_temp_file();
            self.write_temp(data);
        } else if !data.is_empty() {
            self.raw_chunks.push(data.to_vec());
        }
    }

    /// pi `finish`.
    pub(crate) fn finish(&mut self) {
        if self.finished {
            return;
        }
        self.finished = true;
        let rest = self.decoder.finish();
        self.append_decoded(&rest);
        if self.should_use_temp_file() {
            self.ensure_temp_file();
        }
    }

    /// pi `snapshot({persistIfTruncated})`.
    pub(crate) fn snapshot(&mut self, persist_if_truncated: bool) -> OutputSnapshot {
        let tail = truncate_tail(
            self.snapshot_text(),
            TruncationOptions {
                max_lines: self.max_lines,
                max_bytes: self.max_bytes,
            },
        );
        let truncated =
            self.total_lines > self.max_lines || self.total_decoded_bytes > self.max_bytes;
        let truncated_by = truncated.then(|| {
            tail.truncated_by
                .unwrap_or(if self.total_decoded_bytes > self.max_bytes {
                    TruncatedBy::Bytes
                } else {
                    TruncatedBy::Lines
                })
        });
        let truncation = TruncationResult {
            truncated,
            truncated_by,
            total_lines: self.total_lines,
            total_bytes: self.total_decoded_bytes,
            max_lines: self.max_lines,
            max_bytes: self.max_bytes,
            ..tail
        };
        if persist_if_truncated && truncation.truncated {
            self.ensure_temp_file();
        }
        OutputSnapshot {
            content: truncation.content.clone(),
            truncation,
            full_output_path: self.temp_path.clone(),
        }
    }

    /// pi `closeTempFile`.
    pub(crate) fn close_temp_file(&mut self) -> Result<(), FsError> {
        if let Some(mut file) = self.temp_file.take()
            && let Err(error) = file.flush()
        {
            self.temp_error
                .get_or_insert_with(|| FsError::from_io(&error, "write", None));
        }
        match self.temp_error.take() {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    /// pi `readFullOutput(maxBytes)`: the complete output, or its first and last
    /// `max_bytes / 2` bytes around `[... N bytes omitted ...]`.
    pub(crate) fn read_full_output(&self, max_bytes: u64) -> Result<FullOutput, FsError> {
        let Some(path) = &self.temp_path else {
            return Ok(FullOutput {
                content: String::from_utf8_lossy(&self.raw_chunks.concat()).into_owned(),
                truncated: false,
            });
        };
        let to_error = |error: std::io::Error| FsError::from_io(&error, "read", None);
        let mut file =
            File::open(path).map_err(|error| FsError::from_io(&error, "open", Some(path)))?;
        let size = file.metadata().map_err(to_error)?.len();
        if size <= max_bytes {
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes).map_err(to_error)?;
            return Ok(FullOutput {
                content: String::from_utf8_lossy(&bytes).into_owned(),
                truncated: false,
            });
        }
        let head_len = max_bytes / 2;
        let tail_len = max_bytes - head_len;
        let mut head = vec![0_u8; usize::try_from(head_len).unwrap_or(0)];
        file.read_exact(&mut head).map_err(to_error)?;
        let mut tail = vec![0_u8; usize::try_from(tail_len).unwrap_or(0)];
        file.seek(SeekFrom::Start(size - tail_len))
            .map_err(to_error)?;
        file.read_exact(&mut tail).map_err(to_error)?;
        let head_text = Utf8Decoder::default().decode(&head);
        let tail_start = tail
            .iter()
            .take_while(|byte| (**byte & 0xc0) == 0x80)
            .count();
        let tail_text = String::from_utf8_lossy(&tail[tail_start..]);
        let omitted = size - head_len - tail_len;
        Ok(FullOutput {
            content: format!("{head_text}\n\n[... {omitted} bytes omitted ...]\n\n{tail_text}"),
            truncated: true,
        })
    }

    /// pi `getLastLineBytes`.
    pub(crate) fn last_line_bytes(&self) -> u64 {
        self.current_line_bytes
    }

    fn append_decoded(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        let bytes = count(text.len());
        self.total_decoded_bytes += bytes;
        self.tail_text.push_str(text);
        if self.tail_text.len() > self.max_rolling_bytes * 2 {
            self.trim_tail();
        }
        let newlines = count(text.matches('\n').count());
        if newlines == 0 {
            self.current_line_bytes += bytes;
            self.has_open_line = true;
        } else {
            self.completed_lines += newlines;
            let tail = text.rsplit('\n').next().unwrap_or("");
            self.current_line_bytes = count(tail.len());
            self.has_open_line = !tail.is_empty();
        }
        self.total_lines = self.completed_lines + u64::from(self.has_open_line);
    }

    fn trim_tail(&mut self) {
        if self.tail_text.len() <= self.max_rolling_bytes {
            return;
        }
        let mut start = self.tail_text.len() - self.max_rolling_bytes;
        while !self.tail_text.is_char_boundary(start) {
            start += 1;
        }
        if start != 0 {
            self.tail_starts_at_line_boundary = self.tail_text.as_bytes()[start - 1] == b'\n';
        }
        self.tail_text = self.tail_text[start..].to_owned();
    }

    fn snapshot_text(&self) -> &str {
        if self.tail_starts_at_line_boundary {
            return &self.tail_text;
        }
        match self.tail_text.find('\n') {
            Some(index) => &self.tail_text[index + 1..],
            None => &self.tail_text,
        }
    }

    fn should_use_temp_file(&self) -> bool {
        self.total_raw_bytes > self.max_bytes
            || self.total_decoded_bytes > self.max_bytes
            || self.total_lines > self.max_lines
    }

    fn ensure_temp_file(&mut self) {
        if self.temp_path.is_some() {
            return;
        }
        match create_output_file(&self.temp_dir, self.temp_prefix) {
            Ok((path, file)) => {
                self.temp_path = Some(path);
                self.temp_file = Some(BufWriter::new(file));
                for chunk in std::mem::take(&mut self.raw_chunks) {
                    self.write_temp(&chunk);
                }
            }
            Err(error) => {
                // pi reports stream errors when the file is closed.
                self.temp_path = Some(String::new());
                self.temp_error.get_or_insert(error);
            }
        }
    }

    fn write_temp(&mut self, data: &[u8]) {
        if let Some(file) = &mut self.temp_file
            && let Err(error) = file.write_all(data)
        {
            self.temp_error
                .get_or_insert_with(|| FsError::from_io(&error, "write", None));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decoder_holds_incomplete_sequences() {
        let mut decoder = Utf8Decoder::default();
        let bytes = "é".as_bytes();
        assert_eq!(decoder.decode(&bytes[..1]), "");
        assert_eq!(decoder.decode(&bytes[1..]), "é");
        assert_eq!(decoder.decode(&[0xff, b'a']), "\u{fffd}a");
        assert_eq!(decoder.decode(&[0xe2, 0x82]), "");
        assert_eq!(decoder.finish(), "\u{fffd}");
    }

    #[test]
    fn counts_lines_and_bytes() {
        let mut output = OutputAccumulator::new(String::from("/nonexistent"), "t");
        output.append(b"a\nbb");
        output.append(b"b\n");
        output.finish();
        let snapshot = output.snapshot(false);
        assert_eq!(snapshot.content, "a\nbbb\n");
        assert_eq!(snapshot.truncation.total_lines, 2);
        assert!(!snapshot.truncation.truncated);
        assert_eq!(output.last_line_bytes(), 0);
    }
}
