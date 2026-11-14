//! Contact formats: email, URL, phone.

use super::FormatViolation;

/// Checks a host name: dot-separated labels of 1–63 ASCII letters, digits or hyphens, no
/// label starting or ending with a hyphen, at most 253 characters in total.
fn check_hostname(host: &str, require_dot: bool) -> Result<(), FormatViolation> {
    if host.is_empty() {
        return Err(FormatViolation::Structure);
    }
    if host.len() > 253 {
        return Err(FormatViolation::Length);
    }
    let mut labels = 0_usize;
    let mut last = "";
    for label in host.split('.') {
        if label.is_empty() {
            return Err(FormatViolation::Structure);
        }
        if label.len() > 63 {
            return Err(FormatViolation::Length);
        }
        if !label
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            return Err(FormatViolation::Character);
        }
        if label.starts_with('-') || label.ends_with('-') {
            return Err(FormatViolation::Structure);
        }
        labels = labels.saturating_add(1);
        last = label;
    }
    if require_dot && labels < 2 {
        return Err(FormatViolation::Structure);
    }
    // A top-level label made only of digits is not a domain (it would be an IP address).
    if last.bytes().all(|b| b.is_ascii_digit()) {
        return Err(FormatViolation::Structure);
    }
    Ok(())
}

/// Email address, practical subset of RFC 5321 / RFC 5322 "dot-atom" form.
///
/// Rule: `local@domain`, at most 254 characters; exactly one `@` separates them;
/// `local` is 1–64 characters from `A-Z a-z 0-9` and ``!#$%&'*+/=?^_`{|}~-.``, with no
/// leading, trailing or doubled dot; `domain` is a host name with at least two labels
/// (letters, digits, hyphens; labels of 1–63 characters; last label not all digits).
///
/// Not accepted on purpose: quoted local parts, comments, IP-literal domains and non-ASCII
/// addresses (internationalised domains must be given in punycode).
///
/// # Errors
/// The first [`FormatViolation`] found.
pub fn validate_email(text: &str) -> Result<(), FormatViolation> {
    if text.is_empty() {
        return Err(FormatViolation::Empty);
    }
    if text.len() > 254 {
        return Err(FormatViolation::Length);
    }
    let (local, domain) = text.rsplit_once('@').ok_or(FormatViolation::Structure)?;
    if local.is_empty() || local.len() > 64 {
        return Err(FormatViolation::Length);
    }
    let allowed = |b: u8| b.is_ascii_alphanumeric() || b"!#$%&'*+/=?^_`{|}~-.".contains(&b);
    if !local.bytes().all(allowed) {
        return Err(FormatViolation::Character);
    }
    if local.starts_with('.') || local.ends_with('.') || local.contains("..") {
        return Err(FormatViolation::Structure);
    }
    check_hostname(domain, true)
}

fn check_ipv4(host: &str) -> bool {
    let mut count = 0_usize;
    for part in host.split('.') {
        count = count.saturating_add(1);
        let plain = !part.is_empty() && part.len() <= 3 && part.bytes().all(|b| b.is_ascii_digit());
        let no_leading_zero = part.len() == 1 || !part.starts_with('0');
        if !plain || !no_leading_zero || part.parse::<u16>().map_or(true, |n| n > 255) {
            return false;
        }
    }
    count == 4
}

/// Web address (practical subset of RFC 3986 for `http` and `https`).
///
/// Rule: at most 2048 ASCII characters; scheme `http://` or `https://` (any letter case);
/// then a host that is a host name (single labels such as `localhost` are allowed), a
/// dotted IPv4 address, or a bracketed IPv6 literal (hexadecimal digits, `:` and `.`, at
/// least two colons); an optional `:port` (1–65535); then an optional path, query and
/// fragment made of printable ASCII without spaces or ``" < > \ ^ ` { | }``; every `%` must
/// be followed by two hexadecimal digits.
///
/// Not accepted on purpose: other schemes, user names or passwords in the address
/// (`user@host`), and raw non-ASCII characters (they must be percent-encoded or punycode).
///
/// # Errors
/// The first [`FormatViolation`] found.
pub fn validate_url(text: &str) -> Result<(), FormatViolation> {
    if text.is_empty() {
        return Err(FormatViolation::Empty);
    }
    if text.len() > 2048 {
        return Err(FormatViolation::Length);
    }
    if !text.bytes().all(|b| b.is_ascii_graphic()) {
        return Err(FormatViolation::Character);
    }
    let (scheme, rest) = text.split_once("://").ok_or(FormatViolation::Structure)?;
    if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
        return Err(FormatViolation::UnknownCode);
    }
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = rest.get(..authority_end).unwrap_or("");
    let tail = rest.get(authority_end..).unwrap_or("");
    if authority.contains('@') {
        return Err(FormatViolation::Structure);
    }
    // Split host and port; an IPv6 literal contains colons itself.
    let (host, port) = if let Some(v6) = authority.strip_prefix('[') {
        let (inside, after) = v6.split_once(']').ok_or(FormatViolation::Structure)?;
        let colons = inside.bytes().filter(|b| *b == b':').count();
        if colons < 2
            || !inside
                .bytes()
                .all(|b| b.is_ascii_hexdigit() || b == b':' || b == b'.')
        {
            return Err(FormatViolation::Structure);
        }
        match after.strip_prefix(':') {
            Some(port) => (None, Some(port)),
            None if after.is_empty() => (None, None),
            None => return Err(FormatViolation::Structure),
        }
    } else {
        match authority.split_once(':') {
            Some((host, port)) => (Some(host), Some(port)),
            None => (Some(authority), None),
        }
    };
    if let Some(host) = host {
        if !check_ipv4(host) {
            check_hostname(host, false)?;
        }
    }
    if let Some(port) = port {
        if port.is_empty() || port.len() > 5 || !port.bytes().all(|b| b.is_ascii_digit()) {
            return Err(FormatViolation::Structure);
        }
        if port.parse::<u32>().map_or(true, |p| p == 0 || p > 65_535) {
            return Err(FormatViolation::Range);
        }
    }
    if tail.bytes().any(|b| b"\"<>\\^`{|}".contains(&b)) {
        return Err(FormatViolation::Character);
    }
    let mut bytes = tail.bytes();
    while let Some(b) = bytes.next() {
        if b == b'%' {
            let hex = |b: Option<u8>| b.is_some_and(|b| b.is_ascii_hexdigit());
            if !hex(bytes.next()) || !hex(bytes.next()) {
                return Err(FormatViolation::Structure);
            }
        }
    }
    Ok(())
}

/// International phone number in ITU-T E.164 written form.
///
/// Rule: `+` followed by 7 to 15 digits, the first of which is not `0` (country codes never
/// start with zero). E.164 fixes the 15-digit maximum; the 7-digit minimum is a practical
/// floor (country code plus the shortest national numbers in use). No spaces or punctuation;
/// use [`super::FormatId::normalize`] to strip them first. National formats are not checked.
///
/// # Errors
/// The first [`FormatViolation`] found.
pub fn validate_phone_e164(text: &str) -> Result<(), FormatViolation> {
    if text.is_empty() {
        return Err(FormatViolation::Empty);
    }
    let digits = text.strip_prefix('+').ok_or(FormatViolation::Structure)?;
    if !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(FormatViolation::Character);
    }
    if !(7..=15).contains(&digits.len()) {
        return Err(FormatViolation::Length);
    }
    if digits.starts_with('0') {
        return Err(FormatViolation::Structure);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use FormatViolation as V;

    #[test]
    fn std_email_valid() {
        for good in [
            "a@example.com",
            "first.last@example.co.jp",
            "user+tag@sub.example.org",
            "o'brien@example.ie",
            "x_y-z@ex-ample.com",
            "1234567890@example.com",
            "a@xn--zckzah.jp",
            "!#$%&'*+/=?^_`{|}~-@example.com",
            "a@b.co",
            "A@EXAMPLE.COM",
        ] {
            assert_eq!(validate_email(good), Ok(()), "{good}");
        }
    }

    #[test]
    fn invalid_email() {
        let cases: &[(&str, V)] = &[
            ("", V::Empty),
            ("plain", V::Structure),
            ("@example.com", V::Length),
            ("a@", V::Structure),
            ("a@example", V::Structure),
            ("a@.com", V::Structure),
            ("a@example..com", V::Structure),
            ("a@example.com.", V::Structure),
            (".a@example.com", V::Structure),
            ("a.@example.com", V::Structure),
            ("a..b@example.com", V::Structure),
            ("a b@example.com", V::Character),
            ("a@b@example.com", V::Character),
            ("\"a b\"@example.com", V::Character),
            ("a@exa_mple.com", V::Character),
            ("a@-example.com", V::Structure),
            ("a@example-.com", V::Structure),
            ("a@example.123", V::Structure),
            ("a@[127.0.0.1]", V::Character),
            ("ü@example.com", V::Character),
            ("a@exämple.com", V::Character),
            ("a@example.com\n", V::Character),
        ];
        for (bad, why) in cases {
            assert_eq!(validate_email(bad), Err(*why), "{bad:?}");
        }
    }

    #[test]
    fn edge_email_lengths() {
        let local64 = "a".repeat(64);
        assert_eq!(validate_email(&format!("{local64}@example.com")), Ok(()));
        assert_eq!(
            validate_email(&format!("{local64}a@example.com")),
            Err(V::Length)
        );
        let label63 = "b".repeat(63);
        assert_eq!(validate_email(&format!("a@{label63}.com")), Ok(()));
        assert_eq!(validate_email(&format!("a@{label63}b.com")), Err(V::Length));
        // 254 characters in total is the limit.
        let domain = format!("{label63}.{label63}.{label63}.{}", "c".repeat(60));
        assert_eq!(domain.len(), 252);
        assert_eq!(validate_email(&format!("a@{domain}")), Ok(()));
        assert_eq!(validate_email(&format!("ab@{domain}")), Err(V::Length));
        assert_eq!(validate_email(&format!("abc@{domain}")), Err(V::Length));
        // Host name longer than 253 characters (inside a shorter-than-limit check path).
        assert_eq!(
            check_hostname(
                &format!("{label63}.{label63}.{label63}.{}", "c".repeat(62)),
                true
            ),
            Err(V::Length)
        );
    }

    #[test]
    fn std_url_valid() {
        for good in [
            "https://example.com",
            "http://example.com/",
            "HTTPS://Example.COM/Path?q=1&r=a%20b#frag",
            "https://sub.example.co.jp:8443/a/b.html",
            "http://localhost",
            "http://localhost:3000/x",
            "http://192.168.0.1/admin",
            "http://192.168.0.1:80",
            "https://[2001:db8::1]/x",
            "https://[::1]:8080",
            "https://[::ffff:192.0.2.1]",
            "https://example.com?x=1",
            "https://example.com#top",
            "https://example.com/~user/(a)[b];c=d,e!*'$+@:",
            "https://xn--zckzah.jp/",
        ] {
            assert_eq!(validate_url(good), Ok(()), "{good}");
        }
    }

    #[test]
    fn invalid_url() {
        let cases: &[(&str, V)] = &[
            ("", V::Empty),
            ("example.com", V::Structure),
            ("https:/example.com", V::Structure),
            ("ftp://example.com", V::UnknownCode),
            ("javascript://alert(1)", V::UnknownCode),
            ("mailto:a@example.com", V::Structure),
            ("https://", V::Structure),
            ("https:///path", V::Structure),
            ("https://user@example.com", V::Structure),
            ("https://user:pw@example.com", V::Structure),
            ("https://exa mple.com", V::Character),
            ("https://example.com/a b", V::Character),
            ("https://example.com/\u{7f}", V::Character),
            ("https://例え.jp", V::Character),
            ("https://exa_mple.com", V::Character),
            ("https://-example.com", V::Structure),
            ("https://example..com", V::Structure),
            ("https://256.1.1.1", V::Structure),
            ("https://1.2.3", V::Structure),
            ("https://01.2.3.4", V::Structure),
            ("https://1.2.3.4.5", V::Structure),
            ("https://example.com:", V::Structure),
            ("https://example.com:abc", V::Structure),
            ("https://example.com:123456", V::Structure),
            ("https://example.com:0", V::Range),
            ("https://example.com:65536", V::Range),
            ("https://[::1", V::Structure),
            ("https://[1]", V::Structure),
            ("https://[::g]", V::Structure),
            ("https://[::1]x", V::Structure),
            ("https://[::1]:", V::Structure),
            ("https://example.com/<script>", V::Character),
            ("https://example.com/a|b", V::Character),
            ("https://example.com/a\\b", V::Character),
            ("https://example.com/%", V::Structure),
            ("https://example.com/%2", V::Structure),
            ("https://example.com/%zz", V::Structure),
        ];
        for (bad, why) in cases {
            assert_eq!(validate_url(bad), Err(*why), "{bad:?}");
        }
    }

    #[test]
    fn edge_url_length_and_ports() {
        let base = "https://example.com/";
        let ok = format!("{base}{}", "a".repeat(2048 - base.len()));
        assert_eq!(validate_url(&ok), Ok(()));
        assert_eq!(validate_url(&format!("{ok}a")), Err(V::Length));
        assert_eq!(validate_url("https://example.com:1"), Ok(()));
        assert_eq!(validate_url("https://example.com:65535"), Ok(()));
        assert_eq!(validate_url("http://0.0.0.0"), Ok(()));
        assert_eq!(validate_url("http://255.255.255.255"), Ok(()));
    }

    #[test]
    fn std_phone() {
        for good in [
            "+14155552671",
            "+819012345678",
            "+821012345678",
            "+4915112345678",
            "+919876543210",
            "+2901234",
            "+123456789012345",
        ] {
            assert_eq!(validate_phone_e164(good), Ok(()), "{good}");
        }
    }

    #[test]
    fn invalid_phone() {
        let cases: &[(&str, V)] = &[
            ("", V::Empty),
            ("14155552671", V::Structure),
            ("+", V::Length),
            ("+123456", V::Length),
            ("+1234567890123456", V::Length),
            ("+0123456789", V::Structure),
            ("+1 415 555 2671", V::Character),
            ("+1-415-555-2671", V::Character),
            ("++14155552671", V::Character),
            ("+1415555267a", V::Character),
            ("+１４１５５５５２６７１", V::Character),
        ];
        for (bad, why) in cases {
            assert_eq!(validate_phone_e164(bad), Err(*why), "{bad:?}");
        }
    }
}
