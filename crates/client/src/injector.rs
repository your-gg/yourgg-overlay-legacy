//! Injector module for injecting the overlay DLL into a target process.
//!
//! Uses `SetWindowsHookEx(WH_GETMESSAGE)` so the OS maps the DLL into the
//! target on our behalf. This needs no `PROCESS_VM_WRITE` / `PROCESS_CREATE_THREAD`
//! handle to the target, so — unlike classic remote-thread injection — it is not
//! blocked by kernel anti-cheats (e.g. Vanguard), which deny those access rights
//! on the protected process (that denial is what makes the remote-thread path
//! fail with `STATUS_ACCESS_DENIED` / `0xC0000022`).
//!
//! The overlay DLL is loaded into *this* (injector) process only to obtain the
//! hook procedure; it stays inert here because it initializes the overlay from
//! its hook proc, which only fires inside the target. See `asdf-overlay-dll`.

use core::{mem, time::Duration};
use std::{os::windows::ffi::OsStrExt, path::Path};

use anyhow::{Context, bail};
use scopeguard::defer;
use windows::{
    Win32::{
        Foundation::{CloseHandle, HANDLE, HINSTANCE, HWND, LPARAM, WPARAM},
        System::{
            LibraryLoader::{GetProcAddress, LoadLibraryW},
            SystemInformation::{
                IMAGE_FILE_MACHINE, IMAGE_FILE_MACHINE_AMD64, IMAGE_FILE_MACHINE_ARM64,
                IMAGE_FILE_MACHINE_I386, IMAGE_FILE_MACHINE_UNKNOWN,
            },
            Threading::{
                GetCurrentProcess, IsWow64Process2, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
            },
        },
        UI::WindowsAndMessaging::{
            EnumWindows, GetWindowThreadProcessId, HOOKPROC, IsWindowVisible, PostThreadMessageW,
            SetWindowsHookExW, WH_GETMESSAGE, WM_NULL,
        },
    },
    core::{BOOL, PCWSTR, s},
};

use crate::OverlayDll;

/// Inject the overlay DLL into the target process via `SetWindowsHookEx`.
///
/// On success the DLL has been registered as a `WH_GETMESSAGE` hook on a GUI
/// thread of the target and nudged to load; it then starts its IPC server and
/// pins itself. The hook is intentionally left installed for the session.
pub fn inject(pid: u32, dll: OverlayDll, _timeout: Option<Duration>) -> anyhow::Result<()> {
    let current = current_arch();

    // `SetWindowsHookEx` requires the hook DLL to match the *target* process
    // arch, and we must load that DLL into our own process to register it. So
    // only same-arch injection is supported here; cross-arch needs a
    // matching-arch helper executable (not yet implemented).
    let target = target_arch(pid)?;
    if target != current {
        bail!(
            "cross-arch injection (injector {}, target {}) requires a same-arch helper exe (not implemented)",
            current.0,
            target.0,
        );
    }

    let dll_path: &Path = match current {
        IMAGE_FILE_MACHINE_AMD64 => dll.x64.context("x64 dll path is not provided")?,
        IMAGE_FILE_MACHINE_ARM64 => dll.arm64.context("arm64 dll path is not provided")?,
        IMAGE_FILE_MACHINE_I386 => dll.x86.context("x86 dll path is not provided")?,
        arch => bail!("Unsupported injector arch: {}", arch.0),
    };

    // Load the overlay DLL into the injector process to obtain the hook proc.
    // It does NOT start an overlay here — it only initializes when its hook proc
    // fires inside the target process (see `asdf-overlay-dll`).
    // DIAGNOSTIC: split phase A into its three real Win32 calls so we can tell
    // whether the latency is the DLL load (LoadLibraryW), the window scan
    // (find_gui_thread), or the hook-install syscall (SetWindowsHookExW) itself.
    // The outer "phase A" label conflates all three; do not read it as the hook
    // call alone. Prints to stderr; remove once diagnosed.
    let wide: Vec<u16> = dll_path.as_os_str().encode_wide().chain([0]).collect();
    let t_load = std::time::Instant::now();
    let hmod = unsafe { LoadLibraryW(PCWSTR(wide.as_ptr())) }
        .context("failed to load overlay dll in injector process")?;
    eprintln!(
        "[injector]   A1 LoadLibraryW (map overlay dll into THIS process): {:?}",
        t_load.elapsed()
    );

    let proc = unsafe { GetProcAddress(hmod, s!("msg_hook_proc")) }
        .context("overlay dll is missing the `msg_hook_proc` export")?;
    let hook_proc: HOOKPROC = Some(unsafe { mem::transmute(proc) });

    let t_find = std::time::Instant::now();
    let thread = find_gui_thread(pid)
        .context("cannot find a GUI thread (visible top-level window) in target process")?;
    eprintln!(
        "[injector]   A2 find_gui_thread (EnumWindows): {:?}",
        t_find.elapsed()
    );

    // Register the hook; the OS maps the DLL into the target process.
    let t_hook = std::time::Instant::now();
    unsafe {
        SetWindowsHookExW(WH_GETMESSAGE, hook_proc, Some(HINSTANCE(hmod.0)), thread)
            .context("SetWindowsHookExW failed")?;
    }
    eprintln!(
        "[injector]   A3 SetWindowsHookExW (the actual hook-install syscall): {:?}",
        t_hook.elapsed()
    );
    // Nudge the target thread's message queue so the hook fires now, mapping and
    // initializing the DLL promptly instead of on the next user input.
    unsafe {
        _ = PostThreadMessageW(thread, WM_NULL, WPARAM(0), LPARAM(0));
    }

    Ok(())
}

/// Architecture of the current (injector) process.
fn current_arch() -> IMAGE_FILE_MACHINE {
    process_arch(unsafe { GetCurrentProcess() })
}

/// Architecture of the target process. Opens it with
/// `PROCESS_QUERY_LIMITED_INFORMATION` only — an access level permitted even on
/// Vanguard-protected processes.
fn target_arch(pid: u32) -> anyhow::Result<IMAGE_FILE_MACHINE> {
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }
        .context("cannot open target process for arch query")?;
    defer!(unsafe {
        _ = CloseHandle(handle);
    });
    Ok(process_arch(handle))
}

/// Get the architecture of a process handle (resolving WOW64).
fn process_arch(handle: HANDLE) -> IMAGE_FILE_MACHINE {
    let mut native = IMAGE_FILE_MACHINE_UNKNOWN;
    let mut wow64 = IMAGE_FILE_MACHINE_UNKNOWN;
    unsafe {
        _ = IsWow64Process2(handle, &mut wow64, Some(&mut native));
    }

    if wow64 != IMAGE_FILE_MACHINE_UNKNOWN {
        wow64
    } else {
        native
    }
}

/// Context passed to the [`EnumWindows`] callback to collect a target thread.
struct FindThread {
    pid: u32,
    thread: u32,
}

unsafe extern "system" fn enum_windows_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let ctx = unsafe { &mut *(lparam.0 as *mut FindThread) };

    let mut window_pid = 0u32;
    let tid = unsafe { GetWindowThreadProcessId(hwnd, Some(&mut window_pid)) };
    if tid != 0 && window_pid == ctx.pid && unsafe { IsWindowVisible(hwnd) }.as_bool() {
        ctx.thread = tid;
        // Stop enumerating.
        return BOOL(0);
    }

    BOOL(1)
}

/// Find a thread in `pid` that owns a visible top-level window, so it pumps
/// messages (which `WH_GETMESSAGE` requires to deliver the hook).
fn find_gui_thread(pid: u32) -> Option<u32> {
    let mut ctx = FindThread { pid, thread: 0 };
    // `EnumWindows` returns `Err` when our callback stops it early; expected.
    _ = unsafe { EnumWindows(Some(enum_windows_proc), LPARAM(&mut ctx as *mut _ as isize)) };
    (ctx.thread != 0).then_some(ctx.thread)
}
