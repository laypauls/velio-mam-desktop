//! 비밀 값(원격 접속 비밀번호·개인 키)의 보관 — Windows DPAPI(CurrentUser) 로 감싼 blob 만 DB 에 둔다.
//!
//! `mam-ai/src/keys/dpapi.rs`(외부 AI 키)와 같은 방식이지만 **복사했다**: `mam-ai` 는 ONNX Runtime 등 무거운 의존성을 끌고 와서
//! `mam-core` 가 의존할 수 없고, 의존 방향을 뒤집어 공용 crate 를 새로 만들면 `mam-ai` 변경이 따라온다(이 작업의 범위 밖).
//! 추가 엔트로피 문자열을 다르게 해(`ENTROPY`) 한쪽 blob 을 다른 쪽 용도로 복호할 수 없게 한다(용도 분리).
//!
//! [`Secret`] 은 `Debug`/`Display` 가 내용을 찍지 않고 `Serialize` 를 구현하지 않는다 — 로그·에러·IPC 로 새지 않게 하는 틀이다.

use std::fmt;

use zeroize::Zeroizing;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SecretError {
    #[error("secret protection failed: {0}")]
    Crypto(&'static str),
    #[error("secret protection is not supported on this platform")]
    Unsupported,
}

/// 비밀 값을 감싸고 푸는 수단. 운영은 [`Dpapi`], 시험은 가짜를 넣는다.
pub trait SecretBox: Send + Sync + fmt::Debug {
    fn seal(&self, plain: &[u8]) -> Result<Vec<u8>, SecretError>;
    fn open(&self, blob: &[u8]) -> Result<Zeroizing<Vec<u8>>, SecretError>;
}

/// Windows DPAPI (CurrentUser). 같은 Windows 사용자 계정에서만 복호된다. 다른 플랫폼에서는 `Unsupported`.
#[derive(Debug, Default, Clone, Copy)]
pub struct Dpapi;

impl SecretBox for Dpapi {
    fn seal(&self, plain: &[u8]) -> Result<Vec<u8>, SecretError> {
        imp::protect(plain)
    }

    fn open(&self, blob: &[u8]) -> Result<Zeroizing<Vec<u8>>, SecretError> {
        imp::unprotect(blob)
    }
}

/// 메모리에서 지워지는 비밀 문자열. 내용은 [`Secret::expose`] 로만 빌려 쓴다.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(Zeroizing<String>);

impl Secret {
    pub fn new(value: String) -> Self {
        Self(Zeroizing::new(value))
    }

    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

/// 입력(IPC 인자)으로만 읽는다 — `Serialize` 는 구현하지 않는다.
impl<'de> serde::Deserialize<'de> for Secret {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        String::deserialize(d).map(Self::new)
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

/// 원격 접속 비밀 전용 추가 엔트로피 — 비밀이 아니다(용도 분리용).
#[cfg(windows)]
const ENTROPY: &[u8] = b"mam-desktop/remote-source/v1";

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

    /// 출력 blob 을 복사해 옮기고, 시스템 할당 메모리는 0 으로 덮은 뒤 해제한다.
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

    pub fn protect(plain: &[u8]) -> Result<Vec<u8>, SecretError> {
        let (input, entropy) = (blob(plain), blob(ENTROPY));
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
            return Err(SecretError::Crypto("DPAPI protect failed"));
        }
        Ok(take(out).to_vec())
    }

    pub fn unprotect(cipher: &[u8]) -> Result<Zeroizing<Vec<u8>>, SecretError> {
        let (input, entropy) = (blob(cipher), blob(ENTROPY));
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
            return Err(SecretError::Crypto("DPAPI unprotect failed"));
        }
        Ok(take(out))
    }
}

#[cfg(not(windows))]
mod imp {
    use super::*;

    pub fn protect(_plain: &[u8]) -> Result<Vec<u8>, SecretError> {
        Err(SecretError::Unsupported)
    }

    pub fn unprotect(_cipher: &[u8]) -> Result<Zeroizing<Vec<u8>>, SecretError> {
        Err(SecretError::Unsupported)
    }
}

#[cfg(test)]
mod tests {
    //! 분류: Happy / Validation / Business rule (용도 분리, 비노출)
    use super::*;

    #[cfg(windows)]
    #[test]
    fn t_secret_01_dpapi_roundtrips_and_the_blob_does_not_contain_the_plaintext() {
        let plain = b"hunter2-very-secret";
        let blob = Dpapi.seal(plain).unwrap();
        assert!(!blob.windows(plain.len()).any(|w| w == plain));
        assert_eq!(&**Dpapi.open(&blob).unwrap(), plain);
    }

    #[cfg(windows)]
    #[test]
    fn t_secret_02_a_corrupted_blob_is_an_error_not_a_panic() {
        let mut blob = Dpapi.seal(b"abc").unwrap();
        let mid = blob.len() / 2;
        blob[mid] ^= 0xff;
        assert!(Dpapi.open(&blob).is_err());
        assert!(Dpapi.open(b"").is_err());
    }

    #[test]
    fn t_secret_03_debug_and_display_never_print_the_value() {
        let s = Secret::new("p@ssw0rd".into());
        assert!(!format!("{s:?}{s}").contains("p@ssw0rd"));
        assert_eq!(s.expose(), "p@ssw0rd");
    }
}
