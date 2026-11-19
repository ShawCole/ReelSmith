//! Random ids and capability tokens. Tokens are 32 bytes, hex-encoded, and never logged.

use crate::error::Error;

pub fn token() -> Result<String, Error> {
    Ok(hex(&random_bytes(32)?))
}

pub fn nid(prefix: &str) -> Result<String, Error> {
    Ok(format!("{prefix}_{}", hex(&random_bytes(8)?)))
}

pub fn random_bytes(n: usize) -> Result<Vec<u8>, Error> {
    let mut buf = vec![0u8; n];
    getrandom::getrandom(&mut buf).map_err(|_| Error::invalid("random source failed"))?;
    Ok(buf)
}

pub fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    out
}

/// Compare two equal-length secrets without branching on the bytes.
pub fn tokens_equal(a: &str, b: &str) -> bool {
    let a = a.as_bytes();
    let b = b.as_bytes();
    if a.len() != b.len() || a.is_empty() {
        return false;
    }
    let mut diff = 0u8;
    for (left, right) in a.iter().zip(b.iter()) {
        diff |= left ^ right;
    }
    diff == 0
}
