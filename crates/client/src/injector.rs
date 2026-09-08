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
use std::{os::windows::ffi::OsStrExt, path::Path, thread::sleep, time::Instant};

use anyhow::{Context, bail};
use scopeguard::defer;
use windows::{
    Win32::{
        Foundation::{
            CloseHandle, FILETIME, HANDLE, HINSTANCE, HWND, LPARAM, STILL_ACTIVE, WPARAM,
        },
        System::{
            LibraryLoader::{GetProcAddress, LoadLibraryW},
            SystemInformation::{
                IMAGE_FILE_MACHINE, IMAGE_FILE_MACHINE_AMD64, IMAGE_FILE_MACHINE_ARM64,
                IMAGE_FILE_MACHINE_I386, IMAGE_FILE_MACHINE_UNKNOWN,
            },
            Threading::{
                GetCurrentProcess, GetExitCodeProcess, GetProcessIdOfThread, GetProcessTimes,
                IsWow64Process2, OpenProcess, OpenThread, PROCESS_QUERY_LIMITED_INFORMATION,
                THREAD_QUERY_LIMITED_INFORMATION,
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

/// How often to re-check for the target's GUI thread while waiting for its
/// window to appear.
const GUI_THREAD_POLL: Duration = Duration::from_millis(100);

/// Inject the overlay DLL into the target process via `SetWindowsHookEx`.
///
/// On success the DLL has been registered as a `WH_GETMESSAGE` hook on a GUI
/// thread of the target and nudged to load; it then starts its IPC server and
/// pins itself. The hook is intentionally left installed for the session.
///
/// The target's main window may not exist yet when the caller learns of the
/// process (a game takes seconds from process start to first window). This
/// waits for a visible top-level window for up to `timeout` (indefinitely if
/// `None`), bailing early if the process exits meanwhile.
pub fn inject(pid: u32, dll: OverlayDll, timeout: Option<Duration>) -> anyhow::Result<()> {
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
    let wide: Vec<u16> = dll_path.as_os_str().encode_wide().chain([0]).collect();
    let hmod = unsafe { LoadLibraryW(PCWSTR(wide.as_ptr())) }
        .context("failed to load overlay dll in injector process")?;

    let proc = unsafe { GetProcAddress(hmod, s!("msg_hook_proc")) }
        .context("overlay dll is missing the `msg_hook_proc` export")?;
    let hook_proc: HOOKPROC = Some(unsafe { mem::transmute(proc) });

    let thread = wait_for_gui_thread(pid, timeout)?;

    // Capture a strong identity (process creation time) for the target at
    // discovery time. PIDs are recycled by the OS, so the bare `pid`/`thread`
    // discovered above could, by the time we hook, belong to a *different*
    // process that reused the same numeric id. The creation time pins the exact
    // process instance and lets us detect such a reuse before we act on it.
    let creation_time = process_creation_time(pid)
        .context("cannot read target process creation time for identity check")?;

    // Re-validate immediately before hooking: the thread must still belong to a
    // live process with the same pid *and* the same creation time. This closes
    // the time-of-check/time-of-use gap between discovery and `SetWindowsHookExW`
    // (target exited / pid recycled into an unrelated process).
    validate_thread_identity(thread, pid, creation_time)
        .context("target process changed between discovery and injection; aborting")?;

    // Register the hook; the OS maps the DLL into the target process.
    unsafe {
        SetWindowsHookExW(WH_GETMESSAGE, hook_proc, Some(HINSTANCE(hmod.0)), thread)
            .context("SetWindowsHookExW failed")?;
    }
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

/// Read the creation time of `pid` as a strong, recycle-proof identity.
///
/// PIDs are reused; the (pid, creation-time) pair uniquely identifies a process
/// instance. Opens the process with `PROCESS_QUERY_LIMITED_INFORMATION` only —
/// an access level permitted even on Vanguard-protected processes.
fn process_creation_time(pid: u32) -> anyhow::Result<u64> {
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }
        .context("cannot open target process for creation-time query")?;
    defer!(unsafe {
        _ = CloseHandle(handle);
    });

    let mut creation = FILETIME::default();
    let (mut exit, mut kernel, mut user) = (
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
    );
    unsafe {
        GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user)
            .context("GetProcessTimes failed for target process")?;
    }

    Ok(filetime_to_u64(creation))
}

/// Verify the discovered `thread` still belongs to a live process that is the
/// *same* instance as the one discovered (matching `pid` and `expected_creation`
/// creation time). Bails if the thread is gone, now owned by a different pid, or
/// the pid has been recycled into an unrelated process. This guards against the
/// time-of-check/time-of-use gap before `SetWindowsHookExW`.
fn validate_thread_identity(thread: u32, pid: u32, expected_creation: u64) -> anyhow::Result<()> {
    // Open the thread by id and resolve its *current* owning process. If the
    // thread has exited, this open fails and we bail — exactly what we want.
    let thandle = unsafe { OpenThread(THREAD_QUERY_LIMITED_INFORMATION, false, thread) }
        .context("target GUI thread no longer exists")?;
    defer!(unsafe {
        _ = CloseHandle(thandle);
    });

    let owner = unsafe { GetProcessIdOfThread(thandle) };
    if owner == 0 {
        bail!("cannot resolve owning process of target GUI thread");
    }
    if owner != pid {
        bail!(
            "target GUI thread now belongs to pid {owner}, expected {pid} (pid recycled or thread reassigned)",
        );
    }

    // Same pid is not enough — the pid itself could have been recycled. Confirm
    // the creation time still matches the instance we discovered.
    let creation = process_creation_time(pid)
        .context("cannot re-read target creation time for identity check")?;
    if creation != expected_creation {
        bail!(
            "target pid {pid} was recycled (creation time changed) between discovery and injection",
        );
    }

    Ok(())
}

/// Flatten a [`FILETIME`] into a single 64-bit tick count for comparison.
fn filetime_to_u64(ft: FILETIME) -> u64 {
    ((ft.dwHighDateTime as u64) << 32) | (ft.dwLowDateTime as u64)
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

/// Wait until `pid` owns a visible top-level window and return its thread.
///
/// Polls [`find_gui_thread`] every [`GUI_THREAD_POLL`] until it succeeds, the
/// process exits, or `timeout` elapses (`None` waits indefinitely). Lets the
/// caller attach as soon as it learns the process exists, without racing the
/// game's own window creation.
fn wait_for_gui_thread(pid: u32, timeout: Option<Duration>) -> anyhow::Result<u32> {
    let deadline = timeout.map(|dur| (dur, Instant::now() + dur));
    loop {
        if let Some(thread) = find_gui_thread(pid) {
            return Ok(thread);
        }
        if !process_is_alive(pid)? {
            bail!("target process {pid} exited before creating a window");
        }
        if let Some((dur, deadline)) = deadline
            && Instant::now() >= deadline
        {
            bail!(
                "timed out after {dur:?} waiting for a GUI thread (visible top-level window) in target process {pid}"
            );
        }
        sleep(GUI_THREAD_POLL);
    }
}

/// Whether `pid` is still running. Opens the process with
/// `PROCESS_QUERY_LIMITED_INFORMATION` only — an access level permitted even on
/// Vanguard-protected processes. A pid that can no longer be opened is treated
/// as gone.
fn process_is_alive(pid: u32) -> anyhow::Result<bool> {
    let Ok(handle) = (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }) else {
        return Ok(false);
    };
    defer!(unsafe {
        _ = CloseHandle(handle);
    });

    let mut exit_code = 0u32;
    unsafe {
        GetExitCodeProcess(handle, &mut exit_code)
            .context("GetExitCodeProcess failed for target process")?;
    }
    Ok(exit_code == STILL_ACTIVE.0 as u32)
}

/// Find a thread in `pid` that owns a visible top-level window, so it pumps
/// messages (which `WH_GETMESSAGE` requires to deliver the hook).
fn find_gui_thread(pid: u32) -> Option<u32> {
    let mut ctx = FindThread { pid, thread: 0 };
    // `EnumWindows` returns `Err` when our callback stops it early; expected.
    _ = unsafe { EnumWindows(Some(enum_windows_proc), LPARAM(&mut ctx as *mut _ as isize)) };
    (ctx.thread != 0).then_some(ctx.thread)
}
