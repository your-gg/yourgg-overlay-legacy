#![windows_subsystem = "windows"]

//! Official DLL crate for attaching [`asdf_overlay`] to other processes.
//! Using this DLL, the overlay can be controlled via cross-process IPC.
//!
//! Injection can be done using `asdf-overlay-client` crate.

#[cfg(debug_assertions)]
mod dbg;

mod augment_reader;
mod server;

extern crate asdf_overlay_vulkan_layer;

use anyhow::Context;
use asdf_overlay::{
    backend::{Backends, window::ListenInputFlags},
    event_sink::OverlayEventSink,
    initialize,
};
use asdf_overlay_common::{
    ipc::create_ipc_addr,
    request::{Request, WindowRequest},
};
use asdf_overlay_event::{OverlayEvent, WindowEvent};
use core::time::Duration;
use scopeguard::defer;
use std::{ffi::OsStr, sync::Once, thread};
use tokio::{
    net::windows::named_pipe::{NamedPipeServer, ServerOptions},
    runtime::Runtime,
    time::sleep,
};
use tracing::{debug, error, trace, warn};
use windows::{
    Win32::{
        Foundation::{
            CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE, HINSTANCE, HLOCAL, HMODULE, LPARAM,
            LRESULT, LocalFree, WPARAM,
        },
        Security::{
            ACL,
            Authorization::{
                EXPLICIT_ACCESS_A, SET_ACCESS, SetEntriesInAclA, TRUSTEE_A, TRUSTEE_IS_SID,
                TRUSTEE_IS_USER,
            },
            GetTokenInformation, InitializeSecurityDescriptor, NO_INHERITANCE,
            PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, SECURITY_DESCRIPTOR,
            SetSecurityDescriptorDacl, TOKEN_QUERY, TOKEN_USER, TokenUser,
        },
        System::{
            LibraryLoader::{
                GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_PIN,
                GetModuleHandleExW,
            },
            SystemServices::SECURITY_DESCRIPTOR_REVISION,
            Threading::{GetCurrentProcess, GetCurrentProcessId, OpenProcessToken},
        },
        UI::WindowsAndMessaging::CallNextHookEx,
    },
    core::{BOOL, PCWSTR, PSTR},
};

use crate::augment_reader::AugmentReader;
use crate::server::IpcServerConn;

/// IPC server main loop.
#[tracing::instrument(skip(server))]
async fn run(server: NamedPipeServer) -> anyhow::Result<()> {
    fn handle_window_event(hwnd: u32, req: WindowRequest) -> anyhow::Result<bool> {
        let res = Backends::with_backend(hwnd, |backend| {
            match req {
                WindowRequest::SetPosition(position) => {
                    backend.update_layout(|layout| {
                        layout.position = (position.x, position.y);
                    });
                }

                WindowRequest::SetAnchor(anchor) => {
                    backend.update_layout(|layout| {
                        layout.anchor = (anchor.x, anchor.y);
                    });
                }

                WindowRequest::SetMargin(margin) => {
                    backend.update_layout(|layout| {
                        layout.margin = (margin.top, margin.right, margin.bottom, margin.left);
                    });
                }

                WindowRequest::ListenInput(cmd) => {
                    let mut flags = ListenInputFlags::empty();
                    flags.set(ListenInputFlags::CURSOR, cmd.cursor);
                    flags.set(ListenInputFlags::KEYBOARD, cmd.keyboard);

                    backend.listen_input(flags);
                }

                WindowRequest::BlockInput(cmd) => {
                    backend.block_input(cmd.block);
                }

                WindowRequest::SetBlockingCursor(cmd) => {
                    backend.set_blocking_cursor(cmd.cursor);
                }

                WindowRequest::UpdateSharedHandle(shared) => {
                    if let Err(err) = backend.update_surface(shared.handle) {
                        error!("failed to open shared surface. err: {:?}", err);
                        return false;
                    }
                }
            }

            true
        });

        Ok(res.unwrap_or(false))
    }

    let mut conn = IpcServerConn::new(server).await?;
    let emitter = conn.create_emitter();
    let _augment_reader = match AugmentReader::start(emitter.clone()) {
        Ok(reader) => Some(reader),
        Err(err) => {
            warn!("cannot start augment memory reader: {err:?}");
            None
        }
    };
    {
        debug!("sending initial data");
        // send existing windows
        for backend in Backends::iter() {
            let render = backend.render.lock();
            let gpu_id = render.interop.gpu_id();
            let size = render.window_size;
            _ = emitter.emit(OverlayEvent::Window {
                id: *backend.key() as _,
                event: WindowEvent::Added {
                    width: size.0,
                    height: size.1,
                    gpu_id,
                },
            });
        }
    }

    OverlayEventSink::set(move |event| _ = emitter.emit(event));
    defer!({
        debug!("cleanup start");
        OverlayEventSink::clear();
        Backends::cleanup_backends();
    });

    while let Ok((req_id, req)) = conn.recv().await {
        trace!("recv id: {req_id} req: {req:?}");

        match req {
            Request::Window { id, request } => {
                conn.reply(req_id, handle_window_event(id, request)?)?;
            }
        }
    }
    Ok(())
}

/// IPC server listener.
#[tracing::instrument(skip(create_server))]
async fn run_server(
    mut server: NamedPipeServer,
    mut create_server: impl FnMut() -> anyhow::Result<NamedPipeServer>,
) {
    loop {
        debug!("waiting ipc client...");
        match server.connect().await {
            Ok(_) => {
                if let Err(err) = run(server).await {
                    warn!("client connection ended unexpectedly. err: {:?}", err);
                }
            }
            Err(err) => {
                error!("failed to connect to client. err: {err:?}");
            }
        }

        server = loop {
            match create_server() {
                Ok(server) => break server,
                Err(err) => {
                    error!("failed to create server. retrying after 5 seconds. err: {err:?}");
                    sleep(Duration::from_secs(5)).await;
                }
            }
        };
    }
}

/// Ensures the overlay is started at most once per process.
static INIT: Once = Once::new();

/// Initialize hooks and the IPC server. Runs exactly once, in the *target*
/// process, driven by [`msg_hook_proc`] when the injected hook first fires.
///
/// This is intentionally NOT done in `DllMain`: the injector loads this DLL
/// into its own process to register the hook, and we must not spin up an
/// overlay there. The hook proc only ever runs in the target, so gating
/// initialization on it keeps the injector process inert.
fn start_overlay(module_handle: usize) {
    #[cfg(debug_assertions)]
    {
        use tracing::level_filters::LevelFilter;

        use crate::dbg::WinDbgMakeWriter;

        tracing_subscriber::fmt::fmt()
            .with_ansi(false)
            .with_thread_ids(true)
            .with_max_level(LevelFilter::TRACE)
            .with_writer(WinDbgMakeWriter::new())
            .init();
    }

    // setup tokio runtime
    let Ok(rt) = Runtime::new() else {
        error!("cannot create tokio runtime");
        return;
    };
    let _guard = rt.enter();

    let pid = unsafe { GetCurrentProcessId() };
    // setup first ipc server
    let server = match create_ipc_server(create_ipc_addr(pid), true) {
        Ok(server) => server,
        Err(err) => {
            error!("cannot open ipc server. err: {err:?}");
            return;
        }
    };
    let create_server = move || create_ipc_server(create_ipc_addr(pid), false);

    thread::spawn(move || {
        // initialize overlay
        if let Err(err) = initialize(module_handle as _) {
            // A hook-install failure must NOT abort the host game. Bail out of
            // the overlay thread so the game continues without an overlay.
            error!("overlay init failed: {err:?}");
            return;
        }
        debug!("hook installed");

        rt.block_on(run_server(server, create_server))
    });
}

/// Exported `WH_GETMESSAGE` hook procedure used as the injection entry point.
///
/// `asdf-overlay-client` injects this DLL by registering this procedure with
/// `SetWindowsHookExW`; the OS then maps the DLL into the target process and
/// calls this on the hooked thread. On first call we pin our module (so we
/// survive the hook being removed), start the overlay once, then forward the
/// call down the hook chain.
///
/// # Safety
/// Called by the OS hook chain only.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn msg_hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    INIT.call_once(|| {
        // Pin our module and obtain our own module handle for overlay init.
        // `GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS` treats the pointer as an
        // address inside this module rather than a name.
        let mut hmod = HMODULE::default();
        unsafe {
            _ = GetModuleHandleExW(
                GET_MODULE_HANDLE_EX_FLAG_PIN | GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
                PCWSTR(msg_hook_proc as usize as *const u16),
                &mut hmod,
            );
        }
        start_overlay(hmod.0 as usize);
    });

    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

/// DllMain — intentionally minimal. Overlay initialization happens in
/// [`msg_hook_proc`], so loading this DLL inside the injector process (only to
/// obtain the hook procedure) does not start an overlay there.
///
/// # Safety
/// Can be called by loader only.
#[unsafe(no_mangle)]
#[allow(non_snake_case, unused_variables)]
pub unsafe extern "system" fn DllMain(dll_module: HINSTANCE, fdw_reason: u32, _: *mut ()) -> bool {
    true
}

/// Create a new IPC server using the given address.
fn create_ipc_server(addr: impl AsRef<OsStr>, first: bool) -> anyhow::Result<NamedPipeServer> {
    // The security descriptor stores a pointer to a heap-allocated DACL
    // (`pacl`). That ACL must outlive the pipe-creation call, so it is built
    // here, kept alive across `create_with_security_attributes_raw`, and freed
    // afterwards via `LocalFree` (previously it was leaked).
    let (mut security_desc, pacl) =
        create_user_security_desc().context("failed to create user security desc")?;
    // SAFETY: `pacl` is the ACL allocated by `SetEntriesInAclA`; freeing it once
    // the pipe handle has captured the descriptor is correct. A null pacl is a
    // no-op for `LocalFree`.
    defer!(unsafe {
        let _ = LocalFree(Some(HLOCAL(pacl as *mut _)));
    });

    Ok(unsafe {
        ServerOptions::new()
            .first_pipe_instance(first)
            .create_with_security_attributes_raw(
                addr,
                &mut SECURITY_ATTRIBUTES {
                    nLength: 1,
                    lpSecurityDescriptor: &mut security_desc as *mut _ as _,
                    bInheritHandle: BOOL(0),
                } as *mut _ as _,
            )?
    })
}

/// Build a Windows security descriptor granting read/write access to the
/// CURRENT USER only.
///
/// Returns the descriptor together with the raw pointer to the DACL allocated by
/// `SetEntriesInAclA`; the caller is responsible for freeing that ACL with
/// `LocalFree` once the descriptor is no longer referenced.
///
/// The DACL is restricted to the process owner's SID (obtained from the process
/// token) instead of the World/Everyone SID, so other users on the machine
/// cannot connect to the overlay IPC pipe.
fn create_user_security_desc() -> anyhow::Result<(SECURITY_DESCRIPTOR, *mut ACL)> {
    // Query the current process token for the owner SID.
    let mut token = HANDLE::default();
    unsafe {
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token)?;
    }
    defer!(unsafe {
        let _ = CloseHandle(token);
    });

    // First call retrieves the required buffer length.
    let mut len = 0_u32;
    unsafe {
        // This is expected to fail with ERROR_INSUFFICIENT_BUFFER while setting
        // `len`; ignore that specific result and rely on the second call.
        let _ = GetTokenInformation(token, TokenUser, None, 0, &mut len);
    }
    if len == 0 {
        anyhow::bail!("failed to query token user information length");
    }

    let mut buf = vec![0_u8; len as usize];
    unsafe {
        GetTokenInformation(
            token,
            TokenUser,
            Some(buf.as_mut_ptr().cast()),
            len,
            &mut len,
        )?;
    }
    // SAFETY: `buf` holds a `TOKEN_USER` followed by its SID; the SID pointer it
    // contains stays valid for as long as `buf` is alive (kept until end of fn).
    let token_user = unsafe { &*(buf.as_ptr() as *const TOKEN_USER) };
    let user_sid = token_user.User.Sid;

    let access = EXPLICIT_ACCESS_A {
        grfAccessPermissions: GENERIC_READ.0 | GENERIC_WRITE.0,
        grfAccessMode: SET_ACCESS,
        grfInheritance: NO_INHERITANCE,
        Trustee: TRUSTEE_A {
            TrusteeForm: TRUSTEE_IS_SID,
            TrusteeType: TRUSTEE_IS_USER,
            ptstrName: PSTR(user_sid.0.cast()),
            ..Default::default()
        },
    };

    let mut pacl: *mut ACL = 0 as _;
    unsafe {
        SetEntriesInAclA(Some(&[access]), None, &mut pacl).ok()?;
    }

    let mut security_desc = SECURITY_DESCRIPTOR::default();
    unsafe {
        InitializeSecurityDescriptor(
            PSECURITY_DESCRIPTOR(&mut security_desc as *mut _ as _),
            SECURITY_DESCRIPTOR_REVISION,
        )?;

        SetSecurityDescriptorDacl(
            PSECURITY_DESCRIPTOR(&mut security_desc as *mut _ as _),
            true,
            Some(pacl),
            false,
        )?;
    }

    Ok((security_desc, pacl))
}
