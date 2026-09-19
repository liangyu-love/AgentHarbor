use std::path::Path;

// Windows binds the encrypted file to the current user's logon credentials.
#[cfg(windows)]
fn transform(data: &[u8], encrypt: bool) -> Result<Vec<u8>, String> {
    use windows_sys::Win32::{Foundation::LocalFree, Security::Cryptography::*};
    if data.len() > u32::MAX as usize { return Err("newapi.errors.storage".into()); }
    let input = CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 };
    let mut output = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
    unsafe {
        let ok = if encrypt {
            CryptProtectData(&input, std::ptr::null(), std::ptr::null(), std::ptr::null(), std::ptr::null(), CRYPTPROTECT_UI_FORBIDDEN, &mut output)
        } else {
            CryptUnprotectData(&input, std::ptr::null_mut(), std::ptr::null(), std::ptr::null(), std::ptr::null(), CRYPTPROTECT_UI_FORBIDDEN, &mut output)
        };
        if ok == 0 { return Err("newapi.errors.storage".into()); }
        let result = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        LocalFree(output.pbData.cast());
        Ok(result)
    }
}

pub fn read(path: &Path) -> Result<Option<Vec<u8>>, String> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("newapi.errors.storage".into()),
    };
    #[cfg(windows)]
    let bytes = transform(&bytes, false)?;
    Ok(Some(bytes))
}

pub fn write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    #[cfg(windows)]
    let bytes = transform(bytes, true)?;
    // On Unix, write_atomic creates a file readable only by its owner (0600).
    super::storage::write_atomic(path, &bytes).map_err(|_| "newapi.errors.storage".into())
}
