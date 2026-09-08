//! Process enumeration for game discovery.
//!
//! Uses a Toolhelp32 snapshot so the host never has to spawn `tasklist.exe`
//! or PowerShell: no console flash, no process-discovery heuristics tripped in
//! EDR tooling, and a call costs about a millisecond instead of hundreds.

use core::mem;

use anyhow::Context as _;
use neon::prelude::*;
use scopeguard::defer;
use windows::Win32::{
    Foundation::CloseHandle,
    System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
        TH32CS_SNAPPROCESS,
    },
};

/// A running process as seen by the snapshot: pid and executable file name
/// (`League of Legends.exe`), without a path.
pub struct ProcessEntry {
    pub pid: u32,
    pub name: String,
}

/// Snapshot every running process. Only the executable name and pid are read,
/// so this needs no handle to any process and works on protected ones.
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
        let len = entry
            .szExeFile
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(entry.szExeFile.len());
        processes.push(ProcessEntry {
            pid: entry.th32ProcessID,
            name: String::from_utf16_lossy(&entry.szExeFile[..len]),
        });
        if unsafe { Process32NextW(snapshot, &mut entry) }.is_err() {
            break;
        }
    }

    Ok(processes)
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
        array.set(&mut cx, index as u32, object)?;
    }
    Ok(array)
}

pub fn export_module_functions(cx: &mut ModuleContext) -> NeonResult<()> {
    cx.export_function("listProcesses", list_processes_js)?;
    Ok(())
}
