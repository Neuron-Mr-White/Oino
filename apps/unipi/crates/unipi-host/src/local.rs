//! UNI-229 + v2 (specs/dev/v2/specs/host.md §6.1): the local listener —
//! `<dir>/app.sock` on Unix, the named pipe `\\.\pipe\unipi-oino-app` on
//! Windows (new in v2; unipi-app never shipped one). The UniPi desktop app
//! on this PC reaches its own host here — no iroh, no relay, works offline.
//! Each CONNECTION is one logical op, exactly like one QUIC bi-stream: the
//! same `u8 kind · u32 len` frames and the same first `Hello`, handled by
//! the same `Handler::handle` with `Peer::Local`.
//!
//! Trust: only processes of the same user. Unix checks the peer uid
//! (SO_PEERCRED); Windows creates the pipe with an owner-only DACL and then
//! re-checks the connected client's process user SID
//! (`GetNamedPipeClientProcessId` → process token) — either gate failing
//! refuses the connection. That user can already read `secret.key`,
//! `devices.json` and the oino sessions, so this gives nothing new away.
//! On Unix the socket file is 0600 inside the 0700 data dir as a second
//! fence.

use crate::{data_dir::HostPaths, protocol::Handler};

impl HostPaths {
    /// `<dir>/app.sock` (Unix; on Windows the listener is a named pipe).
    pub fn app_socket(&self) -> std::path::PathBuf {
        self.root.join("app.sock")
    }

    /// The local listener's address as a string, for clients: the socket
    /// path on Unix, the pipe name on Windows (spec §5.1/§6.1).
    pub fn local_addr(&self) -> String {
        #[cfg(unix)]
        {
            self.app_socket().to_string_lossy().into_owned()
        }
        #[cfg(windows)]
        {
            let _ = self;
            PIPE_NAME.to_string()
        }
    }
}

/// The Windows pipe name (spec §5.1 coexistence table: the old unipi has
/// no pipe, and any future one of theirs gets a different name).
#[cfg(windows)]
pub const PIPE_NAME: &str = r"\\.\pipe\unipi-oino-app";

// ---------------------------------------------------------------- Unix --

/// Is this peer the same user as us?
#[cfg(unix)]
pub fn same_user(stream: &tokio::net::UnixStream) -> bool {
    match stream.peer_cred() {
        Ok(cred) => cred.uid() == unsafe { libc::geteuid() },
        Err(error) => {
            tracing::warn!(%error, "app.sock: can't read the peer's uid; refusing");
            false
        }
    }
}

/// Bind `app.sock` and serve it until the returned task is aborted (or the
/// listener fails). The caller holds the instance lock, so a leftover socket
/// file from a dead host is ours to replace.
#[cfg(unix)]
pub fn serve(handler: Handler) -> std::io::Result<tokio::task::JoinHandle<()>> {
    use std::os::unix::fs::PermissionsExt;
    let path = handler.paths.app_socket();
    let _ = std::fs::remove_file(&path);
    let listener = tokio::net::UnixListener::bind(&path)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    tracing::info!(path = %path.display(), "local socket ready");
    Ok(tokio::spawn(async move {
        loop {
            let (stream, _) = match listener.accept().await {
                Ok(pair) => pair,
                Err(error) => {
                    tracing::warn!(%error, "app.sock accept failed");
                    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                    continue;
                }
            };
            if !same_user(&stream) {
                tracing::warn!("app.sock: refused a connection from another user");
                continue;
            }
            let handler = handler.clone();
            tokio::spawn(async move {
                let (read, write) = stream.into_split();
                if let Err(error) = handler
                    .handle(read, write, crate::protocol::Peer::Local)
                    .await
                {
                    tracing::debug!(%error, "local stream ended");
                }
            });
        }
    }))
}

/// Remove the socket file (shutdown).
#[cfg(unix)]
pub fn cleanup(paths: &HostPaths) {
    let _ = std::fs::remove_file(paths.app_socket());
}

// -------------------------------------------------------------- Windows --

#[cfg(windows)]
pub fn serve(handler: Handler) -> std::io::Result<tokio::task::JoinHandle<()>> {
    let owner = OwnerPipe::new()?;
    let first = owner.create(true)?;
    tracing::info!(pipe = PIPE_NAME, "local pipe ready");
    Ok(tokio::spawn(async move {
        let mut server = first;
        loop {
            if let Err(error) = server.connect().await {
                tracing::warn!(%error, "pipe connect failed");
                match owner.create(false) {
                    Ok(next) => {
                        server = next;
                        continue;
                    }
                    Err(recreate) => {
                        tracing::error!(%recreate, "recreating the pipe failed; local listener stops");
                        return;
                    }
                }
            }
            if !same_user(&server) {
                tracing::warn!("pipe: refused a connection from another user");
                match owner.create(false) {
                    Ok(next) => {
                        server = next;
                        continue;
                    }
                    Err(recreate) => {
                        tracing::error!(%recreate, "recreating the pipe failed; local listener stops");
                        return;
                    }
                }
            }
            let client = match owner.create(false) {
                Ok(next) => std::mem::replace(&mut server, next),
                Err(error) => {
                    tracing::error!(%error, "creating the next pipe instance failed; local listener stops");
                    return;
                }
            };
            let handler = handler.clone();
            tokio::spawn(async move {
                // One stream split into halves, exactly like a QUIC bi-stream.
                let (read, write) = tokio::io::split(client);
                if let Err(error) = handler
                    .handle(read, write, crate::protocol::Peer::Local)
                    .await
                {
                    tracing::debug!(%error, "local stream ended");
                }
            });
        }
    }))
}

/// Nothing to remove on shutdown: the pipe disappears with the process.
#[cfg(windows)]
pub fn cleanup(_paths: &HostPaths) {}

/// The owner-only security descriptor (a DACL granting the current user
/// FILE_GENERIC_READ|WRITE) the pipe instances are created with.
#[cfg(windows)]
struct OwnerPipe {
    descriptor: Box<windows_sys::Win32::Security::SECURITY_DESCRIPTOR>,
    acl: *mut windows_sys::Win32::Security::ACL,
}

#[cfg(windows)]
impl OwnerPipe {
    /// SAFETY for the `Send` impl below: the descriptor is owned heap
    /// memory and the ACL pointer is only dereferenced (passed by pointer
    /// into `CreateNamedPipeW`, which copies it) from the one listener
    /// task that owns this value.
    fn new() -> std::io::Result<Self> {
        use windows_sys::Win32::Foundation::{LocalFree, WIN32_ERROR};
        use windows_sys::Win32::Security;
        use windows_sys::Win32::Security::Authorization::{
            SetEntriesInAclW, EXPLICIT_ACCESS_W, GRANT_ACCESS, TRUSTEE_IS_SID, TRUSTEE_IS_UNKNOWN,
            TRUSTEE_W,
        };
        use windows_sys::Win32::Storage::FileSystem::{FILE_GENERIC_READ, FILE_GENERIC_WRITE};
        use windows_sys::Win32::System::SystemServices::SECURITY_DESCRIPTOR_REVISION;

        let mut own_sid = process_user_sid(None)
            .ok_or_else(|| std::io::Error::other("can't read this process's user"))?;
        let mut trustee = TRUSTEE_W::default();
        trustee.TrusteeForm = TRUSTEE_IS_SID;
        trustee.TrusteeType = TRUSTEE_IS_UNKNOWN;
        trustee.ptstrName = own_sid.as_mut_ptr().cast();
        let entries = [EXPLICIT_ACCESS_W {
            grfAccessPermissions: FILE_GENERIC_READ | FILE_GENERIC_WRITE,
            grfAccessMode: GRANT_ACCESS,
            grfInheritance: 0,
            Trustee: trustee,
        }];
        let mut acl = std::ptr::null_mut();
        let status: WIN32_ERROR =
            unsafe { SetEntriesInAclW(1, entries.as_ptr(), std::ptr::null(), &mut acl) };
        if status != 0 {
            return Err(std::io::Error::other(format!(
                "SetEntriesInAclW failed (win32 error {status})"
            )));
        }
        let mut descriptor = Box::new(Security::SECURITY_DESCRIPTOR::default());
        let sd_ptr: Security::PSECURITY_DESCRIPTOR =
            (&mut *descriptor as *mut Security::SECURITY_DESCRIPTOR).cast();
        if unsafe { Security::InitializeSecurityDescriptor(sd_ptr, SECURITY_DESCRIPTOR_REVISION) }
            == 0
        {
            unsafe { LocalFree(acl.cast()) };
            return Err(std::io::Error::last_os_error());
        }
        if unsafe { Security::SetSecurityDescriptorDacl(sd_ptr, 1, acl, 0) } == 0 {
            unsafe { LocalFree(acl.cast()) };
            return Err(std::io::Error::last_os_error());
        }
        Ok(OwnerPipe { descriptor, acl })
    }

    /// Creates one pipe instance with the owner-only DACL. `first` claims
    /// the name (fails if any instance already exists — a second v2 host).
    fn create(
        &self,
        first: bool,
    ) -> std::io::Result<tokio::net::windows::named_pipe::NamedPipeServer> {
        use windows_sys::Win32::Security::{self, SECURITY_ATTRIBUTES};
        let mut attrs = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: (&*self.descriptor as *const Security::SECURITY_DESCRIPTOR)
                .cast_mut()
                .cast(),
            bInheritHandle: 0,
        };
        let mut options = tokio::net::windows::named_pipe::ServerOptions::new();
        options.first_pipe_instance(first);
        // SAFETY: `attrs` is a valid SECURITY_ATTRIBUTES for the duration of
        // the call; CreateNamedPipeW copies the descriptor. tokio links the
        // same windows-sys 0.61 this crate links, so the layout matches.
        unsafe {
            options.create_with_security_attributes_raw(
                PIPE_NAME,
                std::ptr::from_mut(&mut attrs).cast(),
            )
        }
    }
}

#[cfg(windows)]
// SAFETY: see `OwnerPipe::new` — owned heap state, no shared mutable access.
unsafe impl Send for OwnerPipe {}

#[cfg(windows)]
impl Drop for OwnerPipe {
    fn drop(&mut self) {
        use windows_sys::Win32::Foundation::LocalFree;
        unsafe { LocalFree(self.acl.cast()) };
    }
}

/// The user SID of a process as an aligned copy (`None` = this process).
/// The connection check on Windows (uid-equivalent, spec §6.1).
#[cfg(windows)]
fn process_user_sid(pid: Option<u32>) -> Option<Vec<u32>> {
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::Security::{GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER};
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    struct Handle(HANDLE);
    impl Drop for Handle {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { CloseHandle(self.0) };
            }
        }
    }

    // Keeps the `OpenProcess` handle alive until the token is read.
    let process_handle: Option<Handle>;
    let process = match pid {
        // A pseudo handle: must not be closed.
        None => unsafe { GetCurrentProcess() },
        Some(pid) => {
            let opened = Handle(unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) });
            if opened.0.is_null() {
                return None;
            }
            process_handle = Some(opened);
            process_handle
                .as_ref()
                .map(|h| h.0)
                .unwrap_or(std::ptr::null_mut())
        }
    };
    let mut token = std::ptr::null_mut();
    if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) } == 0 {
        return None;
    }
    let token = Handle(token);
    let mut len = 0u32;
    if unsafe { GetTokenInformation(token.0, TokenUser, std::ptr::null_mut(), 0, &mut len) } == 0
        && len == 0
    {
        return None;
    }
    let mut buffer = vec![0u32; len.div_ceil(4) as usize];
    let ok = unsafe {
        GetTokenInformation(
            token.0,
            TokenUser,
            buffer.as_mut_ptr().cast(),
            len,
            &mut len,
        )
    } != 0;
    if !ok {
        return None;
    }
    let user: &TOKEN_USER = unsafe { &*buffer.as_ptr().cast() };
    let sid = user.User.Sid;
    if sid.is_null() {
        return None;
    }
    let sid_len = unsafe { windows_sys::Win32::Security::GetLengthSid(sid) } as usize;
    if sid_len == 0 || sid_len > buffer.len() * 4 {
        return None;
    }
    let mut out = vec![0u32; sid_len.div_ceil(4)];
    unsafe {
        std::ptr::copy_nonoverlapping(
            sid.cast_const().cast::<u8>(),
            out.as_mut_ptr().cast(),
            sid_len,
        )
    };
    Some(out)
}

/// Is the connected pipe client the same user as us? (fail closed)
#[cfg(windows)]
pub fn same_user(server: &tokio::net::windows::named_pipe::NamedPipeServer) -> bool {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::Security::EqualSid;
    use windows_sys::Win32::System::Pipes::GetNamedPipeClientProcessId;

    let mut pid = 0u32;
    if unsafe { GetNamedPipeClientProcessId(server.as_raw_handle() as HANDLE, &mut pid) } == 0 {
        tracing::warn!("pipe: can't read the client's pid; refusing");
        return false;
    }
    let Some(client_sid) = process_user_sid(Some(pid)) else {
        tracing::warn!(pid, "pipe: can't read the client's user; refusing");
        return false;
    };
    let Some(own_sid) = process_user_sid(None) else {
        tracing::warn!("pipe: can't read our own user; refusing local clients");
        return false;
    };
    let equal = unsafe {
        EqualSid(
            own_sid.as_ptr().cast_mut().cast(),
            client_sid.as_ptr().cast_mut().cast(),
        )
    } != 0;
    if !equal {
        tracing::warn!(pid, "pipe: the client runs as another user; refusing");
    }
    equal
}

#[cfg(all(test, unix))]
mod tests {
    #[tokio::test]
    async fn same_user_accepts_our_own_process() {
        let (a, _b) = tokio::net::UnixStream::pair().unwrap();
        assert!(super::same_user(&a));
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn own_sid_is_readable_and_stable() {
        let a = process_user_sid(None).expect("our sid");
        let b = process_user_sid(None).expect("our sid");
        assert_eq!(a, b);
        assert!(!a.is_empty());
        // The prefix of any account SID is S-1 (little-endian 1).
        assert_eq!(a[0] & 0xff, 1);
    }

    #[test]
    fn pipe_name_is_the_v2_one() {
        assert_eq!(PIPE_NAME, r"\\.\pipe\unipi-oino-app");
        assert!(crate::ipc::is_pipe(PIPE_NAME));
    }
}
