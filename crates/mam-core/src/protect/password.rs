//! Argon2id 해시 (D1) — 메모리 19MiB · 반복 2 · 병렬 1, 16바이트 솔트, PHC 문자열.
//!
//! 평문은 이 모듈 밖으로 나가지 않으며 내부 복사본은 즉시 폐기(`zeroize`)한다. 호출자가 들고 있는 평문은
//! `Zeroizing<String>` 으로 받아 쓰는 것을 권장한다.

use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::{Algorithm, Argon2, Params, Version};
use zeroize::Zeroizing;

use super::ProtectError;

/// 최소 길이 (문자 수) — UI 와 같다
pub const MIN_PASSWORD_CHARS: usize = 8;
/// 해시 비용을 입력 크기로 키우지 못하게 하는 상한 (문자 수)
pub const MAX_PASSWORD_CHARS: usize = 256;

const MEMORY_KIB: u32 = 19 * 1024;
const ITERATIONS: u32 = 2;
const PARALLELISM: u32 = 1;
const SALT_BYTES: usize = 16;

fn argon2() -> Result<Argon2<'static>, ProtectError> {
    let params = Params::new(MEMORY_KIB, ITERATIONS, PARALLELISM, None)
        .map_err(|_| ProtectError::Internal("invalid argon2 parameters".into()))?;
    Ok(Argon2::new(Algorithm::Argon2id, Version::V0x13, params))
}

/// 새 비밀번호 검증: 문자 수 8~256
pub fn validate_new_password(password: &str) -> Result<(), ProtectError> {
    let n = password.chars().count();
    if n < MIN_PASSWORD_CHARS {
        return Err(ProtectError::InvalidInput(format!(
            "password must be at least {MIN_PASSWORD_CHARS} characters"
        )));
    }
    if n > MAX_PASSWORD_CHARS {
        return Err(ProtectError::InvalidInput(format!(
            "password must be at most {MAX_PASSWORD_CHARS} characters"
        )));
    }
    Ok(())
}

/// 평문 -> PHC 문자열 (settings 에 저장할 값). 길이 검증 포함.
pub fn hash_password(password: &str) -> Result<String, ProtectError> {
    validate_new_password(password)?;
    let mut salt = Zeroizing::new([0u8; SALT_BYTES]);
    getrandom::fill(&mut *salt)
        .map_err(|_| ProtectError::Internal("random generator unavailable".into()))?;
    let salt = SaltString::encode_b64(&*salt)
        .map_err(|_| ProtectError::Internal("salt encoding failed".into()))?;
    let hash = argon2()?
        .hash_password(password.as_bytes(), &salt)
        .map_err(|_| ProtectError::Internal("password hashing failed".into()))?;
    Ok(hash.to_string())
}

/// 저장된 PHC 문자열과 비교. 불일치는 `PasswordIncorrect`, 저장값 손상은 `Internal`.
pub fn verify_password_hash(stored: &str, password: &str) -> Result<(), ProtectError> {
    let parsed = PasswordHash::new(stored)
        .map_err(|_| ProtectError::Internal("stored password hash is invalid".into()))?;
    match Argon2::default().verify_password(password.as_bytes(), &parsed) {
        Ok(()) => Ok(()),
        Err(argon2::password_hash::Error::Password) => Err(ProtectError::PasswordIncorrect),
        Err(_) => Err(ProtectError::Internal(
            "password verification failed".into(),
        )),
    }
}

#[cfg(test)]
mod tests {
    //! 분류: Happy / Validation / Business rule

    use super::*;

    #[test]
    fn t_prot_hash_01_phc_is_argon2id_with_required_params_and_no_plaintext() {
        let pw = "correct horse battery";
        let phc = hash_password(pw).unwrap();
        assert!(phc.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"), "{phc}");
        assert!(!phc.contains(pw));
        assert!(verify_password_hash(&phc, pw).is_ok());
        assert!(matches!(
            verify_password_hash(&phc, "wrong password!"),
            Err(ProtectError::PasswordIncorrect)
        ));
    }

    #[test]
    fn t_prot_hash_02_salt_is_random_per_hash() {
        assert_ne!(
            hash_password("same-password").unwrap(),
            hash_password("same-password").unwrap()
        );
    }

    #[test]
    fn t_prot_hash_03_length_bounds_count_characters_not_bytes() {
        assert!(validate_new_password("1234567").is_err());
        assert!(validate_new_password("12345678").is_ok());
        assert!(validate_new_password("가나다라마바사아").is_ok()); // 8자 (24바이트)
        assert!(validate_new_password("가나다라마바사").is_err()); // 7자
        assert!(validate_new_password(&"a".repeat(MAX_PASSWORD_CHARS + 1)).is_err());
    }

    #[test]
    fn t_prot_hash_04_corrupt_stored_hash_is_internal_without_echoing_it() {
        let err = verify_password_hash("not-a-phc-string", "whatever-pass").unwrap_err();
        assert_eq!(err.code(), "internal");
        assert!(!err.to_string().contains("not-a-phc-string"));
    }
}
