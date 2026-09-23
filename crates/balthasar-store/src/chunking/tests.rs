use super::*;

/// Chunks as the store records them.
fn as_pieces(chunks: &[Chunk]) -> Vec<crate::Piece> {
    chunks.iter().map(Chunk::piece).collect()
}

const ROW: u64 = 7;

/// Every piece lands on a character boundary, whatever the text is made of.
mod mem_utf8_chunk_boundaries {
    use super::*;

    /// A piece that split a character would not be text, and the row is not recoverable from it.
    fn whole(text: &str, most: usize) {
        let pieces = cut(ROW, text, most);
        let back: String = pieces.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(back, text, "the row does not come back from its pieces");
        for piece in &pieces {
            assert!(
                text.is_char_boundary(piece.offset) && text.is_char_boundary(piece.end()),
                "a piece cut through a character: {piece:?}"
            );
        }
    }

    #[test]
    fn multibyte_text_is_cut_between_characters() {
        for most in 1..=12 {
            whole("héllo wörld", most);
            whole("日本語のテキスト", most);
            whole("a😀b😀c", most);
            whole("combining e\u{0301}", most);
        }
    }

    #[test]
    fn a_piece_is_never_wider_than_asked_unless_one_character_is() {
        let pieces = cut(ROW, "日本語", 4);
        for piece in &pieces {
            assert!(piece.text.len() <= 4, "{piece:?}");
        }
        // One character of three bytes against a budget of one: taken whole rather than dropped.
        let tight = cut(ROW, "日本語", 1);
        assert_eq!(tight.len(), 3);
        assert_eq!(tight[0].text, "日");
    }

    #[test]
    fn nothing_is_cut_from_nothing() {
        assert!(cut(ROW, "", 10).is_empty());
    }

    #[test]
    fn a_row_that_fits_is_one_piece() {
        let pieces = cut(ROW, "short", 100);
        assert_eq!(pieces.len(), 1);
        assert_eq!(pieces[0].offset, 0);
        assert_eq!(pieces[0].text, "short");
    }
}

/// A piece read is coverage of that piece, not of the row it came from.
mod mem_partial_chunk_does_not_cover_whole_row {
    use super::*;

    #[test]
    fn reading_the_head_covers_the_head_alone() {
        let text = "a".repeat(100);
        let pieces = cut(ROW, &text, 10);
        assert_eq!(covered(&as_pieces(&pieces[..1])), 10);
        assert_eq!(covered(&as_pieces(&pieces[..3])), 30);
        assert_eq!(
            covered(&as_pieces(&pieces)),
            100,
            "all of it, once all read"
        );
    }

    #[test]
    fn a_piece_past_a_gap_does_not_cover_the_gap() {
        // The tail arriving first must not mark the middle read: what is in the gap has not been
        // seen, and coverage that says otherwise loses it for good.
        let text = "a".repeat(100);
        let pieces = cut(ROW, &text, 10);
        let skipped = [pieces[0].clone(), pieces[5].clone(), pieces[9].clone()];
        assert_eq!(
            covered(&as_pieces(&skipped)),
            10,
            "coverage stops at the first gap, not at the furthest piece"
        );
    }

    #[test]
    fn nothing_read_is_nothing_covered() {
        assert_eq!(covered(&[]), 0);
    }

    #[test]
    fn a_row_amended_since_starts_again_from_nothing() {
        // Its pieces name a revision the store no longer returns, so nothing is covered.
        let before = cut(ROW, &"a".repeat(50), 10);
        let after = cut(ROW, &"b".repeat(50), 10);
        assert_ne!(before[0].revision, after[0].revision);
        assert_eq!(
            covered(&[]),
            0,
            "no piece of the new revision has been read"
        );
    }

    #[test]
    fn pieces_arriving_out_of_order_still_cover_their_run() {
        let text = "a".repeat(40);
        let mut pieces = cut(ROW, &text, 10);
        pieces.reverse();
        assert_eq!(covered(&as_pieces(&pieces)), 40);
    }
}

/// What a piece is identified by.
mod identity {
    use super::*;

    #[test]
    fn every_piece_of_a_row_names_the_same_revision() {
        let text = "a".repeat(45);
        let pieces = cut(ROW, &text, 10);
        assert!(pieces.iter().all(|p| p.revision == revision(&text)));
        assert!(pieces.iter().all(|p| p.cursor == ROW));
    }

    #[test]
    fn a_revision_follows_the_text_and_nothing_else() {
        assert_eq!(revision("the same"), revision("the same"));
        assert_ne!(revision("the same"), revision("the same "));
    }

    #[test]
    fn offsets_run_end_to_end_with_no_gap_and_no_overlap() {
        let pieces = cut(ROW, &"x".repeat(95), 10);
        for pair in pieces.windows(2) {
            assert_eq!(pair[0].end(), pair[1].offset, "{pair:?}");
        }
        assert_eq!(pieces[0].offset, 0);
        assert_eq!(pieces.last().expect("a piece").end(), 95);
    }
}
