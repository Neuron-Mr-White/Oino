//! JSON helpers ported from pi-ai `utils/json-parse.js` and the `partial-json` package:
//! repairing malformed string literals, streaming (partial) parsing and tool-argument parsing.

use std::fmt::Write as _;

use oino_ai::JsonObject;
use serde_json::Value;

const VALID_JSON_ESCAPES: [char; 8] = ['"', '\\', '/', 'b', 'f', 'n', 'r', 't'];

/// pi `repairJson`: escapes raw control characters inside strings and doubles backslashes
/// before invalid escape characters.
#[must_use]
pub fn repair_json(json: &str) -> String {
    let chars: Vec<char> = json.chars().collect();
    let mut repaired = String::with_capacity(json.len());
    let mut in_string = false;
    let mut index = 0;
    while index < chars.len() {
        let ch = chars[index];
        index += 1;
        if !in_string {
            repaired.push(ch);
            in_string = ch == '"';
            continue;
        }
        if ch == '"' {
            repaired.push(ch);
            in_string = false;
            continue;
        }
        if ch == '\\' {
            match chars.get(index) {
                Some('u')
                    if chars.len() >= index + 5
                        && chars[index + 1..index + 5]
                            .iter()
                            .all(char::is_ascii_hexdigit) =>
                {
                    repaired.push('\\');
                    repaired.extend(&chars[index..index + 5]);
                    index += 5;
                }
                Some(next) if VALID_JSON_ESCAPES.contains(next) => {
                    repaired.push('\\');
                    repaired.push(*next);
                    index += 1;
                }
                _ => repaired.push_str("\\\\"),
            }
            continue;
        }
        match ch {
            '\u{8}' => repaired.push_str("\\b"),
            '\u{c}' => repaired.push_str("\\f"),
            '\n' => repaired.push_str("\\n"),
            '\r' => repaired.push_str("\\r"),
            '\t' => repaired.push_str("\\t"),
            c if u32::from(c) <= 0x1f => {
                let _ = write!(repaired, "\\u{:04x}", u32::from(c));
            }
            c => repaired.push(c),
        }
    }
    repaired
}

/// pi `parseJsonWithRepair`: strict parse, then a parse of the repaired text.
///
/// # Errors
/// The strict parse error when neither form parses.
pub fn parse_json_with_repair(json: &str) -> Result<Value, serde_json::Error> {
    serde_json::from_str(json).or_else(|error| {
        let repaired = repair_json(json);
        if repaired == json {
            Err(error)
        } else {
            serde_json::from_str(&repaired)
        }
    })
}

/// pi `parseStreamingJson`: the value of possibly incomplete JSON (`{}` when nothing parses).
#[must_use]
pub fn parse_streaming_json(partial: &str) -> Value {
    if partial.trim().is_empty() {
        return Value::Object(JsonObject::new());
    }
    parse_json_with_repair(partial)
        .ok()
        .or_else(|| partial_parse(partial))
        .or_else(|| partial_parse(&repair_json(partial)))
        .unwrap_or_else(|| Value::Object(JsonObject::new()))
}

/// [`parse_streaming_json`] narrowed to an object (tool-call arguments in partials).
#[must_use]
pub fn parse_streaming_object(partial: &str) -> JsonObject {
    match parse_streaming_json(partial) {
        Value::Object(map) => map,
        _ => JsonObject::new(),
    }
}

/// pi `parseToolCallArguments` result.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ToolArguments {
    /// Parsed object (`{}` on failure).
    pub arguments: JsonObject,
    /// Error text including the received arguments, when parsing failed.
    pub error: Option<String>,
}

/// pi `parseToolCallArguments`: parses the complete arguments text of a finished call.
#[must_use]
pub fn parse_tool_call_arguments(json: &str) -> ToolArguments {
    if json.trim().is_empty() {
        return ToolArguments::default();
    }
    let failure = |message: String| ToolArguments {
        arguments: JsonObject::new(),
        error: Some(format!("{message}\n\nReceived arguments:\n{json}")),
    };
    match parse_json_with_repair(json) {
        Ok(Value::Object(arguments)) => ToolArguments {
            arguments,
            error: None,
        },
        Ok(_) => failure("Tool arguments must be a JSON object".to_owned()),
        Err(error) => failure(error.to_string()),
    }
}

/// `partial-json` `parse(text, Allow.ALL)`; `None` when the text is malformed.
fn partial_parse(text: &str) -> Option<Value> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let chars: Vec<char> = trimmed.chars().collect();
    PartialParser {
        chars: &chars,
        index: 0,
    }
    .any()
    .ok()
}

/// Failure while partially parsing (`partial-json` throws `PartialJSON`/`MalformedJSON`).
#[derive(Debug)]
struct Stop;

struct PartialParser<'a> {
    chars: &'a [char],
    index: usize,
}

impl PartialParser<'_> {
    fn len(&self) -> usize {
        self.chars.len()
    }

    fn at(&self, index: usize) -> Option<char> {
        self.chars.get(index).copied()
    }

    fn text(&self, start: usize, end: usize) -> String {
        self.chars[start.min(self.len())..end.min(self.len())]
            .iter()
            .collect()
    }

    fn skip_blank(&mut self) {
        while matches!(self.at(self.index), Some(' ' | '\n' | '\r' | '\t')) {
            self.index += 1;
        }
    }

    /// Literal at the cursor, complete or as a truncated prefix at the end of input.
    fn literal(&mut self, word: &str) -> bool {
        let rest = self.text(self.index, self.len());
        let n = word.chars().count();
        if rest.starts_with(word) || (rest.chars().count() < n && word.starts_with(&rest)) {
            self.index += n;
            true
        } else {
            false
        }
    }

    fn any(&mut self) -> Result<Value, Stop> {
        self.skip_blank();
        match self.at(self.index) {
            None => Err(Stop),
            Some('"') => self.string().map(Value::String),
            Some('{') => Ok(self.object()),
            Some('[') => Ok(self.array()),
            Some(_) => {
                if self.literal("null") {
                    return Ok(Value::Null);
                }
                if self.literal("true") {
                    return Ok(Value::Bool(true));
                }
                if self.literal("false") {
                    return Ok(Value::Bool(false));
                }
                // JSON.stringify turns ±Infinity/NaN into null.
                if self.literal("Infinity") || self.literal("-Infinity") || self.literal("NaN") {
                    return Ok(Value::Null);
                }
                self.number()
            }
        }
    }

    fn string(&mut self) -> Result<String, Stop> {
        let start = self.index;
        let mut escape = false;
        self.index += 1;
        while let Some(ch) = self.at(self.index) {
            if ch == '"' && !(escape && self.at(self.index - 1) == Some('\\')) {
                break;
            }
            escape = ch == '\\' && !escape;
            self.index += 1;
        }
        let parse = |text: String| serde_json::from_str::<String>(&text).map_err(|_| Stop);
        if self.at(self.index) == Some('"') {
            self.index += 1;
            return parse(self.text(start, self.index - usize::from(escape)));
        }
        parse(format!(
            "{}\"",
            self.text(start, self.index - usize::from(escape))
        ))
        .or_else(|_| {
            let last_backslash = self.chars.iter().rposition(|c| *c == '\\').unwrap_or(start);
            parse(format!("{}\"", self.text(start, last_backslash)))
        })
    }

    fn object(&mut self) -> Value {
        self.index += 1;
        self.skip_blank();
        let mut object = JsonObject::new();
        while self.at(self.index) != Some('}') {
            self.skip_blank();
            if self.index >= self.len() || self.at(self.index) != Some('"') {
                return Value::Object(object);
            }
            let Ok(key) = self.string() else {
                return Value::Object(object);
            };
            self.skip_blank();
            self.index += 1;
            let Ok(value) = self.any() else {
                return Value::Object(object);
            };
            object.insert(key, value);
            self.skip_blank();
            if self.at(self.index) == Some(',') {
                self.index += 1;
            }
        }
        self.index += 1;
        Value::Object(object)
    }

    fn array(&mut self) -> Value {
        self.index += 1;
        let mut array = Vec::new();
        while self.at(self.index) != Some(']') {
            let before = self.index;
            let Ok(value) = self.any() else {
                return Value::Array(array);
            };
            array.push(value);
            self.skip_blank();
            if self.at(self.index) == Some(',') {
                self.index += 1;
            }
            if self.index == before {
                return Value::Array(array);
            }
        }
        self.index += 1;
        Value::Array(array)
    }

    fn number(&mut self) -> Result<Value, Stop> {
        let parse = |text: &str| serde_json::from_str::<Value>(text).map_err(|_| Stop);
        let whole = self.text(0, self.len());
        if self.index == 0 {
            if whole == "-" {
                return Err(Stop);
            }
            return parse(&whole).or_else(|_| match whole.rfind('e') {
                Some(e) => parse(&whole[..e]),
                None => Err(Stop),
            });
        }
        let start = self.index;
        if self.at(self.index) == Some('-') {
            self.index += 1;
        }
        while let Some(ch) = self.at(self.index) {
            if matches!(ch, ',' | ']' | '}') {
                break;
            }
            self.index += 1;
        }
        let text = self.text(start, self.index);
        parse(&text).or_else(|_| match text.rfind('e') {
            Some(e) if e > 0 => parse(&text[..e]),
            _ => Err(Stop),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn streaming_keeps_complete_prefix() {
        assert_eq!(parse_streaming_json("{\"q\":"), json!({}));
        assert_eq!(parse_streaming_json("{\"q\":\"x"), json!({"q": "x"}));
        assert_eq!(
            parse_streaming_json("{\"a\":1,\"b\":[tr"),
            json!({"a": 1, "b": [true]})
        );
        assert_eq!(parse_streaming_json("{\"n\":12"), json!({"n": 12}));
        assert_eq!(parse_streaming_json("  "), json!({}));
    }

    #[test]
    fn repair_escapes_control_characters_and_bad_escapes() {
        assert_eq!(repair_json("{\"a\":\"x\ny\"}"), "{\"a\":\"x\\ny\"}");
        assert_eq!(repair_json("{\"a\":\"c:\\d\"}"), "{\"a\":\"c:\\\\d\"}");
        assert_eq!(repair_json("{\"a\":\"\\u00e9\"}"), "{\"a\":\"\\u00e9\"}");
    }

    #[test]
    fn tool_arguments_report_errors() {
        assert_eq!(parse_tool_call_arguments("").arguments, JsonObject::new());
        let parsed = parse_tool_call_arguments("[1]");
        assert_eq!(
            parsed.error.as_deref(),
            Some("Tool arguments must be a JSON object\n\nReceived arguments:\n[1]")
        );
        assert!(parse_tool_call_arguments("{\"a\":").error.is_some());
    }
}
