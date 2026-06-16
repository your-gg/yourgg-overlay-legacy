//! Out-of-process overlay injector helper.
//!
//! Installs the overlay hook into the target via `SetWindowsHookExW`, waits
//! until the injected DLL's IPC server is up (= DLL mapped + self-pinned), then
//! exits. Because the DLL pins itself on first hook fire, the hook can be
//! removed (this process exiting) while the overlay keeps running; a separate
//! client (e.g. the Electron addon) then connects to the IPC pipe.
//!
//! Why a separate exe: calling `SetWindowsHookExW` from a large host process
//! (Electron/Chromium) draws heavy anti-cheat scrutiny and can block for many
//! seconds; from a small dedicated, code-signed exe it is near-instant. The
//! helper must be the SAME architecture as the target process.
//!
//! Usage:  injector-helper <pid> <dll-path>
//! Exit:   0 = hook installed and the overlay's IPC server is up
//!         1 = bad args / install failed / timed out (cause written to stderr)

use std::{env, path::PathBuf, process::ExitCode, time::Duration};

use anyhow::Context;
use asdf_overlay_client::{OverlayDll, install_hook, wait_for_ipc};

/// Upper bound for the IPC-server wait after the hook is installed.
const TIMEOUT: Duration = Duration::from_secs(60);

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            // The parent (addon) captures this process's stderr and surfaces it.
            eprintln!("[injector-helper] failed: {err:?}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> anyhow::Result<()> {
    let mut args = env::args().skip(1);
    let pid: u32 = args
        .next()
        .context("usage: injector-helper <pid> <dll-path>")?
        .parse()
        .context("invalid pid")?;
    let dll_path = PathBuf::from(
        args.next()
            .context("usage: injector-helper <pid> <dll-path>")?,
    );

    // Same path in every arch slot: install_hook reads only the slot matching
    // THIS helper's architecture, and the caller spawns the helper whose arch
    // matches the target — so the passed path is always the correct one.
    install_hook(
        pid,
        OverlayDll {
            x64: Some(&dll_path),
            x86: Some(&dll_path),
            arm64: Some(&dll_path),
        },
        Some(TIMEOUT),
    )?;

    // Block until the DLL has mapped, pinned itself, and started its IPC server,
    // so the overlay survives this process exiting (which removes the hook).
    wait_for_ipc(pid, Some(TIMEOUT)).await
}
