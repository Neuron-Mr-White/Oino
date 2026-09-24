//! Guest-side SDK for the `extension` world of `oino:extension@0.1.0`.
//!
//! `wit_bindgen` generates the canonical-ABI bindings; [`Extension`] is the
//! ergonomic trait users implement, exported with [`export_extension!`].
//! Everything crosses the boundary as JSON (`serde_json::Value`) except the
//! typed helper records. Guest-side state lives in `static` storage: the
//! generated export traits are static methods, so instances cannot hold
//! `self` state.
//!
//! Provider components use the sibling `oino-extension-guest-provider` crate
//! (two `wit_bindgen::generate!` invocations in one crate collide on the
//! generated cabi macro names).

#![allow(unsafe_code)]

pub mod ext {
    //! Bindings for the `extension` world.
    wit_bindgen::generate!({
        world: "extension",
        path: "../../../wit",
        generate_all,
        export_macro_name: "export",
        pub_export_macro: true,
    });
}

use serde_json::Value;

/// Declarative registration returned from [`Extension::init`].
#[derive(Debug, Default, Clone)]
pub struct Registration {
    pub tools: Vec<ext::oino::extension::types::ToolDeclaration>,
    pub commands: Vec<(String, String)>,
    pub hooks: Vec<String>,
    pub providers: Vec<String>,
}

/// Result of [`Extension::run_tool`].
#[derive(Debug, Default, Clone)]
pub struct ToolOutput {
    pub content: Vec<ext::oino::extension::types::ContentBlock>,
    pub is_error: bool,
    pub details: Option<Value>,
    pub added_tool_names: Vec<String>,
    pub terminate: bool,
}

/// The ergonomic extension trait. Methods are static (the generated export
/// trait has no `self`); keep mutable state in `static` storage. All methods
/// have defaults so a crate only implements what it contributes.
pub trait Extension {
    fn init(_config: Value) -> Result<Registration, String> {
        Ok(Registration::default())
    }
    fn handle_event(_kind: &str, _payload: Value) -> Result<Value, String> {
        Ok(Value::Null)
    }
    fn run_tool(_name: &str, _call_id: &str, _args: Value) -> Result<ToolOutput, String> {
        Err("no tools".into())
    }
    fn run_command(name: &str, _args: &str) -> Result<Value, String> {
        Err(format!("unknown command `{name}`"))
    }
}

/// Host imports (`oino:extension/host`), each gated by a manifest permission.
pub mod host {
    use super::*;
    use ext::oino::extension::host as h;

    pub fn log(level: u8, msg: &str) {
        h::log(level, msg);
    }
    pub fn secret(name: &str) -> Result<String, String> {
        h::secret(name)
    }
    pub fn kv_get(key: &str) -> Option<Vec<u8>> {
        h::kv_get(key)
    }
    pub fn kv_set(key: &str, value: &[u8]) -> Result<(), String> {
        h::kv_set(key, value)
    }
    pub fn session_append(kind: &str, payload: &Value) -> Result<(), String> {
        h::session_append(kind, &serde_json::to_string(payload).unwrap_or_default())
    }
    pub fn notify(level: u8, text: &str) {
        h::notify(level, text);
    }
    pub fn confirm(title: &str, body: &str) -> bool {
        h::confirm(title, body)
    }
    pub fn exec(cmd: &str, args: &[String], cwd: Option<&str>) -> Result<h::ExecOutput, String> {
        h::exec(cmd, args, cwd)
    }
    pub fn tool_progress(call_id: &str, message: &str, details: Option<Value>) {
        let details = details.map(|v| serde_json::to_string(&v).unwrap_or_default());
        h::tool_progress(call_id, message, details.as_deref());
    }
}

/// Blocking HTTP over `wasi:http/outgoing-handler` with an incremental body
/// reader suitable for SSE streams.
pub mod http {
    use super::ext;
    use ext::wasi::http::types::{
        Fields, FutureIncomingResponse, IncomingBody, IncomingResponse, OutgoingRequest, Scheme,
    };
    use ext::wasi::io::streams::InputStream;

    /// An HTTP response with a streaming body; `read_chunk` yields `None` at
    /// end of stream.
    pub struct Response {
        pub status: u16,
        body: InputStream,
        _body: IncomingBody,
        _response: IncomingResponse,
    }

    impl Response {
        pub fn read_chunk(&mut self) -> Option<Vec<u8>> {
            match self.body.blocking_read(64 * 1024) {
                Ok(chunk) if chunk.is_empty() => None,
                Ok(chunk) => Some(chunk),
                Err(_) => None,
            }
        }
    }

    /// Debug probe: runs `get` one step at a time and reports which stage
    /// fails, for host-side trap diagnosis.
    pub fn probe(url: &str) -> String {
        let (scheme, authority, path) = match split_url(url) {
            Ok(v) => v,
            Err(e) => return format!("split: {e}"),
        };
        let request = OutgoingRequest::new(Fields::new());
        if request
            .set_method(&ext::wasi::http::types::Method::Get)
            .is_err()
        {
            return "set_method".into();
        }
        if request.set_scheme(Some(&scheme)).is_err() {
            return "set_scheme".into();
        }
        if request.set_authority(Some(&authority)).is_err() {
            return "set_authority".into();
        }
        if request.set_path_with_query(Some(&path)).is_err() {
            return "set_path".into();
        }
        let future = match ext::wasi::http::outgoing_handler::handle(request, None) {
            Ok(f) => f,
            Err(e) => return format!("handle: {e:?}"),
        };
        let incoming = match wait_incoming(future) {
            Ok(r) => r,
            Err(e) => return format!("incoming: {e}"),
        };
        let status = incoming.status();
        let body = match incoming.consume() {
            Ok(b) => b,
            Err(()) => return "consume".into(),
        };
        let stream = match body.stream() {
            Ok(s) => s,
            Err(()) => return "stream".into(),
        };
        let mut out = format!("status={status}");
        loop {
            match stream.blocking_read(1024) {
                Ok(chunk) if chunk.is_empty() => break,
                Ok(chunk) => out.push_str(&format!(" +{}", chunk.len())),
                Err(e) => {
                    out.push_str(&format!(" err:{e:?}"));
                    break;
                }
            }
        }
        out
    }

    /// GET `url`, blocking until response headers arrive.
    pub fn get(url: &str) -> Result<Response, String> {
        request("GET", url, &[], None)
    }

    /// Send a request with method, headers and optional body; blocks until
    /// response headers arrive. Streaming bodies are not supported — the body
    /// is buffered and sent with an explicit content length.
    pub fn request(
        method: &str,
        url: &str,
        headers: &[(String, String)],
        body: Option<&[u8]>,
    ) -> Result<Response, String> {
        let (scheme, authority, path) = split_url(url)?;
        let fields = Fields::new();
        for (name, value) in headers {
            fields
                .set(name, &[value.as_bytes().to_vec()])
                .map_err(|err| format!("set header `{name}`: {err:?}"))?;
        }
        if let Some(bytes) = body {
            // Explicit content-length so wasi-http does not use chunked
            // framing — some OpenAI-compatible servers reject chunked POSTs.
            fields
                .set("content-length", &[bytes.len().to_string().into_bytes()])
                .map_err(|err| format!("set content-length: {err:?}"))?;
        }
        let request = OutgoingRequest::new(fields);
        let method = match method.to_ascii_uppercase().as_str() {
            "GET" => ext::wasi::http::types::Method::Get,
            "POST" => ext::wasi::http::types::Method::Post,
            "PUT" => ext::wasi::http::types::Method::Put,
            "DELETE" => ext::wasi::http::types::Method::Delete,
            "HEAD" => ext::wasi::http::types::Method::Head,
            other => return Err(format!("unsupported method `{other}`")),
        };
        request
            .set_method(&method)
            .map_err(|_| "set method".to_string())?;
        request
            .set_scheme(Some(&scheme))
            .map_err(|_| "set scheme".to_string())?;
        request
            .set_authority(Some(&authority))
            .map_err(|_| "set authority".to_string())?;
        request
            .set_path_with_query(Some(&path))
            .map_err(|_| "set path".to_string())?;
        if let Some(bytes) = body {
            let outgoing = request.body().map_err(|_| "request body".to_string())?;
            let stream = outgoing
                .write()
                .map_err(|_| "request body stream".to_string())?;
            stream
                .blocking_write_and_flush(bytes)
                .map_err(|err| format!("write body: {err:?}"))?;
            drop(stream);
            ext::wasi::http::types::OutgoingBody::finish(outgoing, None)
                .map_err(|_| "finish body".to_string())?;
        }
        let future = ext::wasi::http::outgoing_handler::handle(request, None)
            .map_err(|err| format!("outgoing handle: {err:?}"))?;
        let incoming = wait_incoming(future)?;
        let status = incoming.status();
        let body = incoming.consume().map_err(|_| "consume body".to_string())?;
        let stream = body.stream().map_err(|_| "body stream".to_string())?;
        Ok(Response {
            status,
            body: stream,
            _body: body,
            _response: incoming,
        })
    }

    fn wait_incoming(future: FutureIncomingResponse) -> Result<IncomingResponse, String> {
        use ext::wasi::io::poll::poll;
        loop {
            match future.get() {
                Some(Ok(Ok(response))) => return Ok(response),
                Some(Ok(Err(err))) => return Err(format!("http error: {err:?}")),
                Some(Err(())) => return Err("future dropped".into()),
                None => {
                    let pollable = future.subscribe();
                    let _ = poll(&[&pollable]);
                }
            }
        }
    }

    fn split_url(url: &str) -> Result<(Scheme, String, String), String> {
        let (scheme, rest) = if let Some(rest) = url.strip_prefix("https://") {
            (Scheme::Https, rest)
        } else if let Some(rest) = url.strip_prefix("http://") {
            (Scheme::Http, rest)
        } else {
            return Err(format!("unsupported url `{url}`"));
        };
        let (authority, path) = match rest.find('/') {
            Some(index) => (&rest[..index], rest[index..].to_string()),
            None => (rest, "/".to_string()),
        };
        Ok((scheme, authority.to_string(), path))
    }
}

// ---- export glue (used by the macros) --------------------------------------

#[doc(hidden)]
pub fn json_in(raw: &str) -> Result<Value, String> {
    serde_json::from_str(raw).map_err(|err| format!("invalid json input: {err}"))
}

#[doc(hidden)]
pub fn json_out(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "null".into())
}

#[doc(hidden)]
pub mod dispatch {
    use super::*;

    pub fn init<T: Extension>(
        config: ext::oino::extension::types::Json,
    ) -> Result<ext::exports::oino::extension::guest::Registration, String> {
        let registration = T::init(json_in(&config)?)?;
        Ok(ext::exports::oino::extension::guest::Registration {
            tools: registration.tools,
            commands: registration
                .commands
                .into_iter()
                .map(
                    |(name, description)| ext::exports::oino::extension::guest::CommandDecl {
                        name,
                        description,
                    },
                )
                .collect(),
            hooks: registration.hooks,
            providers: registration.providers,
        })
    }

    pub fn handle_event<T: Extension>(
        kind: String,
        payload: ext::oino::extension::types::Json,
    ) -> Result<ext::oino::extension::types::Json, String> {
        Ok(json_out(&T::handle_event(&kind, json_in(&payload)?)?))
    }

    pub fn run_tool<T: Extension>(
        name: String,
        call_id: String,
        args: ext::oino::extension::types::Json,
    ) -> Result<ext::exports::oino::extension::guest::ToolOutput, String> {
        let output = T::run_tool(&name, &call_id, json_in(&args)?)?;
        Ok(ext::exports::oino::extension::guest::ToolOutput {
            content: output.content,
            is_error: output.is_error,
            details: output.details.map(|v| json_out(&v)),
            added_tool_names: output.added_tool_names,
            terminate: output.terminate,
        })
    }

    pub fn run_command<T: Extension>(
        name: String,
        args: String,
    ) -> Result<ext::oino::extension::types::Json, String> {
        Ok(json_out(&T::run_command(&name, &args)?))
    }
}

/// Export `T` as the `extension` world's guest interface.
#[macro_export]
macro_rules! export_extension {
    ($t:ty) => {
        struct OinoGuestExport;
        impl $crate::ext::exports::oino::extension::guest::Guest for OinoGuestExport {
            fn init(
                config: $crate::ext::oino::extension::types::Json,
            ) -> Result<$crate::ext::exports::oino::extension::guest::Registration, String> {
                $crate::dispatch::init::<$t>(config)
            }
            fn handle_event(
                kind: String,
                payload: $crate::ext::oino::extension::types::Json,
            ) -> Result<$crate::ext::oino::extension::types::Json, String> {
                $crate::dispatch::handle_event::<$t>(kind, payload)
            }
            fn run_tool(
                name: String,
                call_id: String,
                args: $crate::ext::oino::extension::types::Json,
            ) -> Result<$crate::ext::exports::oino::extension::guest::ToolOutput, String> {
                $crate::dispatch::run_tool::<$t>(name, call_id, args)
            }
            fn run_command(
                name: String,
                args: String,
            ) -> Result<$crate::ext::oino::extension::types::Json, String> {
                $crate::dispatch::run_command::<$t>(name, args)
            }
        }
        $crate::ext::export!(OinoGuestExport with_types_in $crate::ext);
    };
}
