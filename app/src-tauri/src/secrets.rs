//! Tokens live in the Windows credential manager, never in files, logs or the database.
use std::ffi::c_void;

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Security::Credentials::{
    CredDeleteW, CredFree, CredReadW, CredWriteW, CREDENTIALW, CRED_PERSIST_LOCAL_MACHINE,
    CRED_TYPE_GENERIC,
};

pub const TARGET_PREFIX: &str = "dev.justaclick";
/// HRESULT 0x80070490: the credential manager has no such entry.
const NOT_FOUND: i32 = -2_147_023_728;

pub fn target(name: &str) -> String {
    format!("{TARGET_PREFIX}/{name}")
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn save(target: &str, secret: &str) -> Result<(), String> {
    let name = wide(target);
    let user = wide("just-a-click");
    let mut blob = secret.as_bytes().to_vec();
    let credential = CREDENTIALW {
        Type: CRED_TYPE_GENERIC,
        TargetName: PWSTR(name.as_ptr() as *mut u16),
        CredentialBlobSize: blob.len() as u32,
        CredentialBlob: blob.as_mut_ptr(),
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        UserName: PWSTR(user.as_ptr() as *mut u16),
        ..Default::default()
    };
    unsafe { CredWriteW(&credential, 0) }.map_err(|error| format!("could not save: {error}"))
}

pub fn load(target: &str) -> Result<Option<String>, String> {
    let name = wide(target);
    let mut found: *mut CREDENTIALW = std::ptr::null_mut();
    match unsafe { CredReadW(PCWSTR(name.as_ptr()), CRED_TYPE_GENERIC, None, &mut found) } {
        Ok(()) => {}
        Err(error) if error.code().0 == NOT_FOUND => return Ok(None),
        Err(error) => return Err(format!("could not read: {error}")),
    }
    let secret = unsafe {
        let blob = std::slice::from_raw_parts((*found).CredentialBlob, (*found).CredentialBlobSize as usize);
        String::from_utf8(blob.to_vec())
    };
    unsafe { CredFree(found as *const c_void) };
    secret
        .map(Some)
        .map_err(|error| format!("the stored value is not text: {error}"))
}

/// Deleting an entry that is already gone is not an error: disconnecting twice is fine.
pub fn delete(target: &str) -> Result<(), String> {
    let name = wide(target);
    match unsafe { CredDeleteW(PCWSTR(name.as_ptr()), CRED_TYPE_GENERIC, None) } {
        Ok(()) => Ok(()),
        Err(error) if error.code().0 == NOT_FOUND => Ok(()),
        Err(error) => Err(format!("could not delete: {error}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Only ever the test entry, never the real one.
    fn test_target() -> String {
        format!("{TARGET_PREFIX}.test/notion")
    }

    #[test]
    fn the_target_carries_the_product_prefix() {
        assert_eq!(target("notion"), "dev.justaclick/notion");
    }

    #[test]
    fn a_secret_survives_a_round_trip_and_disappears_after_delete() {
        let name = test_target();
        let _ = delete(&name);
        assert_eq!(load(&name).expect("load"), None);
        save(&name, "secret-token-value").expect("save");
        assert_eq!(load(&name).expect("load"), Some("secret-token-value".to_string()));
        delete(&name).expect("delete");
        assert_eq!(load(&name).expect("load"), None);
    }

    #[test]
    fn deleting_twice_is_not_an_error() {
        let name = format!("{TARGET_PREFIX}.test/missing");
        delete(&name).expect("first delete");
        delete(&name).expect("second delete");
    }

    /// Run on its own, then `a_saved_secret_is_there_in_a_new_process` in a second process.
    #[test]
    #[ignore = "persistence check across processes; see Task 11"]
    fn persistence_writes_the_secret() {
        save(&format!("{TARGET_PREFIX}.test/persist"), "persisted-value").expect("save");
    }

    #[test]
    #[ignore = "persistence check across processes; see Task 11"]
    fn a_saved_secret_is_there_in_a_new_process() {
        let name = format!("{TARGET_PREFIX}.test/persist");
        let found = load(&name).expect("load");
        delete(&name).expect("delete");
        assert_eq!(found, Some("persisted-value".to_string()));
    }

    #[test]
    fn a_korean_secret_survives_the_round_trip() {
        let name = format!("{TARGET_PREFIX}.test/korean");
        save(&name, "비밀-토큰").expect("save");
        assert_eq!(load(&name).expect("load"), Some("비밀-토큰".to_string()));
        delete(&name).expect("delete");
    }
}
