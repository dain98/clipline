//! Safe generic-credential access with one audited Windows allocation owner.

use std::ffi::OsStr;
use std::ptr;

use windows_sys::Win32::Foundation::{GetLastError, ERROR_NOT_FOUND};
use windows_sys::Win32::Security::Credentials::{
    CredDeleteW, CredFree, CredReadW, CredWriteW, CREDENTIALW, CRED_PERSIST_LOCAL_MACHINE,
    CRED_TYPE_GENERIC,
};

use super::{last_os_error, wide_null_checked};

#[derive(Clone, Copy)]
pub(crate) struct CredentialStore {
    value_label: &'static str,
}

impl CredentialStore {
    pub(crate) const fn new(value_label: &'static str) -> Self {
        Self { value_label }
    }

    pub(crate) fn write(self, target: &str, username: &str, value: &str) -> Result<(), String> {
        let mut target_w = wide_null_checked(OsStr::new(target), "credential target")?;
        let mut username_w = wide_null_checked(OsStr::new(username), "credential username")?;
        let mut blob = value.as_bytes().to_vec();
        let blob_len =
            u32::try_from(blob.len()).map_err(|_| format!("{} is too large", self.value_label))?;
        let credential = CREDENTIALW {
            Flags: 0,
            Type: CRED_TYPE_GENERIC,
            TargetName: target_w.as_mut_ptr(),
            Comment: ptr::null_mut(),
            LastWritten: Default::default(),
            CredentialBlobSize: blob_len,
            CredentialBlob: blob.as_mut_ptr(),
            Persist: CRED_PERSIST_LOCAL_MACHINE,
            AttributeCount: 0,
            Attributes: ptr::null_mut(),
            TargetAlias: ptr::null_mut(),
            UserName: username_w.as_mut_ptr(),
        };
        if unsafe { CredWriteW(&credential, 0) } == 0 {
            return Err(last_os_error(&format!("store {}", self.value_label)));
        }
        Ok(())
    }

    pub(crate) fn read(self, target: &str) -> Result<String, String> {
        let target_w = wide_null_checked(OsStr::new(target), "credential target")?;
        let mut raw: *mut CREDENTIALW = ptr::null_mut();
        if unsafe { CredReadW(target_w.as_ptr(), CRED_TYPE_GENERIC, 0, &mut raw) } == 0 {
            return Err(last_os_error(&format!("read {}", self.value_label)));
        }
        if raw.is_null() {
            return Err(format!(
                "read {}: Windows returned a null credential",
                self.value_label
            ));
        }
        let _owner = OwnedCredential(raw);
        let credential = unsafe { &*raw };
        unsafe {
            decode_credential_blob(
                credential.CredentialBlob,
                credential.CredentialBlobSize,
                self.value_label,
            )
        }
    }

    pub(crate) fn delete_if_present(self, target: &str) -> Result<(), String> {
        let target_w = wide_null_checked(OsStr::new(target), "credential target")?;
        if unsafe { CredDeleteW(target_w.as_ptr(), CRED_TYPE_GENERIC, 0) } != 0 {
            return Ok(());
        }
        if unsafe { GetLastError() } == ERROR_NOT_FOUND {
            return Ok(());
        }
        Err(last_os_error(&format!("delete {}", self.value_label)))
    }
}

struct OwnedCredential(*mut CREDENTIALW);

impl Drop for OwnedCredential {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                CredFree(self.0.cast());
            }
        }
    }
}

unsafe fn decode_credential_blob(
    blob: *const u8,
    blob_len: u32,
    value_label: &str,
) -> Result<String, String> {
    let bytes = if blob_len == 0 {
        Vec::new()
    } else {
        if blob.is_null() {
            return Err(format!(
                "read {value_label}: Windows returned a null nonempty credential blob"
            ));
        }
        unsafe { std::slice::from_raw_parts(blob, blob_len as usize) }.to_vec()
    };
    String::from_utf8(bytes).map_err(|_| format!("{value_label} is not valid UTF-8"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_blob_decoder_accepts_empty_and_valid_utf8() {
        assert_eq!(
            unsafe { decode_credential_blob(std::ptr::null(), 0, "test secret") }.unwrap(),
            ""
        );
        let value = b"secret";
        assert_eq!(
            unsafe { decode_credential_blob(value.as_ptr(), value.len() as u32, "test secret") }
                .unwrap(),
            "secret"
        );
    }

    #[test]
    fn credential_blob_decoder_rejects_null_nonempty_and_invalid_utf8() {
        let null_error =
            unsafe { decode_credential_blob(std::ptr::null(), 1, "cloud token") }.unwrap_err();
        assert!(null_error.contains("null nonempty credential blob"));

        let invalid = [0xff];
        assert_eq!(
            unsafe { decode_credential_blob(invalid.as_ptr(), 1, "cloud token") }.unwrap_err(),
            "cloud token is not valid UTF-8"
        );
    }

    #[test]
    fn credential_store_preserves_domain_specific_diagnostic_labels() {
        assert_eq!(
            CredentialStore::new("cloud token").value_label,
            "cloud token"
        );
        assert_eq!(
            CredentialStore::new("osu! client secret").value_label,
            "osu! client secret"
        );
    }
}
