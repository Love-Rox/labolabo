//! Same-user access control for LaboLabo's Windows Named Pipes -- the
//! Windows counterpart of the unix transports' `chmod 0600` + parent-dir
//! `0700` (docs/hooks-protocol.md §4.2/§8, docs/control-protocol.md §9).
//!
//! Both Named Pipe servers (`hooks::NamedPipeEventTransport` and
//! `control::ControlServer`) create their pipe with the
//! [`same_user_security_descriptor`] DACL: full access for the current
//! user's SID and for `SYSTEM` (`SY` -- the OS itself, the closest analog
//! to unix `root` being unaffected by `0600`), nothing for anyone else,
//! with the protected flag (`P`) so no inherited ACEs can widen it. A
//! Windows named pipe with no explicit security descriptor would instead
//! get a default DACL that grants *read* access to `Everyone` and the
//! anonymous logon -- enough for another local user to connect to (and tie
//! up) a pipe instance even without being able to send events -- so both
//! servers fail closed: if this descriptor cannot be built, they refuse to
//! bind at all rather than fall back to the wider default.
//!
//! The current user's SID has to be resolved at runtime (there is no SDDL
//! token for "whoever is creating this object right now" that works for a
//! DACL grant -- `OW`/owner-rights changes semantics rather than naming the
//! creator), hence the small `windows-sys` dance below: process token ->
//! `TokenUser` -> `ConvertSidToStringSidW`.

use std::io;

use interprocess::os::windows::security_descriptor::SecurityDescriptor;
use widestring::U16CString;
use windows_sys::Win32::Foundation::{CloseHandle, LocalFree, HANDLE, HLOCAL};
use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;
use windows_sys::Win32::Security::{GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

/// Builds the security descriptor described in the module doc comment:
/// `D:P(A;;GA;;;SY)(A;;GA;;;<current user SID>)`.
pub(crate) fn same_user_security_descriptor() -> io::Result<SecurityDescriptor> {
    let sid = current_user_sid_string()?;
    let sddl = format!("D:P(A;;GA;;;SY)(A;;GA;;;{sid})");
    let sddl_w = U16CString::from_str(&sddl)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidInput, err))?;
    SecurityDescriptor::deserialize(&sddl_w)
}

/// The current process's user SID in string form (`S-1-5-21-...`).
fn current_user_sid_string() -> io::Result<String> {
    // SAFETY: standard process-token query sequence. Every handle/buffer
    // passed to the Win32 calls below is either the pseudo handle returned
    // by GetCurrentProcess() (never needs closing) or owned by this
    // function; `token` is closed exactly once on every path, and the
    // ConvertSidToStringSidW allocation is freed with LocalFree as its
    // documentation requires.
    unsafe {
        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return Err(io::Error::last_os_error());
        }
        let result = user_sid_string_from_token(token);
        CloseHandle(token);
        result
    }
}

/// # Safety
/// `token` must be a valid access-token handle opened with `TOKEN_QUERY`.
unsafe fn user_sid_string_from_token(token: HANDLE) -> io::Result<String> {
    // First call sizes the TOKEN_USER buffer (it always fails with
    // ERROR_INSUFFICIENT_BUFFER; only `len` matters).
    let mut len: u32 = 0;
    GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut len);
    if len == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut buf = vec![0u8; len as usize];
    if GetTokenInformation(token, TokenUser, buf.as_mut_ptr().cast(), len, &mut len) == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: a successful GetTokenInformation(TokenUser) call fills the
    // buffer with a TOKEN_USER whose embedded SID pointer points into this
    // same buffer, which stays alive for the whole conversion below.
    let token_user = &*buf.as_ptr().cast::<TOKEN_USER>();

    let mut sid_w: *mut u16 = std::ptr::null_mut();
    if ConvertSidToStringSidW(token_user.User.Sid, &mut sid_w) == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut n = 0usize;
    while *sid_w.add(n) != 0 {
        n += 1;
    }
    let sid = String::from_utf16_lossy(std::slice::from_raw_parts(sid_w, n));
    LocalFree(sid_w as HLOCAL);
    Ok(sid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_user_sid_looks_like_a_sid() {
        let sid = current_user_sid_string().expect("current user SID should resolve");
        assert!(
            sid.starts_with("S-1-"),
            "SID string should start with S-1-, got {sid:?}"
        );
    }

    #[test]
    fn same_user_security_descriptor_builds() {
        // The SDDL string must parse -- a typo here would otherwise only
        // surface as both pipe servers silently refusing to bind.
        same_user_security_descriptor().expect("security descriptor should deserialize");
    }
}
