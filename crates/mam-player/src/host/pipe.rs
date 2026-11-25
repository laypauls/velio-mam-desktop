//! 제어 채널 Named Pipe. 동기(non-overlapped) 핸들은 읽기 대기 중 같은 핸들의 쓰기가 직렬화돼 교착할 수 있으므로
//! 방향별로 2개를 쓴다: `<이름>.cmd`(host → worker), `<이름>.evt`(worker → host). 서버(워커)가 만들고 호스트가 이름으로 연다.
//!
//! 보안 (계약 §13.6): 이름은 128비트 난수(`mam-player-<32 hex>`), DACL 은 **현재 사용자 SID 만** 허용,
//! 첫 인스턴스 플래그로 선점 방지, 원격 클라이언트 거부. 이름은 워커 인자로 검증한다 (`is_valid_pipe_name`).

use std::ffi::c_void;
use std::fs::{File, OpenOptions};
use std::io;
use std::os::windows::io::{FromRawHandle, RawHandle};

use windows::Win32::Foundation::{
    CloseHandle, ERROR_PIPE_CONNECTED, GetLastError, HANDLE, HLOCAL, INVALID_HANDLE_VALUE,
    LocalFree,
};
use windows::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows::Win32::Security::Cryptography::{BCRYPT_USE_SYSTEM_PREFERRED_RNG, BCryptGenRandom};
use windows::Win32::Security::{
    GetTokenInformation, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
    TokenUser,
};
use windows::Win32::Storage::FileSystem::{
    FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAGS_AND_ATTRIBUTES, PIPE_ACCESS_INBOUND,
    PIPE_ACCESS_OUTBOUND,
};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT,
};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::core::{HSTRING, PCWSTR, PWSTR};

const PREFIX: &str = "mam-player-";

fn full_name(name: &str, suffix: &str) -> String {
    format!(r"\\.\pipe\{name}.{suffix}")
}

/// 파이프 이름이 `mam-player-<32자리 소문자 hex>` 형식인가 — 경로 구분자·`..` 로 다른 파이프를 가리키지 못하게 한다
pub fn is_valid_pipe_name(name: &str) -> bool {
    name.strip_prefix(PREFIX).is_some_and(|h| {
        h.len() == 32
            && h.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

/// 128비트 난수 파이프 이름 (OS 난수 — 추측 불가)
pub fn random_pipe_name() -> io::Result<String> {
    let mut buf = [0u8; 16];
    // SAFETY: 길이가 맞는 버퍼, 시스템 기본 RNG 사용 (알고리즘 핸들 불필요)
    let st = unsafe { BCryptGenRandom(None, &mut buf, BCRYPT_USE_SYSTEM_PREFERRED_RNG) };
    if st.0 < 0 {
        return Err(io::Error::other(format!(
            "BCryptGenRandom failed: {:#x}",
            st.0
        )));
    }
    Ok(format!(
        "{PREFIX}{}",
        buf.iter().map(|b| format!("{b:02x}")).collect::<String>()
    ))
}

/// 현재 프로세스 토큰의 사용자 SID 문자열 (`S-1-5-21-...`)
pub fn current_user_sid() -> io::Result<String> {
    // SAFETY: 이 함수 안에서 연 토큰 핸들만 쓰고 닫는다. 버퍼는 GetTokenInformation 이 알려준 크기로 할당한다
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token)
            .map_err(|e| io::Error::other(e.to_string()))?;
        let result = (|| {
            let mut needed = 0u32;
            let _ = GetTokenInformation(token, TokenUser, None, 0, &mut needed);
            if needed == 0 {
                return Err(io::Error::other("token user size unknown"));
            }
            let mut buf = vec![0u8; needed as usize];
            GetTokenInformation(
                token,
                TokenUser,
                Some(buf.as_mut_ptr() as *mut c_void),
                needed,
                &mut needed,
            )
            .map_err(|e| io::Error::other(e.to_string()))?;
            let tu = &*(buf.as_ptr() as *const TOKEN_USER);
            let mut s = PWSTR::null();
            ConvertSidToStringSidW(tu.User.Sid, &mut s)
                .map_err(|e| io::Error::other(e.to_string()))?;
            let out = s.to_string().map_err(|e| io::Error::other(e.to_string()));
            let _ = LocalFree(Some(HLOCAL(s.0 as *mut c_void)));
            out
        })();
        let _ = CloseHandle(token);
        result
    }
}

/// 현재 사용자에게만 모든 접근을 허용하는 DACL (보호됨 — 상속 없음)
pub fn owner_only_sddl(sid: &str) -> String {
    format!("D:P(A;;GA;;;{sid})")
}

/// 현재 사용자 전용 보안 기술자 (해제는 Drop 에서 LocalFree)
struct OwnerOnly {
    sd: PSECURITY_DESCRIPTOR,
}

impl OwnerOnly {
    fn new() -> io::Result<Self> {
        let sddl = HSTRING::from(owner_only_sddl(&current_user_sid()?));
        let mut sd = PSECURITY_DESCRIPTOR::default();
        // SAFETY: 유효한 널 종료 SDDL, 출력 포인터는 LocalFree 로 해제한다
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                PCWSTR(sddl.as_ptr()),
                SDDL_REVISION_1,
                &mut sd,
                None,
            )
        }
        .map_err(|e| io::Error::other(e.to_string()))?;
        Ok(Self { sd })
    }

    fn attributes(&self) -> SECURITY_ATTRIBUTES {
        SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: self.sd.0,
            bInheritHandle: false.into(),
        }
    }
}

impl Drop for OwnerOnly {
    fn drop(&mut self) {
        // SAFETY: ConvertStringSecurityDescriptor... 가 LocalAlloc 으로 만든 블록
        unsafe {
            let _ = LocalFree(Some(HLOCAL(self.sd.0)));
        }
    }
}

/// 파이프 1개를 만든다 (인스턴스 1개, 원격 거부, 첫 인스턴스, 현재 사용자 DACL)
fn create(
    name: &str,
    suffix: &str,
    access: FILE_FLAGS_AND_ATTRIBUTES,
    dacl: &OwnerOnly,
) -> io::Result<File> {
    let n = HSTRING::from(full_name(name, suffix));
    let sa = dacl.attributes();
    // SAFETY: sa 는 이 호출 동안 유효하다. 반환 핸들의 소유권은 File 로 넘긴다
    let h: HANDLE = unsafe {
        CreateNamedPipeW(
            &n,
            access | FILE_FLAG_FIRST_PIPE_INSTANCE,
            PIPE_TYPE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            1,
            64 * 1024,
            64 * 1024,
            0,
            Some(&sa as *const SECURITY_ATTRIBUTES),
        )
    };
    if h == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { File::from_raw_handle(h.0 as RawHandle) })
}

/// 워커(서버) 쪽 파이프 쌍 (아직 연결 전)
pub struct ServerPipes {
    cmd: File,
    evt: File,
}

impl ServerPipes {
    /// 이름 검증 → 두 파이프를 모두 먼저 만든다 (호스트가 어느 순서로 열어도 이름이 이미 있다)
    pub fn create(name: &str) -> io::Result<Self> {
        if !is_valid_pipe_name(name) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid pipe name",
            ));
        }
        let dacl = OwnerOnly::new()?;
        Ok(Self {
            cmd: create(name, "cmd", PIPE_ACCESS_INBOUND, &dacl)?,
            evt: create(name, "evt", PIPE_ACCESS_OUTBOUND, &dacl)?,
        })
    }

    /// 호스트가 둘 다 열 때까지 대기. 반환: (명령 읽기, 이벤트 쓰기)
    pub fn accept(self) -> io::Result<(File, File)> {
        connect(&self.cmd)?;
        connect(&self.evt)?;
        Ok((self.cmd, self.evt))
    }

    #[cfg(test)]
    fn cmd_handle(&self) -> HANDLE {
        use std::os::windows::io::AsRawHandle;
        HANDLE(self.cmd.as_raw_handle())
    }
}

fn connect(f: &File) -> io::Result<()> {
    use std::os::windows::io::AsRawHandle;
    let h = HANDLE(f.as_raw_handle());
    // SAFETY: f 가 살아 있는 동안 유효한 서버 파이프 핸들
    match unsafe { ConnectNamedPipe(h, None) } {
        Ok(()) => Ok(()),
        // 클라이언트가 ConnectNamedPipe 호출 전에 이미 붙은 경우 — 정상
        Err(_) if unsafe { GetLastError() } == ERROR_PIPE_CONNECTED => Ok(()),
        Err(e) => Err(io::Error::other(e.to_string())),
    }
}

/// 호스트(클라이언트) 쪽 열기. 파이프가 아직 없으면 NotFound. 반환: (명령 쓰기, 이벤트 읽기)
pub fn open_client(name: &str) -> io::Result<(File, File)> {
    if !is_valid_pipe_name(name) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid pipe name",
        ));
    }
    let cmd = OpenOptions::new()
        .write(true)
        .open(full_name(name, "cmd"))?;
    let evt = OpenOptions::new().read(true).open(full_name(name, "evt"))?;
    Ok((cmd, evt))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};

    // 분류: Happy / External(상대 종료) / Validation(이름) / 보안(DACL)

    #[test]
    fn t_player_pipe_01_round_trip_both_directions() {
        let name = random_pipe_name().unwrap();
        let server = ServerPipes::create(&name).unwrap();
        let t = std::thread::spawn(move || {
            let (cmd, mut evt) = server.accept().unwrap();
            let mut line = String::new();
            BufReader::new(cmd).read_line(&mut line).unwrap();
            evt.write_all(line.to_uppercase().as_bytes()).unwrap();
        });
        let (mut cmd, evt) = open_client(&name).unwrap();
        cmd.write_all(b"hello\n").unwrap();
        let mut got = String::new();
        BufReader::new(evt).read_line(&mut got).unwrap();
        t.join().unwrap();
        assert_eq!(got, "HELLO\n");
    }

    #[test]
    fn t_player_pipe_02_reader_sees_eof_when_peer_drops() {
        let name = random_pipe_name().unwrap();
        let server = ServerPipes::create(&name).unwrap();
        let t = std::thread::spawn(move || {
            let (_cmd, evt) = server.accept().unwrap();
            drop(evt);
        });
        let (_cmd, evt) = open_client(&name).unwrap();
        t.join().unwrap();
        let mut line = String::new();
        assert_eq!(
            BufReader::new(evt).read_line(&mut line).unwrap_or(0),
            0,
            "EOF(또는 broken pipe)"
        );
    }

    #[test]
    fn t_player_pipe_03_missing_or_malformed_names_are_errors() {
        assert!(
            open_client(&random_pipe_name().unwrap()).is_err(),
            "없는 파이프"
        );
        for bad in [
            "",
            "mam-player-",
            "mam-player-xyz",
            r"mam-player-..\..\x",
            "other-00000000000000000000000000000000",
            "mam-player-0000000000000000000000000000000G",
        ] {
            assert!(!is_valid_pipe_name(bad), "{bad}");
            assert!(ServerPipes::create(bad).is_err(), "{bad}");
            assert!(open_client(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn t_player_pipe_04_names_are_random_128_bit() {
        let (a, b) = (random_pipe_name().unwrap(), random_pipe_name().unwrap());
        assert_ne!(a, b);
        assert!(is_valid_pipe_name(&a) && is_valid_pipe_name(&b));
    }

    #[test]
    fn t_player_pipe_05_second_server_with_the_same_name_is_refused() {
        // 첫 인스턴스 플래그 — 같은 이름을 선점해 두는 공격을 막는다
        let name = random_pipe_name().unwrap();
        let _first = ServerPipes::create(&name).unwrap();
        assert!(ServerPipes::create(&name).is_err());
    }

    #[test]
    fn t_player_pipe_06_dacl_allows_only_the_current_user() {
        use windows::Win32::Security::Authorization::{
            ConvertSecurityDescriptorToStringSecurityDescriptorW, GetSecurityInfo, SE_KERNEL_OBJECT,
        };
        use windows::Win32::Security::{ACL, DACL_SECURITY_INFORMATION};
        // Arrange
        let name = random_pipe_name().unwrap();
        let server = ServerPipes::create(&name).unwrap();
        let sid = current_user_sid().unwrap();
        // Act: 만든 파이프의 실제 DACL 을 SDDL 로 읽는다
        let mut sd = PSECURITY_DESCRIPTOR::default();
        let mut dacl: *mut ACL = std::ptr::null_mut();
        let sddl = unsafe {
            GetSecurityInfo(
                server.cmd_handle(),
                SE_KERNEL_OBJECT,
                DACL_SECURITY_INFORMATION,
                None,
                None,
                Some(&mut dacl),
                None,
                Some(&mut sd),
            )
            .ok()
            .unwrap();
            let mut s = PWSTR::null();
            ConvertSecurityDescriptorToStringSecurityDescriptorW(
                sd,
                SDDL_REVISION_1,
                DACL_SECURITY_INFORMATION,
                &mut s,
                None,
            )
            .unwrap();
            let out = s.to_string().unwrap();
            let _ = LocalFree(Some(HLOCAL(s.0 as *mut c_void)));
            let _ = LocalFree(Some(HLOCAL(sd.0)));
            out
        };
        // Assert: ACE 는 하나, 현재 사용자 SID 만. Everyone/Users/Authenticated Users 등 없음
        assert_eq!(sddl.matches("(A;").count(), 1, "{sddl}");
        assert!(sddl.contains(&sid), "{sddl}");
        for broad in [";;;WD)", ";;;BU)", ";;;AU)", ";;;AN)", ";;;S-1-1-0)"] {
            assert!(!sddl.contains(broad), "{sddl}");
        }
        assert_eq!(owner_only_sddl("S-1-5-21-1"), "D:P(A;;GA;;;S-1-5-21-1)");
    }
}
