//! Reading one row in pieces, when the whole of it will not fit at once. A piece is the row it
//! came from, where in that row it starts, and what the row said when it was cut — the last so
//! that a row amended since does not look partly read.

use sha2::{Digest, Sha256};

/// What a row said when it was cut.
#[must_use]
pub fn revision(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

/// One piece of a row, starting `offset` bytes into it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    pub cursor: u64,
    pub offset: usize,
    pub text: String,
    pub revision: String,
}

impl Chunk {
    /// This piece as the store records it.
    #[must_use]
    pub fn piece(&self) -> crate::Piece {
        crate::Piece {
            offset: self.offset,
            length: self.text.len(),
        }
    }

    #[cfg(test)]
    #[must_use]
    pub fn end(&self) -> usize {
        self.offset + self.text.len()
    }
}

/// Cut `text` into pieces of at most `most` bytes, never through a character. A `most` narrower
/// than one character yields a piece per character rather than none.
#[must_use]
pub fn cut(cursor: u64, text: &str, most: usize) -> Vec<Chunk> {
    let revision = revision(text);
    if text.is_empty() {
        return Vec::new();
    }
    let most = most.max(1);
    let mut out = Vec::new();
    let mut offset = 0;
    while offset < text.len() {
        let want = (offset + most).min(text.len());
        let mut end = want;
        while end > offset && !text.is_char_boundary(end) {
            end -= 1;
        }
        // A character wider than the whole budget: take it rather than loop for ever.
        if end == offset {
            end = text[offset..]
                .char_indices()
                .nth(1)
                .map_or(text.len(), |(at, _)| offset + at);
        }
        out.push(Chunk {
            cursor,
            offset,
            text: text[offset..end].to_owned(),
            revision: revision.clone(),
        });
        offset = end;
    }
    out
}

/// How much of a row has been read, given the pieces recorded for it.
///
/// The contiguous run from the start and no further: a piece past a gap is not coverage of what
/// the gap holds, and reporting the furthest end would mark the gap read. The store returns
/// pieces of one revision, so a row amended since starts again from nothing.
#[must_use]
pub fn covered(read: &[crate::Piece]) -> usize {
    let mut ordered: Vec<&crate::Piece> = read.iter().collect();
    ordered.sort_by_key(|piece| piece.offset);
    let mut through = 0;
    for piece in ordered {
        if piece.offset > through {
            break;
        }
        through = through.max(piece.offset + piece.length);
    }
    through
}

#[cfg(test)]
#[path = "chunking/tests.rs"]
mod tests;
