//! Library for attaching `asdf-overlay` to a process and initiating IPC channel.
//!
//! By utilizing this library, you can render overlay from any process and control it via IPC.
//! It's designed to give you maximum flexibility as you can keep most of the logic in this process.
//!
//! # Example
//! ```no_run
//! use std::path::Path;
//! use std::time::Duration;
//! use asdf_overlay_client::{inject, OverlayDll};
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let dll = OverlayDll {
//!         x64: Some(Path::new("asdf-overlay-x64.dll")),
//!         x86: Some(Path::new("asdf-overlay-x86.dll")),
//!         arm64: Some(Path::new("asdf-overlay-arm64.dll")),
//!     };
//!
//!    let (mut conn, mut events) = inject(
//!         1234, // target process pid
//!         dll, // overlay dll paths
//!         Some(Duration::from_secs(10)), // timeout for injection and ipc connection
//!    ).await?;
//!
//!   // Use `conn` to send requests to overlay, and `events` to receive events from the overlay.
//!
//!   Ok(())
//! }
//!

pub mod client;
mod injector;
#[cfg(feature = "surface")]
pub mod surface;
#[cfg(feature = "surface")]
pub mod ty;

pub use asdf_overlay_common as common;
pub use asdf_overlay_event as event;

use core::time::Duration;
use std::path::Path;

use anyhow::Context;
use asdf_overlay_common::ipc::create_ipc_addr;
use tokio::{net::windows::named_pipe::ClientOptions, time::sleep};

use crate::client::{IpcClientConn, IpcClientEventStream};

/// Paths to overlay DLLs for different architectures.
#[derive(Debug, Clone, Copy, Default)]
pub struct OverlayDll<'a> {
    /// Path to DLL to be used for x64 applications.
    pub x64: Option<&'a Path>,

    /// Path to DLL to be used for x86 applications.
    pub x86: Option<&'a Path>,

    /// Path to DLL to be used for ARM64 applications.
    pub arm64: Option<&'a Path>,
}

/// Inject overlay DLL into target process and create IPC connection.
/// * If you didn't supply DLL path for the target architecture, it will return an error.
/// * If injection or IPC connection fails, it will return an error.
/// * If timeout is `None`, it may wait indefinitely.
///
/// This installs the hook *in the calling process*. When the caller is a large
/// host process (e.g. an Electron/Chromium app), the anti-cheat scrutinizes the
/// hook install and `SetWindowsHookExW` can block for many seconds. To avoid
/// that, run [`install_hook`] from a small dedicated (ideally signed) helper exe
/// and then call [`connect`] from the host.
pub async fn inject(
    pid: u32,
    dll: OverlayDll<'_>,
    timeout: Option<Duration>,
) -> anyhow::Result<(IpcClientConn, IpcClientEventStream)> {
    install_hook(pid, dll, timeout)?;
    connect(pid, timeout).await
}

/// Install the overlay hook into the target via `SetWindowsHookExW`. Returns once
/// the hook is registered and nudged; the injected DLL then maps, pins itself,
/// and starts its IPC server inside the target asynchronously.
///
/// Run this from a small dedicated helper process (not a large host like
/// Electron) to keep the anti-cheat validation cost low. After it returns, wait
/// for the IPC server with [`wait_for_ipc`] (helper side) or [`connect`] (client
/// side).
///
/// NOTE: `timeout` is not enforced here — the hook install is a synchronous,
/// uninterruptible Win32 syscall that a tokio timeout cannot cancel. Bound a
/// wedged install at the process boundary instead (kill the helper).
pub fn install_hook(
    pid: u32,
    dll: OverlayDll<'_>,
    timeout: Option<Duration>,
) -> anyhow::Result<()> {
    injector::inject(pid, dll, timeout).context("failed to inject overlay DLL")
}

/// Connect to the overlay's IPC server, which the injected DLL starts inside the
/// target after [`install_hook`] fires. The pipe may not exist immediately, so
/// the connect is retried until it succeeds (or `timeout` elapses).
pub async fn connect(
    pid: u32,
    timeout: Option<Duration>,
) -> anyhow::Result<(IpcClientConn, IpcClientEventStream)> {
    let ipc_addr = create_ipc_addr(pid);

    let connect = async {
        loop {
            match ClientOptions::new().open(&ipc_addr) {
                Ok(client) => break IpcClientConn::new(client).await,
                Err(_) => sleep(Duration::from_millis(50)).await,
            }
        }
    };

    match timeout {
        Some(dur) => tokio::time::timeout(dur, connect)
            .await
            .map_err(|_| anyhow::anyhow!("ipc client wait timeout"))?,
        None => connect.await,
    }
}

/// Wait until the overlay's IPC server is up (the pipe becomes openable), without
/// holding a connection. Used by the injector helper process to confirm the DLL
/// has mapped and pinned itself before exiting — once this returns, the hook can
/// be safely removed (helper exit) and a real client may [`connect`].
pub async fn wait_for_ipc(pid: u32, timeout: Option<Duration>) -> anyhow::Result<()> {
    let ipc_addr = create_ipc_addr(pid);

    // Probe pipe existence by opening then immediately dropping the handle. The
    // DLL's server loop tolerates this transient connect and re-arms a fresh
    // instance for the real client.
    let wait = async {
        loop {
            match ClientOptions::new().open(&ipc_addr) {
                Ok(_probe) => break,
                Err(_) => sleep(Duration::from_millis(50)).await,
            }
        }
    };

    match timeout {
        Some(dur) => tokio::time::timeout(dur, wait)
            .await
            .map_err(|_| anyhow::anyhow!("ipc server wait timeout"))?,
        None => wait.await,
    }
    Ok(())
}
