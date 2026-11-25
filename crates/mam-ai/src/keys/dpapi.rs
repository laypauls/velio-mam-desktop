//! Windows DPAPI(CurrentUser) 래퍼. 같은 Windows 사용자 계정에서만 복호할 수 있다.
//! Windows 가 아닌 플랫폼에서는 항상 `Unsupported` 를 돌려준다 (키는 세션 메모리로만 쓴다).

use zeroize::Zeroizing;

use super::KeyError;

/// 앱 고유 추가 엔트로피 — 다른 앱이 같은 사용자 권한으로 blob 을 그대로 복호하기 어렵게 한다 (비밀이 아님).
const ENTROPY: &[u8] = b"mam-desktop/external-ai-key/v1";

#[cfg(windows)]
mod imp {
    use super::*;
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
    };

    fn blob(data: &[u8]) -> CRYPT_INTEGER_BLOB {
        CRYPT_INTEGER_BLOB {
            cbData: data.len() as u32,
            pbData: data.as_ptr() as *mut u8,
        }
    }

    /// 출력 blob 을 복사해 Vec 으로 옮기고, 시스템 할당 메모리는 0 으로 덮은 뒤 해제한다.
    fn take(out: CRYPT_INTEGER_BLOB) -> Zeroizing<Vec<u8>> {
        // SAFETY: 성공한 호출이 채운 (pbData, cbData) 는 LocalAlloc 된 유효한 버퍼다.
        unsafe {
            let slice = std::slice::from_raw_parts_mut(out.pbData, out.cbData as usize);
            let v = Zeroizing::new(slice.to_vec());
            slice.fill(0);
            LocalFree(out.pbData as *mut core::ffi::c_void);
            v
        }
    }

    pub fn protect(plain: &[u8]) -> Result<Vec<u8>, KeyError> {
        let input = blob(plain);
        let entropy = blob(ENTROPY);
        let mut out = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: std::ptr::null_mut(),
        };
        // SAFETY: 입력 blob 들은 호출 동안 살아 있고, out 은 유효한 쓰기 대상이다.
        let ok = unsafe {
            CryptProtectData(
                &input,
                std::ptr::null(),
                &entropy,
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )
        };
        if ok == 0 {
            return Err(KeyError::Crypto("DPAPI protect failed"));
        }
        Ok(take(out).to_vec())
    }

    pub fn unprotect(cipher: &[u8]) -> Result<Zeroizing<Vec<u8>>, KeyError> {
        let input = blob(cipher);
        let entropy = blob(ENTROPY);
        let mut out = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: std::ptr::null_mut(),
        };
        // SAFETY: protect 와 같다. 설명 문자열 출력은 요청하지 않는다(null).
        let ok = unsafe {
            CryptUnprotectData(
                &input,
                std::ptr::null_mut(),
                &entropy,
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )
        };
        if ok == 0 {
            return Err(KeyError::Crypto("DPAPI unprotect failed"));
        }
        Ok(take(out))
    }
}

#[cfg(not(windows))]
mod imp {
    use super::*;

    pub fn protect(_plain: &[u8]) -> Result<Vec<u8>, KeyError> {
        let _ = ENTROPY;
        Err(KeyError::Unsupported)
    }

    pub fn unprotect(_cipher: &[u8]) -> Result<Zeroizing<Vec<u8>>, KeyError> {
        Err(KeyError::Unsupported)
    }
}

pub(super) use imp::{protect, unprotect};
