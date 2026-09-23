use super::*;

fn half() -> Policy {
    Policy {
        share: 0.50,
        margin: 0,
    }
}

/// The limit the documented arithmetic produces, over the windows models actually have.
mod mem_budget_denominator {
    use super::*;

    #[test]
    fn half_the_window_is_the_limit_while_the_reply_leaves_room_for_it() {
        // L = min(floor(C * 0.5), C - R) - M, and here the policy is the binding half.
        for (capacity, reply, limit) in [
            (32_000_u32, 4_000_u32, 16_000_u32),
            (128_000, 8_000, 64_000),
            (200_000, 32_000, 100_000),
            (1_000_000, 64_000, 500_000),
        ] {
            let room = Room::of(half(), Some(capacity), reply).expect("a usable window");
            assert_eq!(room.limit, limit, "{capacity} window");
            assert_eq!(room.capacity, capacity, "the real window is kept");
            assert_eq!(room.ceiling, capacity / 2);
            assert_eq!(room.reply, reply);
        }
    }

    #[test]
    fn a_reply_larger_than_the_spare_half_binds_instead_of_the_policy() {
        // C - R is the smaller of the two here, and the limit follows it rather than the share.
        let room = Room::of(half(), Some(32_000), 24_000).expect("a usable window");
        assert_eq!(room.ceiling, 16_000, "the policy's share is still recorded");
        assert_eq!(room.limit, 8_000, "but the reservation is what binds");
    }

    #[test]
    fn the_margin_comes_off_whichever_bound_won() {
        let policy = Policy {
            share: 0.50,
            margin: 1_000,
        };
        assert_eq!(
            Room::of(policy, Some(128_000), 8_000)
                .expect("usable")
                .limit,
            63_000
        );
    }

    #[test]
    fn the_physical_window_is_never_reported_as_half_of_itself() {
        // Halving the reported window would give the same limit and lose the question of which
        // bound produced it.
        let room = Room::of(half(), Some(200_000), 32_000).expect("usable");
        assert_eq!(room.capacity, 200_000);
        assert_ne!(room.capacity, room.ceiling);
    }
}

/// What the arithmetic does at its edges, where a silent zero would read as a valid budget.
mod mem_budget_checked_arithmetic {
    use super::*;

    #[test]
    fn an_unknown_window_is_refused_rather_than_treated_as_none() {
        assert_eq!(
            Room::of(half(), None, 1_000),
            Err(Unsatisfiable::UnknownCapacity)
        );
        assert_eq!(
            Room::of(half(), Some(0), 1_000),
            Err(Unsatisfiable::UnknownCapacity)
        );
    }

    #[test]
    fn a_reply_that_fills_the_window_is_refused_rather_than_leaving_nothing() {
        assert_eq!(
            Room::of(half(), Some(8_000), 8_000),
            Err(Unsatisfiable::ReplyFillsCapacity {
                capacity: 8_000,
                reply: 8_000,
            })
        );
        assert!(matches!(
            Room::of(half(), Some(8_000), 9_000),
            Err(Unsatisfiable::ReplyFillsCapacity { .. })
        ));
    }

    #[test]
    fn a_margin_wider_than_the_room_is_refused_rather_than_underflowing() {
        assert_eq!(
            Room::of(
                Policy {
                    share: 0.50,
                    margin: 9_000
                },
                Some(16_000),
                1_000
            ),
            Err(Unsatisfiable::MarginFillsRoom {
                room: 8_000,
                margin: 9_000,
            })
        );
    }

    #[test]
    fn a_share_that_is_not_a_fraction_is_refused() {
        for share in [0.0, -0.5, 1.5, f64::NAN, f64::INFINITY] {
            assert_eq!(
                Room::of(Policy { share, margin: 0 }, Some(32_000), 1_000),
                Err(Unsatisfiable::UnusableShare),
                "share {share}"
            );
        }
    }

    #[test]
    fn a_whole_window_share_is_allowed_and_still_leaves_the_reply_out() {
        // `share = 1.0` means the policy does not bind; the reservation still does.
        let room = Room::of(
            Policy {
                share: 1.0,
                margin: 0,
            },
            Some(32_000),
            8_000,
        )
        .expect("usable");
        assert_eq!(room.ceiling, 32_000);
        assert_eq!(room.limit, 24_000);
    }

    #[test]
    fn the_largest_window_does_not_overflow_its_share() {
        let room = Room::of(half(), Some(u32::MAX), 1_000).expect("usable");
        assert_eq!(room.ceiling, u32::MAX / 2);
        assert_eq!(room.limit, u32::MAX / 2);
    }
}

/// What a count of a complete input is measured against.
mod admission {
    use super::*;

    #[test]
    fn a_count_inside_the_limit_is_admitted_with_what_is_left() {
        let room = Room::of(half(), Some(128_000), 8_000).expect("usable");
        assert_eq!(room.admits(0), Admission::Fits { spare: 64_000 });
        assert_eq!(room.admits(64_000), Admission::Fits { spare: 0 });
        assert!(room.admits(63_999).fits());
    }

    #[test]
    fn a_count_over_the_limit_says_by_how_much() {
        let room = Room::of(half(), Some(128_000), 8_000).expect("usable");
        assert_eq!(room.admits(64_001), Admission::Over { by: 1 });
        assert_eq!(room.admits(200_000), Admission::Over { by: 136_000 });
        assert!(!room.admits(64_001).fits());
    }

    #[test]
    fn the_reply_and_the_margin_must_also_fit_inside_the_window() {
        // With a share that does not bind, the limit alone would admit a request that leaves the
        // reply hanging outside the window.
        let room = Room::of(
            Policy {
                share: 1.0,
                margin: 2_000,
            },
            Some(32_000),
            8_000,
        )
        .expect("usable");
        assert_eq!(room.limit, 22_000);
        assert_eq!(room.admits(22_000), Admission::Fits { spare: 0 });
        assert_eq!(
            room.admits(22_001),
            Admission::Over { by: 1 },
            "input + reply + margin is what has to fit"
        );
    }

    #[test]
    fn a_count_that_would_overflow_the_sum_is_over_rather_than_wrapping() {
        let room = Room::of(half(), Some(32_000), 1_000).expect("usable");
        assert!(matches!(room.admits(u32::MAX), Admission::Over { .. }));
    }
}

/// What each side says it can count, and what an unknown answer means.
mod mem_accounting_capability_negotiation {
    use super::*;

    #[test]
    fn a_peer_that_says_nothing_is_taken_as_estimating() {
        // The compatible reading: an older peer named no capability and was estimating already.
        assert_eq!(Counting::read(None), Counting::Estimated);
        assert_eq!(Counting::read(Some("")), Counting::Estimated);
    }

    #[test]
    fn a_capability_this_build_does_not_know_is_not_taken_as_the_stronger_one() {
        for said in ["exact", "provider", "VERIFIED", "verified-v2", "nonsense"] {
            assert_eq!(
                Counting::read(Some(said)),
                Counting::Estimated,
                "`{said}` was read as more than it is"
            );
        }
    }

    #[test]
    fn only_a_verified_count_can_certify_a_ceiling() {
        assert!(Counting::read(Some("verified")).certifiable());
        assert!(!Counting::read(Some("estimated")).certifiable());
        assert!(!Counting::default().certifiable());
    }

    #[test]
    fn what_is_read_is_what_is_written_back() {
        for counting in [Counting::Estimated, Counting::Verified] {
            assert_eq!(Counting::read(Some(counting.as_str())), counting);
        }
    }
}

/// A peer that says nothing about how it counts cannot be read as counting exactly.
mod mem_legacy_sibling_cannot_certify_strict {
    use super::*;

    #[test]
    fn an_older_peer_says_nothing_and_is_read_as_estimating() {
        assert_eq!(Counting::read(None), Counting::Estimated);
        assert!(!Counting::read(None).certifiable());
    }

    #[test]
    fn a_word_this_build_does_not_know_is_not_a_certificate_either() {
        for said in ["exact", "tokenised", "verified-ish", ""] {
            let counting = Counting::read(Some(said));
            assert!(!counting.certifiable(), "{said} certified a strict request");
        }
    }

    #[test]
    fn only_the_word_that_means_it_certifies() {
        assert_eq!(Counting::read(Some("verified")), Counting::Verified);
        assert!(Counting::read(Some("verified")).certifiable());
        assert!(!Counting::read(Some("estimated")).certifiable());
    }

    #[test]
    fn what_is_estimated_still_gets_a_limit_and_a_margin_to_hold_it() {
        // Not certifiable is not unusable: an estimate plans against the same `L`, and `margin`
        // is what a configuration holds back for being wrong about it.
        let room = Room::of(
            Policy {
                share: 0.5,
                margin: 2_000,
            },
            Some(32_000),
            4_000,
        )
        .expect("usable");
        assert_eq!(room.limit, 14_000);
        assert!(!Counting::read(None).certifiable());
    }
}
