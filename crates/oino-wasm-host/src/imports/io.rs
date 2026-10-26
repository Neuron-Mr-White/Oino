//! `process`, `http`, `fs`, `timers`, `kv`: implemented by the host itself.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use oino_extension_api::ops::ModelsOp;
use oino_extension_api::{CallContext, EventKind, HostError, RawJson};
use serde_json::json;
use wasmtime::component::{Access, Accessor, Resource, StreamReader};

use super::{ready, ready_wit};
use crate::bindings::oino::extension::{
    fs as wf, http as wh, kv as wk, process as wp, timers as wtm, types as wt,
};
use crate::convert;
use crate::paths;
use crate::process::{self, Exit, Input, Output, Sig, Spawn};
use crate::resources::{ChildRes, ListenerRes, LockRes, WatcherRes};
use crate::state::{ExtShared, HostState, current, current_async};
use crate::streams::ByteProducer;

type HostResult<T> = wasmtime::Result<Result<T, wt::HostError>>;

// ── process ──────────────────────────────────────────────────────────────

fn host_path(cx: &CallContext, p: Option<String>) -> PathBuf {
    match p {
        Some(p) if Path::new(&p).is_absolute() => PathBuf::from(p),
        Some(p) => cx.cwd.join(p),
        None => cx.cwd.clone(),
    }
}

fn exit_status(e: Exit) -> wp::ExitStatus {
    wp::ExitStatus {
        code: e.code,
        signal: e.signal,
        timed_out: e.timed_out,
    }
}

fn output_target(
    ext: &ExtShared,
    cx: &CallContext,
    t: wp::OutputTarget,
) -> Result<Output, HostError> {
    Ok(match t {
        wp::OutputTarget::Null => Output::Null,
        wp::OutputTarget::Pipe => Output::Pipe,
        wp::OutputTarget::File(path) => {
            Output::File(paths::resolve(&ext.preopens(cx), &path, true)?)
        }
    })
}

fn spawn(
    ext: &Arc<ExtShared>,
    cx: &CallContext,
    spec: wp::SpawnSpec,
) -> Result<Arc<process::ChildHandle>, HostError> {
    let perms = ext.perms();
    ExtShared::require(
        perms.allows_program(&spec.command, ext.bundled),
        &format!("process.allow:{}", spec.command),
    )?;
    if spec.detached {
        ExtShared::require(perms.process.detached, "process.detached")?;
    }
    let stdin = match spec.stdin {
        wp::StdioMode::Pipe => Input::Pipe,
        wp::StdioMode::Null | wp::StdioMode::InheritLog => Input::Null,
    };
    let s = Spawn {
        command: spec.command,
        args: spec.args,
        cwd: host_path(cx, spec.cwd),
        env: spec.env,
        clear_env: spec.clear_env,
        stdin,
        stdout: output_target(ext, cx, spec.stdout)?,
        stderr: output_target(ext, cx, spec.stderr)?,
        new_process_group: spec.new_process_group,
        detached: spec.detached,
        timeout: spec.timeout_ms.map(Duration::from_millis),
    };
    let detached = s.detached;
    let child = ext.processes.spawn(&s)?;
    if detached {
        ext.record_daemon(child.pid);
    }
    Ok(child)
}

fn child<T>(
    host: &mut Access<'_, T, HostState>,
    res: &Resource<ChildRes>,
) -> wasmtime::Result<Arc<process::ChildHandle>> {
    Ok(host.get().table.get(res).map(|c| Arc::clone(&c.0))?)
}

fn child_async<T>(
    accessor: &Accessor<T, HostState>,
    res: &Resource<ChildRes>,
) -> wasmtime::Result<Arc<process::ChildHandle>> {
    accessor.with(|mut a| child(&mut a, res))
}

fn spawn_allowed(ext: &ExtShared) -> Result<(), HostError> {
    ExtShared::require(ext.perms().process.spawn, "process.spawn")
}

impl wp::Host for HostState {}
impl wp::HostChild for HostState {}

impl<T: Send> wp::HostChildWithStore<T> for HostState {
    fn drop(
        accessor: &Accessor<T, Self>,
        rep: Resource<ChildRes>,
    ) -> impl Future<Output = wasmtime::Result<()>> + Send {
        let r = accessor.with(|mut a| a.get().table.delete(rep).map(|_| ()));
        std::future::ready(r.map_err(Into::into))
    }

    fn spawn(
        mut host: Access<'_, T, Self>,
        spec: wp::SpawnSpec,
    ) -> impl Future<Output = HostResult<Resource<ChildRes>>> + Send {
        let (ext, cx) = current(&mut host);
        let r = match spawn(&ext, &cx, spec) {
            Ok(c) => host
                .get()
                .table
                .push(ChildRes(c))
                .map(Ok)
                .map_err(Into::into),
            Err(e) => Ok(Err(convert::host_error(e))),
        };
        std::future::ready(r)
    }

    fn pid(
        mut host: Access<'_, T, Self>,
        self_: Resource<ChildRes>,
    ) -> impl Future<Output = wasmtime::Result<u32>> + Send {
        std::future::ready(child(&mut host, &self_).map(|c| c.pid))
    }

    async fn write_stdin(
        accessor: &Accessor<T, Self>,
        self_: Resource<ChildRes>,
        data: Vec<u8>,
    ) -> HostResult<()> {
        let c = child_async(accessor, &self_)?;
        Ok(convert::wit(c.write_stdin(&data).await))
    }

    fn close_stdin(
        mut host: Access<'_, T, Self>,
        self_: Resource<ChildRes>,
    ) -> impl Future<Output = wasmtime::Result<()>> + Send {
        std::future::ready(child(&mut host, &self_).map(|c| c.close_stdin()))
    }

    fn stdout(
        mut host: Access<'_, T, Self>,
        self_: Resource<ChildRes>,
    ) -> impl Future<Output = wasmtime::Result<Option<StreamReader<u8>>>> + Send {
        let r = child(&mut host, &self_).and_then(|c| match c.take_stdout() {
            Some(s) => Ok(Some(StreamReader::new(&mut host, ByteProducer::new(s))?)),
            None => Ok(None),
        });
        std::future::ready(r)
    }

    fn stderr(
        mut host: Access<'_, T, Self>,
        self_: Resource<ChildRes>,
    ) -> impl Future<Output = wasmtime::Result<Option<StreamReader<u8>>>> + Send {
        let r = child(&mut host, &self_).and_then(|c| match c.take_stderr() {
            Some(s) => Ok(Some(StreamReader::new(&mut host, ByteProducer::new(s))?)),
            None => Ok(None),
        });
        std::future::ready(r)
    }

    async fn wait(
        accessor: &Accessor<T, Self>,
        self_: Resource<ChildRes>,
    ) -> wasmtime::Result<wp::ExitStatus> {
        let c = child_async(accessor, &self_)?;
        Ok(exit_status(c.wait().await))
    }

    fn try_wait(
        mut host: Access<'_, T, Self>,
        self_: Resource<ChildRes>,
    ) -> impl Future<Output = wasmtime::Result<Option<wp::ExitStatus>>> + Send {
        std::future::ready(child(&mut host, &self_).map(|c| c.try_wait().map(exit_status)))
    }

    fn signal(
        mut host: Access<'_, T, Self>,
        self_: Resource<ChildRes>,
        sig: wp::Signal,
        group: bool,
    ) -> impl Future<Output = HostResult<()>> + Send {
        let sig = match sig {
            wp::Signal::Term => Sig::Term,
            wp::Signal::Kill => Sig::Kill,
            wp::Signal::Int => Sig::Int,
            wp::Signal::Hup => Sig::Hup,
        };
        std::future::ready(child(&mut host, &self_).map(|c| convert::wit(c.signal(sig, group))))
    }
}

impl<T: Send> wp::HostWithStore<T> for HostState {
    async fn exec(
        accessor: &Accessor<T, Self>,
        command: String,
        args: Vec<String>,
        options: wp::ExecOptions,
    ) -> HostResult<wp::ExecResult> {
        let (ext, cx) = current_async(accessor);
        let r = match ExtShared::require(
            ext.perms().allows_program(&command, ext.bundled),
            &format!("process.allow:{command}"),
        ) {
            Ok(()) => {
                let cwd = host_path(&cx, options.cwd);
                process::exec(
                    &command,
                    &args,
                    &cwd,
                    &options.env,
                    options.timeout_ms.map(Duration::from_millis),
                )
                .await
                .map(|o| wp::ExecResult {
                    stdout: o.stdout,
                    stderr: o.stderr,
                    code: o.code,
                    killed: o.killed,
                })
            }
            Err(e) => Err(e),
        };
        Ok(convert::wit(r))
    }

    fn process_tree(
        mut host: Access<'_, T, Self>,
        pid: u32,
    ) -> impl Future<Output = HostResult<Vec<u32>>> + Send {
        let (ext, _) = current(&mut host);
        ready_wit(spawn_allowed(&ext).and_then(|()| process::process_tree(pid)))
    }

    fn process_stats(
        mut host: Access<'_, T, Self>,
        pid: u32,
    ) -> impl Future<Output = HostResult<wp::ProcStats>> + Send {
        let (ext, _) = current(&mut host);
        let r = spawn_allowed(&ext)
            .and_then(|()| process::process_stats(pid))
            .map(|s| wp::ProcStats {
                pid: s.pid,
                ppid: s.ppid,
                state: s.state,
                cpu_ms: s.cpu_ms,
                rss_bytes: s.rss_bytes,
                read_bytes: s.read_bytes,
                write_bytes: s.write_bytes,
                start_time_ms: s.start_time_ms,
            });
        ready_wit(r)
    }

    fn self_pid(_host: Access<'_, T, Self>) -> impl Future<Output = wasmtime::Result<u32>> + Send {
        ready(std::process::id())
    }
}

// ── http ─────────────────────────────────────────────────────────────────

impl wh::Host for HostState {}
impl wh::HostListener for HostState {}

async fn fetch(
    ext: Arc<ExtShared>,
    cx: CallContext,
    req: wh::Request,
) -> Result<
    (
        wh::ResponseHead,
        futures::stream::BoxStream<'static, Vec<u8>>,
    ),
    HostError,
> {
    let url: reqwest::Url = req
        .url
        .parse()
        .map_err(|e| HostError::InvalidArgument(format!("url: {e}")))?;
    let host = crate::http::host_of(&url)
        .ok_or_else(|| HostError::InvalidArgument("url has no host".into()))?;
    let allowed = ext.perms().allows_host(&host, ext.bundled) || ext.http_grants.allows(&host);
    ExtShared::require(allowed, &format!("network.hosts:{host}"))?;
    if req.impersonate.is_some() {
        ExtShared::require(ext.perms().network.fingerprint, "network.fingerprint")?;
    }
    let mut headers = req.headers;
    if let wh::Auth::Provider(provider) = &req.auth {
        ExtShared::require(
            ext.perms().allows_credential(provider),
            &format!("credentials:{provider}"),
        )?;
        match ext
            .host
            .models(&ext.id, &cx, ModelsOp::CredentialFor(provider.clone()))
            .await?
        {
            oino_extension_api::ops::ModelsReply::Credential(Some(c)) => headers.extend(c.headers),
            _ => return Err(HostError::NotFound(format!("credential for {provider}"))),
        }
    }
    let method = match req.method {
        wh::Method::Get => reqwest::Method::GET,
        wh::Method::Head => reqwest::Method::HEAD,
        wh::Method::Post => reqwest::Method::POST,
        wh::Method::Put => reqwest::Method::PUT,
        wh::Method::Patch => reqwest::Method::PATCH,
        wh::Method::Delete => reqwest::Method::DELETE,
        wh::Method::Options => reqwest::Method::OPTIONS,
    };
    let (head, body) = ext
        .http
        .fetch(crate::http::Request {
            method,
            url,
            headers,
            body: req.body,
            timeout: req.timeout_ms.map(|t| Duration::from_millis(u64::from(t))),
            follow_redirects: req.follow_redirects,
            proxy: req.proxy,
        })
        .await?;
    Ok((
        wh::ResponseHead {
            status: head.status,
            headers: head.headers,
            final_url: head.final_url,
            impersonated: false,
        },
        body,
    ))
}

impl<T: Send> wh::HostWithStore<T> for HostState {
    async fn fetch(
        accessor: &Accessor<T, Self>,
        req: wh::Request,
    ) -> HostResult<(wh::ResponseHead, StreamReader<u8>)> {
        let (ext, cx) = current_async(accessor);
        match fetch(ext, cx, req).await {
            Ok((head, body)) => {
                let reader =
                    accessor.with(|mut a| StreamReader::new(&mut a, ByteProducer::new(body)))?;
                Ok(Ok((head, reader)))
            }
            Err(e) => Ok(Err(convert::host_error(e))),
        }
    }
}

fn listener<T>(
    host: &mut Access<'_, T, HostState>,
    res: &Resource<ListenerRes>,
) -> wasmtime::Result<Arc<crate::http::Listener>> {
    Ok(host.get().table.get(res).map(|l| Arc::clone(&l.0))?)
}

impl<T: Send> wh::HostListenerWithStore<T> for HostState {
    fn drop(
        accessor: &Accessor<T, Self>,
        rep: Resource<ListenerRes>,
    ) -> impl Future<Output = wasmtime::Result<()>> + Send {
        let r = accessor.with(|mut a| a.get().table.delete(rep).map(|_| ()));
        std::future::ready(r.map_err(Into::into))
    }

    fn bind(
        mut host: Access<'_, T, Self>,
        port: u16,
    ) -> impl Future<Output = HostResult<Resource<ListenerRes>>> + Send {
        let (ext, _) = current(&mut host);
        let allowed = ExtShared::require(
            ext.perms().network.listen_loopback,
            "network.listenLoopback",
        );
        // Binding is synchronous at the std level; tokio adopts the socket.
        let r = allowed.and_then(|()| {
            let std_listener =
                std::net::TcpListener::bind(("127.0.0.1", port)).map_err(|e| HostError::io(&e))?;
            std_listener
                .set_nonblocking(true)
                .map_err(|e| HostError::io(&e))?;
            crate::http::Listener::from_std(std_listener)
        });
        let out = match r {
            Ok(l) => host
                .get()
                .table
                .push(ListenerRes(Arc::new(l)))
                .map(Ok)
                .map_err(Into::into),
            Err(e) => Ok(Err(convert::host_error(e))),
        };
        std::future::ready(out)
    }

    fn port(
        mut host: Access<'_, T, Self>,
        self_: Resource<ListenerRes>,
    ) -> impl Future<Output = wasmtime::Result<u16>> + Send {
        std::future::ready(listener(&mut host, &self_).map(|l| l.port()))
    }

    async fn accept(
        accessor: &Accessor<T, Self>,
        self_: Resource<ListenerRes>,
    ) -> wasmtime::Result<Option<wh::Incoming>> {
        let l = accessor.with(|mut a| listener(&mut a, &self_))?;
        Ok(l.accept().await.map(|i| wh::Incoming {
            id: i.id,
            method: i.method,
            path: i.path,
            headers: i.headers,
            body: i.body,
        }))
    }

    fn respond(
        mut host: Access<'_, T, Self>,
        self_: Resource<ListenerRes>,
        id: u64,
        status: u16,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    ) -> impl Future<Output = HostResult<()>> + Send {
        std::future::ready(
            listener(&mut host, &self_).map(|l| convert::wit(l.respond(id, status, headers, body))),
        )
    }
}

// ── fs ───────────────────────────────────────────────────────────────────

impl wf::Host for HostState {}
impl wf::HostLock for HostState {}
impl wf::HostWatcher for HostState {}

impl<T: Send> wf::HostWithStore<T> for HostState {
    fn preopens(
        mut host: Access<'_, T, Self>,
    ) -> impl Future<Output = wasmtime::Result<Vec<wf::Preopen>>> + Send {
        let (ext, cx) = current(&mut host);
        let list = ext
            .preopens(&cx)
            .into_iter()
            .map(|p| wf::Preopen {
                guest_path: p.guest_path(),
                host_path: p.host.to_string_lossy().into_owned(),
                name: p.name,
                writable: p.writable,
            })
            .collect();
        ready(list)
    }

    fn atomic_write(
        mut host: Access<'_, T, Self>,
        path: String,
        data: Vec<u8>,
        mode: Option<u32>,
    ) -> impl Future<Output = HostResult<()>> + Send {
        let (ext, cx) = current(&mut host);
        let r = paths::resolve(&ext.preopens(&cx), &path, true)
            .and_then(|p| crate::kv::atomic_write(&p, &data, mode).map_err(|e| HostError::io(&e)));
        ready_wit(r)
    }
}

impl<T: Send> wf::HostLockWithStore<T> for HostState {
    fn drop(
        accessor: &Accessor<T, Self>,
        rep: Resource<LockRes>,
    ) -> impl Future<Output = wasmtime::Result<()>> + Send {
        let r = accessor
            .with(|mut a| a.get().table.delete(rep))
            .map(|mut lock| lock.release());
        std::future::ready(r.map_err(Into::into))
    }

    async fn acquire(
        accessor: &Accessor<T, Self>,
        path: String,
        exclusive: bool,
    ) -> HostResult<Resource<LockRes>> {
        let (ext, cx) = current_async(accessor);
        let r = match paths::resolve(&ext.preopens(&cx), &path, true) {
            Ok(p) => LockRes::acquire(&p, exclusive).await,
            Err(e) => Err(e),
        };
        match r {
            Ok(lock) => Ok(Ok(accessor.with(|mut a| a.get().table.push(lock))?)),
            Err(e) => Ok(Err(convert::host_error(e))),
        }
    }

    fn release(
        mut host: Access<'_, T, Self>,
        self_: Resource<LockRes>,
    ) -> impl Future<Output = wasmtime::Result<()>> + Send {
        let r = host.get().table.get_mut(&self_).map(LockRes::release);
        std::future::ready(r.map_err(Into::into))
    }
}

impl<T: Send> wf::HostWatcherWithStore<T> for HostState {
    fn drop(
        accessor: &Accessor<T, Self>,
        rep: Resource<WatcherRes>,
    ) -> impl Future<Output = wasmtime::Result<()>> + Send {
        let r = accessor.with(|mut a| a.get().table.delete(rep).map(|_| ()));
        std::future::ready(r.map_err(Into::into))
    }

    fn watch(
        mut host: Access<'_, T, Self>,
        path: String,
        recursive: bool,
    ) -> impl Future<Output = HostResult<Resource<WatcherRes>>> + Send {
        let (ext, cx) = current(&mut host);
        let r = paths::resolve(&ext.preopens(&cx), &path, false)
            .and_then(|p| WatcherRes::watch(&p, path.trim_end_matches('/').to_owned(), recursive));
        let out = match r {
            Ok(w) => host.get().table.push(w).map(Ok).map_err(Into::into),
            Err(e) => Ok(Err(convert::host_error(e))),
        };
        std::future::ready(out)
    }

    async fn next(
        accessor: &Accessor<T, Self>,
        self_: Resource<WatcherRes>,
    ) -> wasmtime::Result<Option<Vec<wf::FsEvent>>> {
        let rx = accessor.with(|mut a| a.get().table.get(&self_).map(|w| Arc::clone(&w.rx)))?;
        let batch = rx.lock().await.recv().await;
        Ok(batch.map(|b| {
            b.into_iter()
                .map(|c| wf::FsEvent {
                    kind: c.kind.to_owned(),
                    path: c.path,
                })
                .collect()
        }))
    }
}

// ── timers ───────────────────────────────────────────────────────────────

impl wtm::Host for HostState {}

impl<T: Send> wtm::HostWithStore<T> for HostState {
    fn set(
        mut host: Access<'_, T, Self>,
        id: String,
        delay_ms: u64,
        repeat: bool,
    ) -> impl Future<Output = wasmtime::Result<()>> + Send {
        let (ext, cx) = current(&mut host);
        let weak = Arc::downgrade(&ext);
        let session = cx.session_id;
        ext.timers.set(
            &id,
            Duration::from_millis(delay_ms),
            repeat,
            Arc::new(move |timer_id: &str| {
                if let Some(ext) = weak.upgrade() {
                    let payload = RawJson::from(json!({"type": "oino_timer", "timerId": timer_id}));
                    ext.host
                        .deliver_event(&ext.id, &session, EventKind::OinoTimer, payload);
                }
            }),
        );
        ready(())
    }

    fn cancel(
        mut host: Access<'_, T, Self>,
        id: String,
    ) -> impl Future<Output = wasmtime::Result<()>> + Send {
        let (ext, _) = current(&mut host);
        ext.timers.cancel(&id);
        ready(())
    }

    async fn sleep(_accessor: &Accessor<T, Self>, ms: u64) -> wasmtime::Result<()> {
        tokio::time::sleep(Duration::from_millis(ms)).await;
        Ok(())
    }

    fn now_ms(_host: Access<'_, T, Self>) -> impl Future<Output = wasmtime::Result<i64>> + Send {
        ready(crate::timers::now_ms())
    }
}

// ── kv ───────────────────────────────────────────────────────────────────

fn scope_dir(ext: &ExtShared, cx: &CallContext, scope: wk::Scope) -> PathBuf {
    match scope {
        wk::Scope::Global => ext.paths.global.clone(),
        wk::Scope::Workspace => ext.paths.workspace(&paths::workspace_info(&cx.cwd).id),
        wk::Scope::Session => ext.paths.session(&cx.session_id),
    }
}

impl wk::Host for HostState {}

impl<T: Send> wk::HostWithStore<T> for HostState {
    fn get(
        mut host: Access<'_, T, Self>,
        scope: wk::Scope,
        key: String,
    ) -> impl Future<Output = wasmtime::Result<Option<Vec<u8>>>> + Send {
        let (ext, cx) = current(&mut host);
        ready(ext.kv.get(&scope_dir(&ext, &cx, scope), &key))
    }

    fn set(
        mut host: Access<'_, T, Self>,
        scope: wk::Scope,
        key: String,
        value: Vec<u8>,
    ) -> impl Future<Output = HostResult<()>> + Send {
        let (ext, cx) = current(&mut host);
        ready_wit(ext.kv.set(&scope_dir(&ext, &cx, scope), &key, &value))
    }

    fn delete(
        mut host: Access<'_, T, Self>,
        scope: wk::Scope,
        key: String,
    ) -> impl Future<Output = wasmtime::Result<()>> + Send {
        let (ext, cx) = current(&mut host);
        ext.kv.delete(&scope_dir(&ext, &cx, scope), &key);
        ready(())
    }

    fn keys(
        mut host: Access<'_, T, Self>,
        scope: wk::Scope,
        prefix: String,
    ) -> impl Future<Output = wasmtime::Result<Vec<String>>> + Send {
        let (ext, cx) = current(&mut host);
        ready(ext.kv.keys(&scope_dir(&ext, &cx, scope), &prefix))
    }
}
