//! Login-flow plumbing for the provider components' `oauth-login` exports (pi-ai
//! `auth/oauth/callback-server.js`, `device-code.js`): HTTP helpers, a loopback callback raced
//! against a pasted redirect URL, and RFC 8628 device-code polling over `timers.sleep`.
//!
//! pi's `interaction.notify({type: "auth_url" | "device_code"})` becomes `ui.open-url` plus a
//! `ui.notify` message; `interaction.prompt` becomes `ui.input` / `ui.select`.

use std::future::Future;
use std::time::Duration;

use oino_extension::futures::future::{self, Either};
use oino_extension::http::{self, Incoming, Listener, Method};
use oino_extension::{Error, Result, timers, ui};
use serde_json::Value;

use crate::oauth::{DevicePoll, PollStatus, form, query_param};

/// `n` random bytes from the host (`wasi:random`).
#[must_use]
pub fn random_bytes(n: u64) -> Vec<u8> {
    oino_extension::bindings::wasi::random::random::get_random_bytes(n)
}

/// A finished HTTP exchange.
#[derive(Clone, Debug)]
pub struct Reply {
    /// Status code.
    pub status: u16,
    /// Response headers.
    pub headers: Vec<(String, String)>,
    /// Body text.
    pub text: String,
}

impl Reply {
    /// 2xx.
    #[must_use]
    pub const fn ok(&self) -> bool {
        self.status >= 200 && self.status < 300
    }

    /// Body as JSON (`Null` when invalid).
    #[must_use]
    pub fn json(&self) -> Value {
        serde_json::from_str(&self.text).unwrap_or(Value::Null)
    }

    /// Header value.
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// Sends `method url` with `headers` and an optional body (30 s timeout).
///
/// # Errors
/// Transport failures.
pub async fn send(
    method: Method,
    url: &str,
    headers: &[(&str, &str)],
    body: Option<Vec<u8>>,
) -> Result<Reply> {
    let mut builder = http::request(method, url).timeout_ms(30_000);
    for (name, value) in headers {
        builder = builder.header(name, value);
    }
    if let Some(body) = body {
        builder = builder.body(body);
    }
    let response = builder.send().await?;
    let status = response.status();
    let headers = response.head.headers.clone();
    let text = response.text().await;
    Ok(Reply {
        status,
        headers,
        text,
    })
}

/// `POST url` with an `application/x-www-form-urlencoded` body.
///
/// # Errors
/// Transport failures.
pub async fn post_form(
    url: &str,
    headers: &[(&str, &str)],
    pairs: &[(&str, &str)],
) -> Result<Reply> {
    let mut all = vec![("content-type", "application/x-www-form-urlencoded")];
    all.extend_from_slice(headers);
    send(Method::Post, url, &all, Some(form(pairs).into_bytes())).await
}

/// `POST url` with a JSON body.
///
/// # Errors
/// Transport failures.
pub async fn post_json(url: &str, headers: &[(&str, &str)], body: &Value) -> Result<Reply> {
    let mut all = vec![("content-type", "application/json")];
    all.extend_from_slice(headers);
    send(Method::Post, url, &all, Some(body.to_string().into_bytes())).await
}

/// Opens `url` in the browser and tells the user (pi `auth_url` notification).
pub fn show_auth_url(url: &str, instructions: &str) {
    let _ = ui::open_url(url);
    let _ = ui::notify(&format!("{instructions}\n{url}"), "info");
}

/// Shows a device code (pi `device_code` notification) and opens the verification page.
pub fn show_device_code(user_code: &str, verification_uri: &str) {
    let _ = ui::open_url(verification_uri);
    let _ = ui::notify(
        &format!("Open {verification_uri} and enter the code: {user_code}"),
        "info",
    );
}

/// What ended a browser login wait.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Callback {
    /// The loopback server received the redirect: its query string.
    Query(String),
    /// The user pasted something.
    Manual(String),
}

/// HTML page for the browser after the callback.
#[must_use]
pub fn page(text: &str) -> Vec<u8> {
    format!("<!doctype html><html><body><p>{text}</p></body></html>").into_bytes()
}

/// A callback check: `Ok(())` accepts the request, `Err(message)` answers 400 and keeps
/// waiting (pi: an invalid callback does not end the login).
pub type CallbackCheck<'a> = &'a dyn Fn(&str) -> std::result::Result<(), String>;

async fn accept_callback(
    listener: &Listener,
    path: &str,
    check: CallbackCheck<'_>,
    provider: &str,
) -> Result<String> {
    let html = vec![(
        "content-type".to_owned(),
        "text/html; charset=utf-8".to_owned(),
    )];
    loop {
        let Incoming {
            id, path: target, ..
        } = listener
            .accept()
            .await
            .ok_or_else(|| Error::msg("OAuth callback server closed"))?;
        let (route, query) = target.split_once('?').unwrap_or((&target, ""));
        if route != path {
            let _ = listener.respond(id, 404, &html, &page("Callback route not found."));
            continue;
        }
        if let Some(error) = query_param(query, "error") {
            let description = query_param(query, "error_description").unwrap_or(error);
            let _ = listener.respond(
                id,
                400,
                &html,
                &page(&format!("{provider} authorization failed. {description}")),
            );
            return Err(Error::msg(format!(
                "{provider} authorization failed: {description}"
            )));
        }
        if let Err(message) = check(query) {
            let _ = listener.respond(id, 400, &html, &page(&message));
            continue;
        }
        let _ = listener.respond(
            id,
            200,
            &html,
            &page(&format!(
                "Signed in to {provider}. You may now close this page."
            )),
        );
        return Ok(query.to_owned());
    }
}

/// pi `waitForCallbackOrManualInput`: races the loopback callback (when bound) against a
/// pasted code / redirect URL.
///
/// # Errors
/// UI or listener failures.
pub async fn callback_or_manual(
    listener: Option<&Listener>,
    path: &str,
    check: CallbackCheck<'_>,
    provider: &str,
    prompt: &str,
    placeholder: &str,
) -> Result<Callback> {
    let manual = ui::input(prompt, Some(placeholder));
    let Some(listener) = listener else {
        return Ok(Callback::Manual(manual.await?.unwrap_or_default()));
    };
    let callback = accept_callback(listener, path, check, provider);
    match future::select(Box::pin(callback), Box::pin(manual)).await {
        Either::Left((query, _)) => Ok(Callback::Query(query?)),
        Either::Right((input, _)) => Ok(Callback::Manual(input?.unwrap_or_default())),
    }
}

/// A callback check that requires `state` to equal `expected` and a `code`.
pub fn state_and_code(expected: &str) -> impl Fn(&str) -> std::result::Result<(), String> + '_ {
    move |query: &str| {
        if query_param(query, "state").as_deref() != Some(expected) {
            return Err("State mismatch".to_owned());
        }
        if query_param(query, "code").is_none() {
            return Err("Missing authorization code".to_owned());
        }
        Ok(())
    }
}

/// pi `pollOAuthDeviceCodeFlow`.
///
/// # Errors
/// The poll's failure message, or a timeout.
pub async fn poll_device<T, F, Fut>(
    mut schedule: DevicePoll,
    wait_before_first: bool,
    mut poll: F,
) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<PollStatus<T>>>,
{
    if wait_before_first {
        let sleep = schedule.next_sleep(timers::now_ms());
        if sleep > 0 {
            timers::sleep(Duration::from_millis(sleep)).await;
        }
    }
    while schedule.active(timers::now_ms()) {
        match poll().await? {
            PollStatus::Complete(value) => return Ok(value),
            PollStatus::Failed(message) => return Err(Error::msg(message)),
            PollStatus::SlowDown(interval) => schedule.slow_down(interval),
            PollStatus::Pending => {}
        }
        let now = timers::now_ms();
        if !schedule.active(now) {
            break;
        }
        timers::sleep(Duration::from_millis(schedule.next_sleep(now))).await;
    }
    Err(Error::msg(schedule.timeout_message()))
}
