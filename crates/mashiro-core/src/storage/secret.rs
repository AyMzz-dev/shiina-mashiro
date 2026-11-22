//! secret 落盘保护：Windows DPAPI（当前用户作用域）加密后 base64 存库。
//! 密文带 `dpapi:` 前缀；无前缀的旧数据按明文透传（迁移友好），
//! 非 Windows 平台退化为明文存储。

use base64::Engine;

const PREFIX: &str = "dpapi:";
const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

/// 明文 → 入库字符串。
pub fn seal(plain: &str) -> String {
    if plain.is_empty() {
        return String::new();
    }
    match imp::protect(plain.as_bytes()) {
        Some(blob) => format!("{PREFIX}{}", B64.encode(blob)),
        None => {
            tracing::warn!("DPAPI protect failed, secret stored plaintext");
            plain.to_string()
        }
    }
}

/// 入库字符串 → 明文（插件/鉴权层使用）。
pub fn reveal(stored: &str) -> String {
    match stored.strip_prefix(PREFIX) {
        None => stored.to_string(),
        Some(b64) => match B64.decode(b64).ok().and_then(|b| imp::unprotect(&b)) {
            Some(plain) => String::from_utf8(plain).unwrap_or_default(),
            None => {
                tracing::warn!("DPAPI unprotect failed (换用户/系统后旧 secret 需重录)");
                String::new()
            }
        },
    }
}

#[cfg(windows)]
mod imp {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CryptProtectData, CryptUnprotectData, CRYPT_INTEGER_BLOB,
    };

    fn crypt(data: &[u8], unprotect: bool) -> Option<Vec<u8>> {
        unsafe {
            let input = CRYPT_INTEGER_BLOB {
                cbData: data.len() as u32,
                pbData: data.as_ptr() as *mut u8,
            };
            let mut out = CRYPT_INTEGER_BLOB {
                cbData: 0,
                pbData: std::ptr::null_mut(),
            };
            let ok = if unprotect {
                CryptUnprotectData(
                    &input,
                    std::ptr::null_mut(),
                    std::ptr::null(),
                    std::ptr::null(),
                    std::ptr::null(),
                    0,
                    &mut out,
                )
            } else {
                CryptProtectData(
                    &input,
                    std::ptr::null(),
                    std::ptr::null(),
                    std::ptr::null(),
                    std::ptr::null(),
                    0,
                    &mut out,
                )
            };
            if ok == 0 || out.pbData.is_null() {
                return None;
            }
            let buf = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
            LocalFree(out.pbData.cast());
            Some(buf)
        }
    }

    pub fn protect(data: &[u8]) -> Option<Vec<u8>> {
        crypt(data, false)
    }

    pub fn unprotect(data: &[u8]) -> Option<Vec<u8>> {
        crypt(data, true)
    }
}

#[cfg(not(windows))]
mod imp {
    pub fn protect(_: &[u8]) -> Option<Vec<u8>> {
        None
    }
    pub fn unprotect(_: &[u8]) -> Option<Vec<u8>> {
        None
    }
}
