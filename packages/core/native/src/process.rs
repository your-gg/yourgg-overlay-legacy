//! Process enumeration for game discovery.
//!
//! Uses a Toolhelp32 snapshot so the host never has to spawn `tasklist.exe`
//! or PowerShell: no console flash, no process-discovery heuristics tripped in
//! EDR tooling, and a call costs a few milliseconds instead of hundreds.

use core::mem;

use anyhow::Context as _;
use neon::prelude::*;
use scopeguard::defer;
use windows::{
    Win32::{
        Foundation::CloseHandle,
        System::{
            Diagnostics::ToolHelp::{
                CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
                TH32CS_SNAPPROCESS,
            },
            Threading::{
                OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
                QueryFullProcessImageNameW,
            },
        },
    },
    core::PWSTR,
};

/// A running process as seen by the snapshot: pid, executable file name
/// (`League of Legends.exe`) and, when readable, the executable's full path.
pub struct ProcessEntry {
    pub pid: u32,
    pub name: String,
    pub path: Option<String>,
}

/// Snapshot every running process.
///
/// Name and pid come from the snapshot and need no handle. The full path is
/// read with `PROCESS_QUERY_LIMITED_INFORMATION`, an access level granted even
/// on anti-cheat-protected games; processes that still refuse it (protected
/// system processes) simply report no path.
pub fn list_processes() -> anyhow::Result<Vec<ProcessEntry>> {
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }
        .context("CreateToolhelp32Snapshot failed")?;
    defer!(unsafe {
        _ = CloseHandle(snapshot);
    });

    let mut entry = PROCESSENTRY32W {
        dwSize: mem::size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };

    let mut processes = Vec::new();
    // `Process32FirstW` fails with ERROR_NO_MORE_FILES on an empty snapshot,
    // which cannot happen for TH32CS_SNAPPROCESS (System is always listed) but
    // is still just "no entries" rather than an error.
    if unsafe { Process32FirstW(snapshot, &mut entry) }.is_err() {
        return Ok(processes);
    }
    loop {
        let pid = entry.th32ProcessID;
        processes.push(ProcessEntry {
            pid,
            name: utf16_until_nul(&entry.szExeFile),
            path: process_image_path(pid),
        });
        if unsafe { Process32NextW(snapshot, &mut entry) }.is_err() {
            break;
        }
    }

    Ok(processes)
}

/// Full path of the process image, or `None` if the process cannot be opened
/// or queried (idle/system processes, or ones that deny even limited query).
fn process_image_path(pid: u32) -> Option<String> {
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?;
    defer!(unsafe {
        _ = CloseHandle(handle);
    });

    // MAX_PATH is not a hard limit for image paths; grow on ERROR_INSUFFICIENT_BUFFER.
    let mut buf = vec![0u16; 1024];
    loop {
        let mut len = buf.len() as u32;
        let res = unsafe {
            QueryFullProcessImageNameW(
                handle,
                PROCESS_NAME_WIN32,
                PWSTR(buf.as_mut_ptr()),
                &mut len,
            )
        };
        match res {
            Ok(()) => return Some(String::from_utf16_lossy(&buf[..len as usize])),
            Err(_) if buf.len() < 32 * 1024 => buf.resize(buf.len() * 2, 0),
            Err(_) => return None,
        }
    }
}

fn utf16_until_nul(buf: &[u16]) -> String {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len])
}

fn list_processes_js(mut cx: FunctionContext) -> JsResult<JsArray> {
    let processes = list_processes()
        .or_else(|err| cx.throw_error(format!("Failed to list processes. {err:?}")))?;

    let array = JsArray::new(&mut cx, processes.len());
    for (index, process) in processes.into_iter().enumerate() {
        let object = cx.empty_object();
        let pid = cx.number(process.pid);
        object.set(&mut cx, "pid", pid)?;
        let name = cx.string(process.name);
        object.set(&mut cx, "name", name)?;
        if let Some(path) = process.path {
            let path = cx.string(path);
            object.set(&mut cx, "path", path)?;
        }
        array.set(&mut cx, index as u32, object)?;
    }
    Ok(array)
}

pub fn export_module_functions(cx: &mut ModuleContext) -> NeonResult<()> {
    cx.export_function("listProcesses", list_processes_js)?;
    Ok(())
}
