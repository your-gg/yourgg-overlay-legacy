//! Authenticode verification for binaries we load into, or run against, a
//! protected game process.
//!
//! Riot's approval of this overlay presumes the signed binaries our CI
//! produces. Injecting a locally built (unsigned) DLL into a live game risks
//! account enforcement for whoever is logged in, so the injector refuses to
//! touch a binary that is not signed and trusted. This is a guard against
//! accidents during development, not against a determined attacker; it also
//! happens to stop a planted DLL in the install directory from being loaded.

use std::{os::windows::ffi::OsStrExt, path::Path};

use anyhow::bail;
use windows::{
    Win32::{
        Foundation::HWND,
        Security::WinTrust::{
            WINTRUST_ACTION_GENERIC_VERIFY_V2, WINTRUST_DATA, WINTRUST_DATA_0, WINTRUST_FILE_INFO,
            WTD_CACHE_ONLY_URL_RETRIEVAL, WTD_CHOICE_FILE, WTD_REVOKE_NONE, WTD_STATEACTION_CLOSE,
            WTD_STATEACTION_VERIFY, WTD_UI_NONE, WinVerifyTrust,
        },
    },
    core::PCWSTR,
};

/// Environment variable that waives the check. Honoured only in debug builds so
/// a release binary can never be talked out of verifying.
const ALLOW_UNSIGNED_ENV: &str = "YOURGG_ALLOW_UNSIGNED";

/// Verify that `path` carries a valid Authenticode signature chaining to a
/// trusted root, and bail otherwise.
///
/// Revocation is checked from cache only: a user who is offline, or behind a
/// network that blocks the CRL endpoints, must still be able to run the
/// overlay. Expiry of the signing certificate is fine as long as the signature
/// is timestamped, which our CI does.
///
/// The error text names the file and spells out the verdict, because it is the
/// only diagnostic that survives the trip out of the helper process and into
/// Sentry.
pub fn verify_signed(path: &Path) -> anyhow::Result<()> {
    if unsigned_allowed() {
        eprintln!(
            "[signature] check waived by {ALLOW_UNSIGNED_ENV} (debug build): {}",
            path.display(),
        );
        return Ok(());
    }

    let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();

    let mut file_info = WINTRUST_FILE_INFO {
        cbStruct: size_of::<WINTRUST_FILE_INFO>() as u32,
        pcwszFilePath: PCWSTR(wide.as_ptr()),
        ..Default::default()
    };

    let mut data = WINTRUST_DATA {
        cbStruct: size_of::<WINTRUST_DATA>() as u32,
        dwUIChoice: WTD_UI_NONE,
        fdwRevocationChecks: WTD_REVOKE_NONE,
        dwUnionChoice: WTD_CHOICE_FILE,
        dwStateAction: WTD_STATEACTION_VERIFY,
        dwProvFlags: WTD_CACHE_ONLY_URL_RETRIEVAL,
        Anonymous: WINTRUST_DATA_0 {
            pFile: &mut file_info,
        },
        ..Default::default()
    };

    let mut action = WINTRUST_ACTION_GENERIC_VERIFY_V2;
    let data_ptr = &raw mut data;
    let status = unsafe { WinVerifyTrust(HWND::default(), &mut action, data_ptr.cast()) };

    // The verify call allocates provider state that must be released with a
    // second call, whatever the verdict. Write the close action through the
    // same pointer the API holds.
    unsafe {
        (*data_ptr).dwStateAction = WTD_STATEACTION_CLOSE;
        _ = WinVerifyTrust(HWND::default(), &mut action, data_ptr.cast());
    }

    if status != 0 {
        bail!(
            "refusing to inject {}: {} (WinVerifyTrust {:#010x}). \
             Only CI-signed binaries may be used against a Riot game; \
             download the signed artifact from the deploy workflow.",
            path.display(),
            describe(status),
            status as u32,
        );
    }

    Ok(())
}

/// Translate the common `WinVerifyTrust` verdicts into something a reader of a
/// Sentry issue can act on without looking up an HRESULT.
fn describe(status: i32) -> &'static str {
    match status as u32 {
        0x800B0100 => "the file is not signed at all (a local build?)",
        0x800B0109 => "the signing certificate does not chain to a trusted root",
        0x800B010A => "the certificate chain could not be built",
        0x80096010 => "the signature does not match the file (tampered or truncated)",
        0x800B0111 => "the publisher is explicitly distrusted on this machine",
        0x80092026 => "local security settings block this signature",
        _ => "the Authenticode signature could not be verified",
    }
}

/// Whether the caller is allowed to skip verification. Debug builds honour an
/// opt-in environment variable so the overlay can be exercised against a local
/// test application; release builds never skip.
fn unsigned_allowed() -> bool {
    cfg!(debug_assertions) && std::env::var_os(ALLOW_UNSIGNED_ENV).is_some_and(|value| value != "0")
}
