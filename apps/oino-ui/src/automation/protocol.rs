//! The JSON-lines protocol between a driver and the app (docs/dev/v1/gui-driver.md §2).
//!
//! One JSON object per line in each direction. A request is
//! `{"id": <any>, "method": "<name>", "params": {…}}`; the reply is
//! `{"id": <same>, "ok": true, "result": {…}}` or
//! `{"id": <same>, "ok": false, "error": {"message": "…"}}`. Requests are handled one at a
//! time, in order, so a scenario is deterministic.

use serde_json::{Value, json};

use super::registry::Role;

/// Methods the server answers (kept in sync with [`Request::method`] and the docs).
pub const METHODS: [&str; 12] = [
    "tree",
    "state",
    "click",
    "type_text",
    "key",
    "scroll",
    "focus",
    "wait_for",
    "screenshot",
    "settle",
    "ping",
    "quit",
];

/// A parsed request.
#[derive(Clone)]
pub struct Request {
    /// Caller's correlation id (echoed back verbatim).
    pub id: Value,
    /// Method name.
    pub method: String,
    /// Parameters.
    pub params: Value,
}

/// Parsing a request line failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError(pub String);

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl Request {
    /// Parses one line.
    ///
    /// # Errors
    ///
    /// `Err` names the problem (bad JSON, missing `method`, unknown method) so the server can
    /// answer on the same connection.
    pub fn parse(line: &str) -> Result<Self, ParseError> {
        let value: Value =
            serde_json::from_str(line).map_err(|e| ParseError(format!("invalid JSON: {e}")))?;
        let Some(object) = value.as_object() else {
            return Err(ParseError("a request must be a JSON object".into()));
        };
        let method = object
            .get("method")
            .and_then(Value::as_str)
            .ok_or_else(|| ParseError("missing \"method\"".into()))?;
        if !METHODS.contains(&method) {
            return Err(ParseError(format!(
                "unknown method {method:?} (known: {})",
                METHODS.join(", ")
            )));
        }
        Ok(Self {
            id: object.get("id").cloned().unwrap_or(Value::Null),
            method: method.to_owned(),
            params: object.get("params").cloned().unwrap_or(Value::Null),
        })
    }

    /// A string parameter.
    #[must_use]
    pub fn str(&self, key: &str) -> Option<&str> {
        self.params.get(key).and_then(Value::as_str)
    }

    /// A number parameter.
    #[must_use]
    pub fn num(&self, key: &str) -> Option<f64> {
        self.params.get(key).and_then(Value::as_f64)
    }

    /// A boolean parameter (`false` when absent).
    #[must_use]
    pub fn bool(&self, key: &str) -> bool {
        self.params
            .get(key)
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }

    /// The selector of this request (`params.id` / `params.role` / `params.label`).
    #[must_use]
    pub fn selector(&self) -> Selector {
        Selector {
            id: self.str("id").map(str::to_owned),
            role: self.str("role").and_then(Role::parse),
            label: self.str("label").map(str::to_owned),
            include_hidden: self.bool("include_hidden"),
        }
    }

    /// Whether the request names an element (any selector field).
    #[must_use]
    pub fn has_selector(&self) -> bool {
        self.str("id").is_some() || self.str("role").is_some() || self.str("label").is_some()
    }
}

/// Which element a request means.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selector {
    /// Exact element id (`send`, `session-row#3`).
    pub id: Option<String>,
    /// Semantic role.
    pub role: Option<Role>,
    /// Label / value substring, case-insensitive.
    pub label: Option<String>,
    /// Also match elements outside the viewport (`visible: false`).
    pub include_hidden: bool,
}

impl Selector {
    /// Whether this selector names an element by id alone (the exact-id fast path).
    #[must_use]
    pub fn by_id(&self) -> bool {
        self.id.is_some() && self.role.is_none() && self.label.is_none()
    }

    /// Whether `node` matches every given field.
    #[must_use]
    pub fn matches(&self, node: &super::registry::Node) -> bool {
        if let Some(id) = &self.id
            && &node.id != id
        {
            return false;
        }
        if let Some(role) = self.role
            && node.role != role
        {
            return false;
        }
        if let Some(label) = &self.label
            && !node.contains(label)
        {
            return false;
        }
        self.id.is_some() || self.role.is_some() || self.label.is_some()
    }
}

impl std::fmt::Debug for Request {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {}", self.method, self.params)
    }
}

/// A successful reply payload; the server adds `id` and `ok`.
#[must_use]
pub fn ok(id: &Value, result: &Value) -> Value {
    json!({"id": id, "ok": true, "result": result})
}

/// A failed reply payload.
#[must_use]
pub fn error(id: &Value, message: impl Into<String>) -> Value {
    json!({"id": id, "ok": false, "error": {"message": message.into()}})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_request_line() {
        let req = Request::parse(r#"{"id":7,"method":"click","params":{"id":"send"}}"#)
            .expect("valid request");
        assert_eq!(req.id, json!(7));
        assert_eq!(req.method, "click");
        assert_eq!(req.selector().id.as_deref(), Some("send"));
        assert!(req.has_selector());
    }

    #[test]
    fn rejects_unknown_methods_and_junk() {
        let err = Request::parse(r#"{"method":"explode"}"#).expect_err("unknown method");
        assert!(err.0.contains("unknown method"), "{err}");
        assert!(Request::parse("nope").is_err());
        assert!(Request::parse("[]").is_err());
        assert!(Request::parse(r#"{"id":1}"#).is_err());
    }

    #[test]
    fn selector_matches_role_and_label() {
        let node = super::super::registry::Node {
            id: "set-mascot".into(),
            path: "root.settings.set-mascot".into(),
            role: Role::Switch,
            label: "Mascot window".into(),
            value: None,
            enabled: true,
            focused: false,
            checked: Some(true),
            selected: None,
            visible: true,
            bounds: (10.0, 20.0, 44.0, 26.0),
        };
        let sel = Selector {
            label: Some("mascot".into()),
            role: Some(Role::Switch),
            ..Selector::default()
        };
        assert!(sel.matches(&node));
        assert!(
            !Selector {
                role: Some(Role::Button),
                ..sel.clone()
            }
            .matches(&node)
        );
        assert!(
            !Selector {
                id: Some("other".into()),
                ..sel.clone()
            }
            .matches(&node)
        );
        assert!(!Selector::default().matches(&node));
    }

    #[test]
    fn method_table_is_complete() {
        for method in METHODS {
            let line = format!(r#"{{"id":1,"method":"{method}"}}"#);
            assert!(Request::parse(&line).is_ok(), "{method}");
        }
    }
}
