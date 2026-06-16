//! De-risk probe: measure `SetWindowsHookExW` latency when the CALLER is a small
//! standalone exe instead of the Electron native addon.
//!
//! Confirmed bottleneck (live, signed DLL): the single `SetWindowsHookExW`
//! syscall blocks ~13-16s when the caller is the Electron addon (Vanguard
//! validating the hook injection into the protected `League of Legends.exe`).
//! A1 `LoadLibraryW` and A2 `find_gui_thread` are sub-ms — the cost is purely
//! that one syscall.
//!
//! This probe runs the IDENTICAL code path (`asdf_overlay_client::inject`, which
//! prints A1/A2/A3 + phase B). The ONLY changed variable is the caller process:
//! a tiny standalone exe instead of Electron/Chromium. So:
//!
//!   * A3 fast here (sub-second) -> the cost was tied to the Electron caller;
//!     an external injector exe is the fix. Then test a SIGNED build to confirm.
//!   * A3 still ~13-16s here     -> Vanguard validates ANY hook injection into
//!     League regardless of caller; an external injector won't help. Pivot to an
//!     external layered window (the verified Blitz approach — Blitz does NOT
//!     inject at all).
//!
//! Signing: for a clean caller-trust test the probe should be code-signed with
//! the same cert as the DLL. UNSIGNED still answers "fast vs slow"; SIGNED
//! further separates "caller-identity matters" from "caller-signature matters".
//!
//! Usage:  inject-probe <pid> <signed-dll-dir>
//!   <pid>             pid of `League of Legends.exe` (x64, already in a match
//!                     with its window visible)
//!   <signed-dll-dir>  directory containing the SIGNED yourgg_overlay-x64.dll

use std::{env, path::PathBuf, time::Duration, time::Instant};

use anyhow::Context;
use asdf_overlay_client::{OverlayDll, inject};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut args = env::args().skip(1);
    let pid: u32 = args
        .next()
        .context("usage: inject-probe <pid> <signed-dll-dir>")?
        .parse()
        .context("invalid pid")?;
    let dll_dir = PathBuf::from(
        args.next()
            .context("usage: inject-probe <pid> <signed-dll-dir>")?,
    );

    eprintln!(
        "[probe] caller = standalone exe (this pid {})",
        std::process::id()
    );
    eprintln!("[probe] target League pid = {pid}");
    eprintln!("[probe] signed dll dir = {}", dll_dir.display());
    eprintln!("[probe] calling inject() — watch A3 below for the SetWindowsHookExW cost\n");

    let start = Instant::now();
    let res = inject(
        pid,
        OverlayDll {
            x64: Some(&dll_dir.join("yourgg_overlay-x64.dll")),
            x86: Some(&dll_dir.join("yourgg_overlay-x86.dll")),
            arm64: Some(&dll_dir.join("yourgg_overlay-aarch64.dll")),
        },
        Some(Duration::from_secs(60)),
    )
    .await;

    match res {
        Ok((_conn, _event)) => eprintln!(
            "\n[probe] RESULT: inject() OK — total wall time {:?}. \
             Compare A3 to the Electron run's ~15s to decide caller-identity vs intrinsic.",
            start.elapsed()
        ),
        Err(err) => eprintln!(
            "\n[probe] RESULT: inject() FAILED after {:?}: {err:?}",
            start.elapsed()
        ),
    }

    Ok(())
}
