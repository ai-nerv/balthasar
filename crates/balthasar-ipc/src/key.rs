//! The key `serve` is handed on its standard input when it starts, and which every call on its
//! socket must carry.

use std::io::{ErrorKind, Read};

/// How long the key is: 32 random bytes, as lowercase hex.
const LENGTH: usize = 64;

/// What every refusal for a missing or wrong key says.
pub const REFUSED: &str = "a call to serve needs the key it was started with";

/// The key `serve` was started with. Never printed: its `Debug` says only that it is there.
pub struct Key(String);

impl std::fmt::Debug for Key {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Key(..)")
    }
}

impl Key {
    /// One line of 64 lowercase hex characters and a newline, read byte by byte so nothing after
    /// it is consumed and no closed stdin is waited on beyond the line.
    ///
    /// # Errors
    /// When the line is missing, unterminated, or not 64 lowercase hex characters.
    pub fn read(from: &mut impl Read) -> Result<Self, String> {
        const WANTED: &str = "serve takes its key on standard input: one line of 64 lowercase \
                              hex characters";
        let mut line = Vec::with_capacity(LENGTH);
        let mut byte = [0_u8; 1];
        loop {
            match from.read(&mut byte) {
                Ok(0) => return Err(format!("{WANTED}, and none arrived")),
                Ok(_) if byte == [b'\n'] => break,
                Ok(_) if line.len() == LENGTH => return Err(format!("{WANTED}; that is longer")),
                Ok(_) => line.extend_from_slice(&byte),
                Err(why) if why.kind() == ErrorKind::Interrupted => {}
                Err(why) => return Err(format!("{WANTED}, and it could not be read: {why}")),
            }
        }
        let hex = |b: &u8| b.is_ascii_digit() || (b'a'..=b'f').contains(b);
        if line.len() != LENGTH || !line.iter().all(hex) {
            return Err(format!("{WANTED}; that is not one"));
        }
        String::from_utf8(line)
            .map(Self)
            .map_err(|_| format!("{WANTED}; that is not one"))
    }

    /// Whether `presented` is this key, compared without stopping at the first difference.
    #[must_use]
    pub fn admits(&self, presented: Option<&str>) -> bool {
        let Some(presented) = presented else {
            return false;
        };
        let (ours, theirs) = (self.0.as_bytes(), presented.as_bytes());
        let mut differ = ours.len() ^ theirs.len();
        for (at, byte) in ours.iter().enumerate() {
            differ |= usize::from(byte ^ theirs.get(at).copied().unwrap_or(0));
        }
        std::hint::black_box(differ) == 0
    }
}

/// The call in `body`, when it decodes, is written in a family this program reads, and carries
/// `key`; otherwise the refusal to answer it with. The key is taken out before the call goes on.
pub(crate) fn admitted(
    key: &Key,
    wire: crate::Wire,
    body: &[u8],
) -> Result<crate::Request, crate::Reply> {
    let not_a_request = |why: &dyn std::fmt::Display| {
        crate::Reply::refused(format!("that is not a request: {why}"))
    };
    let mut call: serde_json::Value = wire.read(body).map_err(|why| not_a_request(&why))?;
    let fields = call
        .as_object_mut()
        .ok_or_else(|| not_a_request(&"a call is an object"))?;
    if let Some(family) = fields.get("family").and_then(serde_json::Value::as_u64)
        && family > u64::from(crate::FAMILY)
    {
        return Err(crate::Reply::refused(format!(
            "this call is written in family {family}, newer than this balthasar's {}",
            crate::FAMILY
        )));
    }
    let presented = fields.remove("key");
    if !key.admits(presented.as_ref().and_then(serde_json::Value::as_str)) {
        return Err(crate::Reply::refused(REFUSED));
    }
    serde_json::from_value(call).map_err(|why| not_a_request(&why))
}

/// A key and its hex, for tests that serve and call.
#[cfg(test)]
pub(crate) fn for_tests() -> (Key, &'static str) {
    const HEX: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    (Key(HEX.to_owned()), HEX)
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn one_line_of_lowercase_hex_is_a_key_and_nothing_after_it_is_read() {
        let fed = format!("{GOOD}\nleft");
        let mut reader = fed.as_bytes();
        let key = Key::read(&mut reader).expect("a key");
        assert_eq!(reader, b"left");
        assert!(key.admits(Some(GOOD)));
    }

    #[test]
    fn a_missing_or_malformed_key_is_refused_by_name() {
        let upper = GOOD.to_uppercase();
        let long = format!("{GOOD}0\n");
        for fed in [
            "",
            GOOD,
            "abc\n",
            &format!("{upper}\n"),
            &long,
            &format!("{GOOD} \n"),
        ] {
            let why = Key::read(&mut fed.as_bytes()).expect_err("refused");
            assert!(why.contains("64 lowercase hex"), "{fed:?}: {why}");
        }
    }

    #[test]
    fn only_the_same_key_is_admitted_and_it_never_prints() {
        let key = Key::read(&mut format!("{GOOD}\n").as_bytes()).expect("key");
        let other = GOOD.replace('0', "1");
        for wrong in [None, Some(""), Some(&GOOD[..63]), Some(other.as_str())] {
            assert!(!key.admits(wrong), "{wrong:?}");
        }
        assert!(!format!("{key:?}").contains("0123"));
    }
}
