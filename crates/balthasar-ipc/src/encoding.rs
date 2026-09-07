//! Which encoding a body is in, and how to read and write it in that encoding.
//!
//! The family's reply shape is one shape in two encodings: JSON for anything that might be read
//! by a person or piped through a text tool, CBOR for a caller that is only going to parse it.
//! Both carry exactly the same fields; nothing is expressible in one and not the other.
//!
//! **Nothing is negotiated.** A body says what it is in its first byte, so a server answers in
//! whatever it was asked in and a caller that has never heard of CBOR is unaffected. A handshake
//! would be a second thing to keep in step, and a setting would be a third — and either would
//! have to be got right by both ends before the first call, which is precisely when there is
//! nothing to ask.

use serde::Serialize;
use serde::de::DeserializeOwned;

/// How a body on the wire is encoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wire {
    /// Text. The default, and what every existing caller sends.
    Json,
    /// Bytes, for a caller that is not going to read it.
    Cbor,
}

impl Wire {
    /// Which encoding `body` is in.
    ///
    /// JSON's top level here is an object or an array, so it begins `{` or `[` after any leading
    /// space. CBOR's is a map or an array, whose first byte is major type 4 or 5 — `0x80`–`0xBF`.
    /// The two ranges do not overlap, so this is a reading rather than a guess.
    #[must_use]
    pub fn of(body: &[u8]) -> Self {
        match body.iter().find(|b| !b.is_ascii_whitespace()) {
            Some(b'{' | b'[') => Self::Json,
            Some(0x80..=0xBF) => Self::Cbor,
            // Anything else is neither, and JSON gives the better error: a caller that sent
            // rubbish gets told what was wrong with it rather than "not a map".
            _ => Self::Json,
        }
    }

    /// Read `body` as `T`.
    ///
    /// # Errors
    /// When the body is not that shape in this encoding.
    pub fn read<T: DeserializeOwned>(self, body: &[u8]) -> Result<T, String> {
        match self {
            Self::Json => serde_json::from_slice(body).map_err(|why| why.to_string()),
            Self::Cbor => ciborium::from_reader(body).map_err(|why| why.to_string()),
        }
    }

    /// Write `value` in this encoding.
    ///
    /// # Errors
    /// When the value will not encode, which for the shapes on this wire cannot happen.
    pub fn write<T: Serialize>(self, value: &T) -> Result<Vec<u8>, String> {
        match self {
            Self::Json => serde_json::to_vec(value).map_err(|why| why.to_string()),
            Self::Cbor => {
                let mut bytes = Vec::new();
                ciborium::into_writer(value, &mut bytes).map_err(|why| why.to_string())?;
                Ok(bytes)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_body_says_which_encoding_it_is() {
        assert_eq!(Wire::of(br#"{"call":"verbs"}"#), Wire::Json);
        assert_eq!(
            Wire::of(b"  \n {\"call\":\"verbs\"}"),
            Wire::Json,
            "after space"
        );
        assert_eq!(Wire::of(b"[1,2]"), Wire::Json);

        let cbor = Wire::Cbor
            .write(&serde_json::json!({"call": "verbs"}))
            .expect("encode");
        assert_eq!(Wire::of(&cbor), Wire::Cbor);
        let list = Wire::Cbor
            .write(&serde_json::json!([1, 2]))
            .expect("encode");
        assert_eq!(Wire::of(&list), Wire::Cbor);
    }

    #[test]
    fn an_empty_body_is_not_taken_for_cbor() {
        // It is neither, and the reader has to pick one to report the failure in. JSON says
        // "EOF while parsing"; CBOR would say the body is not a map, which is true of every
        // malformed body and tells the caller nothing.
        assert_eq!(Wire::of(b""), Wire::Json);
        assert_eq!(Wire::of(b"garbage"), Wire::Json);
    }

    #[test]
    fn both_encodings_carry_the_same_value() {
        let value = serde_json::json!({"ok": true, "family": 1, "result": [1, "two"]});
        let as_json = Wire::Json.write(&value).expect("json");
        let as_cbor = Wire::Cbor.write(&value).expect("cbor");
        assert_ne!(as_json, as_cbor, "different bytes");
        let back_json: serde_json::Value = Wire::Json.read(&as_json).expect("json");
        let back_cbor: serde_json::Value = Wire::Cbor.read(&as_cbor).expect("cbor");
        assert_eq!(back_json, back_cbor, "one shape, two encodings");
    }

    #[test]
    fn a_round_trip_survives_being_sniffed() {
        // What the server actually does: it is handed bytes, works out the encoding, reads the
        // request, and answers in the same one.
        for wire in [Wire::Json, Wire::Cbor] {
            let body = wire
                .write(&serde_json::json!({"call": "recall"}))
                .expect("write");
            let seen = Wire::of(&body);
            assert_eq!(seen, wire);
            let back: serde_json::Value = seen.read(&body).expect("read");
            assert_eq!(back["call"], serde_json::json!("recall"));
        }
    }
}
