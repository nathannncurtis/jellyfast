use crate::config::{config_dir, KEYCHAIN_SERVICE};
use crate::*;
use std::fs;
use std::path::PathBuf;

// ---- Keychain (security-framework) with documented fallback ------------
//
// Primary: macOS Keychain generic password (service = KEYCHAIN_SERVICE,
// account = "token@<server-url>"). The `security-framework` crate is a
// thin, audited binding; no plaintext token ever touches disk.
//
// Fallback (only if SecItemAdd fails — e.g. broken keychain in CI):
// base64 token in token.fallback with 0600 perms. This is intentionally
// loud (tracing::warn!) and NOT encrypted; documented per the spec.

pub fn keychain_account(server: &str) -> String {
    format!("token@{}", server)
}

// Test seam: unit tests never touch the real Keychain; they use this
// in-memory map instead. Compiled out of non-test builds.
#[cfg(test)]
static TEST_STORE: std::sync::Mutex<Option<std::collections::HashMap<String, String>>> =
    std::sync::Mutex::new(None);

#[cfg(test)]
pub(crate) fn test_store_clear() {
    *TEST_STORE.lock().unwrap_or_else(|e| e.into_inner()) = Some(Default::default());
}

#[cfg(test)]
fn test_store<R>(f: impl FnOnce(&mut std::collections::HashMap<String, String>) -> R) -> R {
    let mut g = TEST_STORE.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(Default::default))
}

pub fn store_token(server: &str, token: &str) {
    #[cfg(test)]
    {
        test_store(|m| m.insert(keychain_account(server), token.to_string()));
        return;
    }
    #[cfg(not(test))]
    {
        use security_framework::passwords;
        match passwords::set_generic_password(KEYCHAIN_SERVICE, &keychain_account(server), token.as_bytes()) {
            Ok(_) => {
                let _ = std::fs::remove_file(fallback_token_path()); // clean stale fallback
                tracing::info!("token stored in Keychain");
            }
            Err(e) => {
                tracing::warn!("Keychain unavailable ({}); using documented fallback file", e);
                store_fallback(token);
            }
        }
    }
}

pub fn load_token(server: &str) -> Option<String> {
    #[cfg(test)]
    {
        return test_store(|m| m.get(&keychain_account(server)).cloned());
    }
    #[cfg(not(test))]
    {
        use security_framework::passwords;
        match passwords::get_generic_password(KEYCHAIN_SERVICE, &keychain_account(server)) {
            Ok(t) if !t.is_empty() => String::from_utf8(t).ok(),
            _ => load_fallback(),
        }
    }
}

pub fn delete_token(server: &str) {
    #[cfg(test)]
    {
        test_store(|m| m.remove(&keychain_account(server)));
    }
    #[cfg(not(test))]
    {
        use security_framework::passwords;
        let _ = passwords::delete_generic_password(KEYCHAIN_SERVICE, &keychain_account(server));
    }
    let _ = fs::remove_file(fallback_token_path());
}

#[cfg_attr(test, allow(dead_code))]
fn store_fallback(token: &str) {
    let p = fallback_token_path();
    let _ = std::fs::create_dir_all(config_dir());
    let b64 = {
        use std::io::Write;
        let mut buf = Vec::new();
        let mut enc = base64_writer(&mut buf);
        let _ = enc.write_all(token.as_bytes());
        drop(enc);
        String::from_utf8(buf).unwrap_or_default()
    };
    let _ = fs::write(&p, b64);
    set_owner_only(&p);
}

/// The fallback file stores base64; it must be decoded on the way back out.
#[cfg_attr(test, allow(dead_code))]
fn load_fallback() -> Option<String> {
    let raw = fs::read_to_string(fallback_token_path()).ok()?;
    let bytes = base64_decode(raw.trim())?;
    String::from_utf8(bytes).ok().filter(|t| !t.is_empty())
}

fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut acc = 0u32;
    let mut bits = 0;
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => break,
            _ => return None,
        } as u32;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

fn fallback_token_path() -> PathBuf {
    config_dir().join("token.fallback")
}

fn set_owner_only(p: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = fs::set_permissions(p, fs::Permissions::from_mode(0o600));
}

// Minimal base64 (stdlib only; avoids the `base64` crate dependency).
fn base64_writer(out: &mut Vec<u8>) -> impl std::io::Write + '_ {
    struct W<'a>(&'a mut Vec<u8>);
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    impl<'a> std::io::Write for W<'a> {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            for chunk in b.chunks(3) {
                let n = (chunk[0] as u32) << 16
                    | (*chunk.get(1).unwrap_or(&0) as u32) << 8
                    | *chunk.get(2).unwrap_or(&0) as u32;
                self.0.push(T[(n >> 18) as usize & 63] as u8);
                self.0.push(T[(n >> 12) as usize & 63] as u8);
                self.0.push(if chunk.len() > 1 { T[(n >> 6) as usize & 63] } else { b'=' });
                self.0.push(if chunk.len() > 2 { T[n as usize & 63] } else { b'=' });
            }
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    W(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::test_env::TestEnv;

    #[test]
    fn account_name() {
        assert_eq!(keychain_account("http://h:1"), "token@http://h:1");
    }

    #[test]
    fn store_load_delete_in_memory() {
        let _e = TestEnv::new();
        assert_eq!(load_token("http://a"), None);
        store_token("http://a", "t1");
        store_token("http://b", "t2");
        assert_eq!(load_token("http://a").as_deref(), Some("t1"));
        store_token("http://a", "t3");
        assert_eq!(load_token("http://a").as_deref(), Some("t3"));
        delete_token("http://a");
        assert_eq!(load_token("http://a"), None);
        assert_eq!(load_token("http://b").as_deref(), Some("t2"));
    }

    #[test]
    fn fallback_roundtrips_through_base64() {
        let _e = TestEnv::new();
        for t in ["a", "ab", "abc", "abcd", "0123456789abcdef0123456789abcdef", "tök/+="] {
            store_fallback(t);
            assert_eq!(load_fallback().as_deref(), Some(t), "token {t}");
        }
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(fallback_token_path()).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        delete_token("x");
        assert_eq!(load_fallback(), None);
    }
}
