//! Shared utilities — hostname, PID checks.

/// Check whether a process is alive by sending signal 0.
pub fn is_pid_alive(pid: i64) -> bool {
    #[cfg(unix)]
    {
        let Ok(pid_i32) = i32::try_from(pid) else {
            return false;
        };
        if pid_i32 <= 0 {
            return false;
        }
        // SAFETY: kill(pid, 0) only checks process existence, sends no signal
        unsafe { libc::kill(pid_i32, 0) == 0 }
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        true
    }
}

/// Return the local hostname.
pub fn hostname() -> String {
    let mut buf = [0u8; 256];
    #[cfg(unix)]
    unsafe {
        libc::gethostname(buf.as_mut_ptr() as *mut libc::c_char, buf.len());
    }
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_process_is_alive() {
        let pid = std::process::id() as i64;
        assert!(is_pid_alive(pid));
    }

    #[test]
    fn invalid_pid_not_alive() {
        assert!(!is_pid_alive(0));
        assert!(!is_pid_alive(-1));
        assert!(!is_pid_alive(i64::MAX));
    }

    #[test]
    fn hostname_non_empty() {
        let h = hostname();
        assert!(!h.is_empty());
    }
}
