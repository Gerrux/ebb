//! Local protection for Private cards.
//!
//! DPAPI binds the ciphertext to the current Windows user profile. The
//! database therefore contains no reusable application key and a copied
//! profile/database cannot be opened by another Windows user. This is a
//! deliberate small vault, not a password-manager replacement.

use windows::Win32::Foundation::{LocalFree, HLOCAL};
use windows::Win32::Security::Cryptography::{
    CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
};

const ENTROPY_PREFIX: &[u8] = b"Ebb private card v1:";

fn entropy(card_id: i64) -> Vec<u8> {
    let mut out = ENTROPY_PREFIX.to_vec();
    out.extend_from_slice(&card_id.to_le_bytes());
    out
}

fn blob(bytes: &mut [u8]) -> CRYPT_INTEGER_BLOB {
    CRYPT_INTEGER_BLOB {
        cbData: bytes.len() as u32,
        pbData: bytes.as_mut_ptr(),
    }
}

pub fn protect(card_id: i64, plaintext: &str) -> windows::core::Result<Vec<u8>> {
    let mut input = plaintext.as_bytes().to_vec();
    let mut extra = entropy(card_id);
    let input_blob = blob(&mut input);
    let entropy_blob = blob(&mut extra);
    let mut output = CRYPT_INTEGER_BLOB::default();
    unsafe {
        CryptProtectData(
            &input_blob,
            windows::core::PCWSTR::null(),
            Some(&entropy_blob),
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )?;
        let bytes = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        let _ = LocalFree(Some(HLOCAL(output.pbData as _)));
        input.fill(0);
        Ok(bytes)
    }
}

pub fn unprotect(card_id: i64, ciphertext: &[u8]) -> windows::core::Result<String> {
    let mut input = ciphertext.to_vec();
    let mut extra = entropy(card_id);
    let input_blob = blob(&mut input);
    let entropy_blob = blob(&mut extra);
    let mut output = CRYPT_INTEGER_BLOB::default();
    unsafe {
        CryptUnprotectData(
            &input_blob,
            None,
            Some(&entropy_blob),
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )?;
        let bytes = std::slice::from_raw_parts(output.pbData, output.cbData as usize);
        let text = String::from_utf8(bytes.to_vec());
        std::ptr::write_bytes(output.pbData, 0, output.cbData as usize);
        let _ = LocalFree(Some(HLOCAL(output.pbData as _)));
        input.fill(0);
        text.map_err(|_| windows::core::Error::from_thread())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn entropy_is_card_bound() {
        assert_ne!(super::entropy(1), super::entropy(2));
        assert!(super::entropy(1).starts_with(super::ENTROPY_PREFIX));
    }
}
