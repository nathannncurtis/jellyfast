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

pub fn store_token(server: &str, token: &str) {
    use security_framework::passwords;
    match passwords::set_generic_password(KEYCHAIN_SERVICE, &keychain_account(server), token.as_bytes()) {
        Ok(_) => {
            let _ = std::fs::remove_file(fallback_token_path()); // clean stale fallback
            tracing::info!("token stored in Keychain");
        }
        Err(e) => {
            tracing::warn!("Keychain unavailable ({}); using documented fallback file", e);
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
    }
}

pub fn load_token(server: &str) -> Option<String> {
    use security_framework::passwords;
    match passwords::get_generic_password(KEYCHAIN_SERVICE, &keychain_account(server)) {
        Ok(t) if !t.is_empty() => String::from_utf8(t).ok(),
        _ => {
            let p = fallback_token_path();
            fs::read_to_string(p).ok().map(|s| s.trim().to_string())
        }
    }
}

pub fn delete_token(server: &str) {
    use security_framework::passwords;
    let _ = passwords::delete_generic_password(KEYCHAIN_SERVICE, &keychain_account(server));
    let _ = fs::remove_file(fallback_token_path());
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
