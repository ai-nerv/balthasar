//! Which encoding a body is in — a body says what it is in its first byte, nothing negotiated.

use serde::Serialize;
use serde::de::DeserializeOwned;

/// How a body on the wire is encoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wire {
    Json,
    Cbor,
}

impl Wire {
    /// Which encoding `body` is in.
    ///
    /// JSON begins `{` or `[` after any leading space; CBOR.s map or array is major type 4 or 5,
    /// `0x80`–`0xBF`. The two ranges do not overlap.
    #[must_use]
    pub fn of(body: &[u8]) -> Self {
        match body.iter().find(|b| !b.is_ascii_whitespace()) {
            Some(b'{' | b'[') => Self::Json,
            Some(0x80..=0xBF) => Self::Cbor,
            // Neither, and JSON gives the better error.
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
