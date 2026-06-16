use core::cell::RefCell;
use core::time::Duration;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::conv::{deserialize_percent_length, emit_event};
use super::util::with_rt;
use crate::{conv::deserialize_handle_update, util::runtime};
use anyhow::{Context as AnyhowContext, bail};
use asdf_overlay_client::{
    client::{IpcClientConn, IpcClientEventStream},
    common::{
        cursor::Cursor,
        request::{BlockInput, ListenInput, SetAnchor, SetBlockingCursor, SetMargin, SetPosition},
    },
    connect,
};
use neon::prelude::*;
use num::FromPrimitive;
use tokio::sync::Mutex;

struct Overlay(RefCell<Option<Inner>>);

impl Overlay {
    pub fn new(ipc: IpcClientConn, event: IpcClientEventStream) -> Self {
        Self(RefCell::new(Some(Inner {
            ipc: Arc::new(Mutex::new(ipc)),
            event: Arc::new(Mutex::new(event)),
        })))
    }

    pub fn ipc(&self, cx: &mut Cx) -> NeonResult<Arc<Mutex<IpcClientConn>>> {
        match *self.0.borrow() {
            Some(ref inner) => Ok(inner.ipc.clone()),
            None => cx.throw_error("Overlay is destroyed"),
        }
    }

    pub fn events(&self, cx: &mut Cx) -> NeonResult<Arc<Mutex<IpcClientEventStream>>> {
        match *self.0.borrow() {
            Some(ref inner) => Ok(inner.event.clone()),
            None => cx.throw_error("Overlay is destroyed"),
        }
    }

    pub fn destroy(&self, cx: &mut Cx) -> NeonResult<()> {
        match self.0.borrow_mut().take() {
            Some(_) => Ok(()),
            None => cx.throw_error("Overlay is already destroyed"),
        }
    }
}

struct Inner {
    ipc: Arc<Mutex<IpcClientConn>>,
    event: Arc<Mutex<IpcClientEventStream>>,
}

impl Finalize for Overlay {
    fn finalize<'a, C: Context<'a>>(self, _: &mut C) {}
}

fn attach(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let dll_dir = cx.argument::<JsString>(0)?.value(&mut cx);
    let pid = cx.argument::<JsNumber>(1)?.value(&mut cx) as u32;
    let timeout = cx
        .argument_opt(2)
        .filter(|v| !v.is_a::<JsUndefined, _>(&mut cx))
        .map(|v| v.downcast_or_throw::<JsNumber, _>(&mut cx))
        .transpose()?
        .map(|timeout| Duration::from_millis(timeout.value(&mut cx) as _));

    let rt = runtime(&mut cx)?;
    let channel = cx.channel();

    let (deferred, promise) = cx.promise();
    rt.spawn(async move {
        let dll_dir = PathBuf::from(dll_dir);
        let res = attach_via_helper(pid, &dll_dir, timeout)
            .await
            .context("cannot inject to the process");

        deferred.settle_with(&channel, move |mut cx| match res {
            Ok((ipc, event)) => Ok(cx.boxed(Overlay::new(ipc, event))),
            Err(err) => cx.throw_error(format!("{err:?}")),
        });
    });

    Ok(promise)
}

/// Out-of-process injection: spawn a small same-arch helper exe to call
/// `SetWindowsHookExW`, then connect to the IPC server the injected DLL starts.
///
/// Installing the hook inside this (Electron/Chromium) process makes the
/// anti-cheat block `SetWindowsHookExW` for many seconds; doing it from the
/// dedicated signed helper exe is near-instant. The DLL pins itself on first
/// hook fire, so it survives the helper exiting (which removes the hook); we
/// then connect from here.
///
/// Target arch is assumed x64 (League of Legends); cross-arch helper selection
/// is a future step.
async fn attach_via_helper(
    pid: u32,
    dll_dir: &Path,
    timeout: Option<Duration>,
) -> anyhow::Result<(IpcClientConn, IpcClientEventStream)> {
    let helper = dll_dir.join("yourgg-injector-x64.exe");
    let dll = dll_dir.join("yourgg_overlay-x64.dll");

    if !helper.exists() {
        bail!(
            "injector helper not found at {} (target arch must be x64; ensure it is bundled alongside the overlay DLLs)",
            helper.display()
        );
    }

    // Run the (slow-under-anti-cheat) SetWindowsHookExW in the helper, out of
    // this process. Capture stderr so the helper's failure cause reaches us;
    // kill_on_drop so a cancelled/timed-out attach can't leak the child.
    let child = tokio::process::Command::new(&helper)
        .arg(pid.to_string())
        .arg(&dll)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .with_context(|| format!("failed to spawn injector helper at {}", helper.display()))?;

    // Bound the wait: the hook syscall itself is uninterruptible, so enforce the
    // caller's timeout at the process boundary — dropping the future on timeout
    // kills the child (kill_on_drop).
    let output_fut = child.wait_with_output();
    let output = match timeout {
        Some(dur) => match tokio::time::timeout(dur, output_fut).await {
            Ok(res) => res.context("failed to wait for injector helper")?,
            Err(_) => bail!("injector helper timed out after {dur:?}"),
        },
        None => output_fut.await.context("failed to wait for injector helper")?,
    };
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = stderr.lines().last().unwrap_or("(no stderr)").trim();
        bail!("injector helper failed ({}): {detail}", output.status);
    }

    connect(pid, timeout).await
}

fn overlay_set_position(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let ipc = cx.argument::<JsBox<Overlay>>(0)?.ipc(&mut cx)?;
    let win_id = cx.argument::<JsNumber>(1)?.value(&mut cx) as u32;
    let x = cx.argument::<JsObject>(2)?;
    let x = deserialize_percent_length(&mut cx, &x)?;
    let y = cx.argument::<JsObject>(3)?;
    let y = deserialize_percent_length(&mut cx, &y)?;

    with_rt(&mut cx, async move {
        ipc.lock()
            .await
            .window(win_id)
            .request(SetPosition { x, y })
            .await?;
        Ok(())
    })
}

fn overlay_update_handle(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let ipc = cx.argument::<JsBox<Overlay>>(0)?.ipc(&mut cx)?;
    let win_id = cx.argument::<JsNumber>(1)?.value(&mut cx) as u32;
    let update = {
        let obj = cx.argument::<JsObject>(2)?;
        deserialize_handle_update(&mut cx, &obj)?
    };

    with_rt(&mut cx, async move {
        ipc.lock().await.window(win_id).request(update).await?;
        Ok(())
    })
}

fn overlay_set_anchor(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let ipc = cx.argument::<JsBox<Overlay>>(0)?.ipc(&mut cx)?;
    let win_id = cx.argument::<JsNumber>(1)?.value(&mut cx) as u32;
    let x = cx.argument::<JsObject>(2)?;
    let x = deserialize_percent_length(&mut cx, &x)?;
    let y = cx.argument::<JsObject>(3)?;
    let y = deserialize_percent_length(&mut cx, &y)?;

    with_rt(&mut cx, async move {
        ipc.lock()
            .await
            .window(win_id)
            .request(SetAnchor { x, y })
            .await?;
        Ok(())
    })
}

fn overlay_set_margin(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let ipc = cx.argument::<JsBox<Overlay>>(0)?.ipc(&mut cx)?;
    let win_id = cx.argument::<JsNumber>(1)?.value(&mut cx) as u32;
    let top = cx.argument::<JsObject>(2)?;
    let top = deserialize_percent_length(&mut cx, &top)?;
    let right = cx.argument::<JsObject>(3)?;
    let right = deserialize_percent_length(&mut cx, &right)?;
    let bottom = cx.argument::<JsObject>(4)?;
    let bottom = deserialize_percent_length(&mut cx, &bottom)?;
    let left = cx.argument::<JsObject>(5)?;
    let left = deserialize_percent_length(&mut cx, &left)?;

    with_rt(&mut cx, async move {
        ipc.lock()
            .await
            .window(win_id)
            .request(SetMargin {
                top,
                right,
                bottom,
                left,
            })
            .await?;
        Ok(())
    })
}

fn overlay_destroy(mut cx: FunctionContext) -> JsResult<JsUndefined> {
    cx.argument::<JsBox<Overlay>>(0)?.destroy(&mut cx)?;
    Ok(cx.undefined())
}

fn overlay_call_next_event(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let events = cx.argument::<JsBox<Overlay>>(0)?.events(&mut cx)?;
    let emitter = cx.argument::<JsObject>(1)?.root(&mut cx);
    let emit = cx.argument::<JsFunction>(2)?.root(&mut cx);

    let rt = runtime(&mut cx)?;
    let channel = cx.channel();

    let (deferred, promise) = cx.promise();
    rt.spawn(async move {
        let event = events.lock().await.recv().await;
        deferred.settle_with(&channel, move |mut cx| match event {
            Some(event) => {
                let emitter = emitter.into_inner(&mut cx);
                let emit = emit.into_inner(&mut cx);
                emit_event(&mut cx, event, emitter, emit)?;

                Ok(cx.boolean(true))
            }
            None => Ok(cx.boolean(false)),
        });
    });

    Ok(promise)
}

fn overlay_listen_input(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let ipc = cx.argument::<JsBox<Overlay>>(0)?.ipc(&mut cx)?;
    let win_id = cx.argument::<JsNumber>(1)?.value(&mut cx) as u32;
    let cursor = cx.argument::<JsBoolean>(2)?.value(&mut cx);
    let keyboard = cx.argument::<JsBoolean>(3)?.value(&mut cx);

    with_rt(&mut cx, async move {
        ipc.lock()
            .await
            .window(win_id)
            .request(ListenInput { cursor, keyboard })
            .await?;

        Ok(())
    })
}

fn overlay_block_input(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let ipc = cx.argument::<JsBox<Overlay>>(0)?.ipc(&mut cx)?;
    let win_id = cx.argument::<JsNumber>(1)?.value(&mut cx) as u32;
    let block = cx.argument::<JsBoolean>(2)?.value(&mut cx);

    with_rt(&mut cx, async move {
        ipc.lock()
            .await
            .window(win_id)
            .request(BlockInput { block })
            .await?;

        Ok(())
    })
}

fn overlay_set_blocking_cursor(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let ipc = cx.argument::<JsBox<Overlay>>(0)?.ipc(&mut cx)?;
    let win_id = cx.argument::<JsNumber>(1)?.value(&mut cx) as u32;
    let cursor = cx
        .argument_opt(2)
        .filter(|v| !v.is_a::<JsUndefined, _>(&mut cx))
        .map(|v| Ok(v.downcast_or_throw::<JsNumber, _>(&mut cx)?.value(&mut cx) as u32))
        .transpose()?;

    let cursor = match cursor {
        Some(discriminant) => {
            let Some(cursor) = Cursor::from_u32(discriminant) else {
                return cx.throw_error("invalid cursor value");
            };
            Some(cursor)
        }

        None => None,
    };

    with_rt(&mut cx, async move {
        ipc.lock()
            .await
            .window(win_id)
            .request(SetBlockingCursor { cursor })
            .await?;

        Ok(())
    })
}

pub fn export_module_functions(cx: &mut ModuleContext) -> NeonResult<()> {
    cx.export_function("attach", attach)?;

    cx.export_function("overlaySetPosition", overlay_set_position)?;
    cx.export_function("overlaySetAnchor", overlay_set_anchor)?;
    cx.export_function("overlaySetMargin", overlay_set_margin)?;
    cx.export_function("overlayUpdateHandle", overlay_update_handle)?;
    cx.export_function("overlayListenInput", overlay_listen_input)?;
    cx.export_function("overlayBlockInput", overlay_block_input)?;
    cx.export_function("overlaySetBlockingCursor", overlay_set_blocking_cursor)?;

    cx.export_function("overlayCallNextEvent", overlay_call_next_event)?;

    cx.export_function("overlayDestroy", overlay_destroy)?;
    Ok(())
}
