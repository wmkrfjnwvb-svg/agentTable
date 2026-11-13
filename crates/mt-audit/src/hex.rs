//! Lowercase hexadecimal helpers (no external dependency).

use core::fmt::{self, Write};

/// Writes `bytes` as lowercase hex.
pub(crate) fn write_hex(f: &mut fmt::Formatter<'_>, bytes: &[u8]) -> fmt::Result {
    for b in bytes {
        f.write_char(nibble(b >> 4))?;
        f.write_char(nibble(b & 0x0f))?;
    }
    Ok(())
}

/// Returns `bytes` as a lowercase hex string.
pub(crate) fn to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len().saturating_mul(2));
    for b in bytes {
        s.push(nibble(b >> 4));
        s.push(nibble(b & 0x0f));
    }
    s
}

fn nibble(n: u8) -> char {
    char::from(if n < 10 {
        b'0'.wrapping_add(n)
    } else {
        b'a'.wrapping_add(n.wrapping_sub(10))
    })
}

fn value(c: u8, allow_upper: bool) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c.wrapping_sub(b'0')),
        b'a'..=b'f' => Some(c.wrapping_sub(b'a').wrapping_add(10)),
        b'A'..=b'F' if allow_upper => Some(c.wrapping_sub(b'A').wrapping_add(10)),
        0..=u8::MAX => None,
    }
}

/// Decodes exactly `N` bytes from `2 * N` hex characters. `allow_upper` permits `A-F`.
pub(crate) fn decode<const N: usize>(text: &str, allow_upper: bool) -> Option<[u8; N]> {
    let raw = text.as_bytes();
    if raw.len() != N.checked_mul(2)? {
        return None;
    }
    let mut out = [0u8; N];
    for (slot, pair) in out.iter_mut().zip(raw.chunks_exact(2)) {
        let mut it = pair.iter();
        let hi = value(*it.next()?, allow_upper)?;
        let lo = value(*it.next()?, allow_upper)?;
        *slot = (hi << 4) | lo;
    }
    Some(out)
}
