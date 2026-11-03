use chacha20poly1305::{
    aead::{Aead, Payload},
    KeyInit, XChaCha20Poly1305, XNonce,
};
use rand::{rngs::OsRng, RngCore};
use std::io;
use zeroize::{Zeroize, Zeroizing};

const AAD: &[u8] = b"server-dashboard:n-sight-api-token:v1";

pub fn harden_process() -> Result<(), String> {
    #[cfg(target_os = "linux")]
    unsafe {
        let limit = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        if libc::setrlimit(libc::RLIMIT_CORE, &limit) != 0 {
            return Err(format!(
                "Core-Dumps konnten nicht deaktiviert werden: {}",
                io::Error::last_os_error()
            ));
        }
        if libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0) != 0 {
            return Err(format!(
                "PR_SET_DUMPABLE konnte nicht gesetzt werden: {}",
                io::Error::last_os_error()
            ));
        }
    }
    Ok(())
}

fn lock_region(ptr: *const u8, len: usize) -> Result<(), String> {
    if len == 0 {
        return Ok(());
    }
    #[cfg(target_os = "linux")]
    unsafe {
        if libc::mlock(ptr.cast(), len) != 0 {
            return Err(format!(
                "Sicherer Speicher konnte nicht gesperrt werden (mlock): {}",
                io::Error::last_os_error()
            ));
        }
    }
    Ok(())
}

fn unlock_region(ptr: *const u8, len: usize) {
    if len == 0 {
        return;
    }
    #[cfg(target_os = "linux")]
    unsafe {
        let _ = libc::munlock(ptr.cast(), len);
    }
}

struct LockedKey {
    bytes: Box<[u8; 32]>,
}

impl LockedKey {
    fn random() -> Result<Self, String> {
        let mut bytes = Box::new([0u8; 32]);
        OsRng.fill_bytes(bytes.as_mut());
        lock_region(bytes.as_ptr(), bytes.len())?;
        Ok(Self { bytes })
    }

    fn as_slice(&self) -> &[u8] {
        self.bytes.as_ref()
    }
}

impl Drop for LockedKey {
    fn drop(&mut self) {
        self.bytes.zeroize();
        unlock_region(self.bytes.as_ptr(), self.bytes.len());
    }
}

pub struct LockedBytes {
    bytes: Zeroizing<Vec<u8>>,
    locked_len: usize,
}

impl LockedBytes {
    fn new(bytes: Vec<u8>) -> Result<Self, String> {
        let bytes = Zeroizing::new(bytes);
        lock_region(bytes.as_ptr(), bytes.len())?;
        Ok(Self {
            locked_len: bytes.len(),
            bytes,
        })
    }

    pub fn as_slice(&self) -> &[u8] {
        self.bytes.as_slice()
    }
}

impl Drop for LockedBytes {
    fn drop(&mut self) {
        self.bytes.zeroize();
        unlock_region(self.bytes.as_ptr(), self.locked_len);
        self.locked_len = 0;
    }
}

pub struct EncryptedSecret {
    key: LockedKey,
    nonce: [u8; 24],
    ciphertext: Zeroizing<Vec<u8>>,
}

impl EncryptedSecret {
    pub fn seal(plaintext: &mut Zeroizing<Vec<u8>>) -> Result<Self, String> {
        if plaintext.is_empty() {
            return Err("Der API-Token ist leer.".into());
        }
        if plaintext.len() > 16 * 1024 {
            return Err("Der API-Token ist unerwartet groß.".into());
        }

        let key = LockedKey::random()?;
        let cipher = XChaCha20Poly1305::new_from_slice(key.as_slice())
            .map_err(|_| "Verschlüsselung konnte nicht initialisiert werden.".to_string())?;
        let mut nonce = [0u8; 24];
        OsRng.fill_bytes(&mut nonce);
        let ciphertext = cipher
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: plaintext.as_slice(),
                    aad: AAD,
                },
            )
            .map_err(|_| "Der API-Token konnte nicht verschlüsselt werden.".to_string())?;

        plaintext.zeroize();
        Ok(Self {
            key,
            nonce,
            ciphertext: Zeroizing::new(ciphertext),
        })
    }

    pub fn expose(&self) -> Result<LockedBytes, String> {
        let cipher = XChaCha20Poly1305::new_from_slice(self.key.as_slice())
            .map_err(|_| "Verschlüsselung konnte nicht initialisiert werden.".to_string())?;
        let plaintext = cipher
            .decrypt(
                XNonce::from_slice(&self.nonce),
                Payload {
                    msg: self.ciphertext.as_slice(),
                    aad: AAD,
                },
            )
            .map_err(|_| {
                "Der API-Token konnte im Arbeitsspeicher nicht entschlüsselt werden.".to_string()
            })?;
        LockedBytes::new(plaintext)
    }
}

impl Drop for EncryptedSecret {
    fn drop(&mut self) {
        self.ciphertext.zeroize();
        self.nonce.zeroize();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_secret() {
        let mut input = Zeroizing::new(b"secret-token-123".to_vec());
        let encrypted = EncryptedSecret::seal(&mut input).unwrap();
        assert!(input.iter().all(|b| *b == 0));
        let clear = encrypted.expose().unwrap();
        assert_eq!(clear.as_slice(), b"secret-token-123");
    }
}
