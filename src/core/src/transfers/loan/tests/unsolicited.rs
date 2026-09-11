//! Moved verbatim out of `loan_market.rs` — see that file's `mod unsolicited_loan_target_tests`.

use super::super::*;
use crate::club::player::builder::PlayerBuilder;
use crate::shared::fullname::FullName;
use crate::transfers::loan::LoanPipeline;
use crate::{
    PersonAttributes, Player, PlayerAttributes, PlayerClubContract, PlayerPosition,
    PlayerPositionType, PlayerPositions, PlayerSkills,
};
use chrono::NaiveDate;

/// Fixtures for the unsolicited-target eligibility policy. Wrapped in a
/// unit struct per the project's no-free-helpers convention.
struct Fx;

impl Fx {
    fn date() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 7, 6).unwrap()
    }

    /// Destination level with no league context on either side, so the
    /// division gate stands down and the club-standing floor is what the
    /// assertion is measuring.
    fn level(
        ability: u8,
        parent_best_in_group: u8,
        parent_rep: u16,
        borrower_rep: u16,
        is_development: bool,
    ) -> LoanDestinationLevel {
        LoanDestinationLevel {
            ability,
            parent_best_in_group,
            parent_rep,
            borrower_rep,
            parent_league_rep: 0,
            borrower_league_rep: 0,
            is_development,
        }
    }

    /// A contracted central midfielder. `with_contract = false` leaves
    /// him contract-less (a returning loanee / free agent on the books).
    fn player(with_contract: bool) -> Player {
        let mut attrs = PlayerAttributes::default();
        attrs.current_ability = 95;
        attrs.potential_ability = 150;
        let mut builder = PlayerBuilder::new()
            .id(1)
            .full_name(FullName::new("Y".into(), "P".into()))
            .birth_date(NaiveDate::from_ymd_opt(2007, 1, 1).unwrap())
            .country_id(1)
            .attributes(PersonAttributes::default())
            .skills(PlayerSkills::default())
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position: PlayerPositionType::MidfielderCenter,
                    level: 16,
                }],
            })
            .player_attributes(attrs);
        if with_contract {
            builder = builder.contract(Some(PlayerClubContract::new(
                20_000,
                NaiveDate::from_ymd_opt(2030, 6, 30).unwrap(),
            )));
        }
        builder.build().unwrap()
    }

    const MAX: u8 = MAX_LOAN_TARGET_AGE;
}

#[test]
fn young_unlisted_prospect_is_a_development_target() {
    let p = Fx::player(true);
    assert_eq!(
        UnsolicitedLoanTarget::classify(
            &p,
            18,
            Fx::MAX,
            SquadAssetClass::ProspectDevelopment,
            false
        ),
        Some(true),
        "an unlisted young prospect is approachable as a development loan"
    );
}

#[test]
fn first_team_contributors_are_never_cold_approached() {
    let p = Fx::player(true);
    for class in [
        SquadAssetClass::CorePlayer,
        SquadAssetClass::FirstTeamUseful,
    ] {
        assert_eq!(
            UnsolicitedLoanTarget::classify(&p, 18, Fx::MAX, class, false),
            None,
            "a first-team contributor must never be cold-approached"
        );
    }
}

/// …and the same protection read off STANDING rather than off a
/// label. A nineteen-year-old first choice carries
/// `ProspectDevelopment` because the class follows a squad status he
/// gets from his birth year, and that walked straight through the arm
/// above.
#[test]
fn the_parents_own_first_choice_is_never_cold_approached_either() {
    let p = Fx::player(true);
    assert_eq!(
        UnsolicitedLoanTarget::classify(
            &p,
            19,
            Fx::MAX,
            SquadAssetClass::ProspectDevelopment,
            true,
        ),
        None,
        "a club does not entertain a cold call about the man who starts for it"
    );
    assert_eq!(
        UnsolicitedLoanTarget::classify(
            &p,
            19,
            Fx::MAX,
            SquadAssetClass::ProspectDevelopment,
            false,
        ),
        Some(true),
        "a genuine prospect is still a development target"
    );
}

#[test]
fn unevaluated_player_is_not_a_target() {
    let p = Fx::player(true);
    assert_eq!(
        UnsolicitedLoanTarget::classify(
            &p,
            18,
            Fx::MAX,
            SquadAssetClass::UnknownNeedsEvaluation,
            false
        ),
        None,
        "a player the club hasn't evaluated yet is left alone"
    );
}

#[test]
fn older_surplus_is_a_generic_cover_target() {
    let p = Fx::player(true);
    assert_eq!(
        UnsolicitedLoanTarget::classify(&p, 30, Fx::MAX, SquadAssetClass::TrueSurplus, false),
        Some(false),
        "older genuine surplus is loanable, but as generic cover (not development)"
    );
}

#[test]
fn young_rotation_develops_but_older_rotation_does_not() {
    let p = Fx::player(true);
    assert_eq!(
        UnsolicitedLoanTarget::classify(&p, 20, Fx::MAX, SquadAssetClass::RotationUseful, false),
        Some(true),
        "a young rotation player can go on a development loan"
    );
    assert_eq!(
        UnsolicitedLoanTarget::classify(&p, 30, Fx::MAX, SquadAssetClass::RotationUseful, false),
        None,
        "an older rotation player is squad depth, not a cold loan target"
    );
}

#[test]
fn over_age_cap_is_not_a_target() {
    let p = Fx::player(true);
    assert_eq!(
        UnsolicitedLoanTarget::classify(
            &p,
            Fx::MAX + 1,
            Fx::MAX,
            SquadAssetClass::TrueSurplus,
            false
        ),
        None,
        "past the loan age cap nobody is a target"
    );
}

#[test]
fn already_listed_or_pinned_players_use_other_paths() {
    let mut listed = Fx::player(true);
    listed.statuses.add(Fx::date(), PlayerStatusType::Loa);
    assert_eq!(
        UnsolicitedLoanTarget::classify(
            &listed,
            18,
            Fx::MAX,
            SquadAssetClass::ProspectDevelopment,
            false
        ),
        None,
        "a loan-listed player flows through the normal listed path"
    );

    let mut pinned = Fx::player(true);
    pinned.is_force_match_selection = true;
    assert_eq!(
        UnsolicitedLoanTarget::classify(
            &pinned,
            18,
            Fx::MAX,
            SquadAssetClass::ProspectDevelopment,
            false
        ),
        None,
        "a manager-pinned player is never cold-approached"
    );
}

#[test]
fn contract_less_player_is_not_a_target() {
    let p = Fx::player(false);
    assert_eq!(
        UnsolicitedLoanTarget::classify(
            &p,
            18,
            Fx::MAX,
            SquadAssetClass::ProspectDevelopment,
            false
        ),
        None,
        "a contract-less player (free agent / returning loanee) is not loaned out"
    );
}

#[test]
fn foreign_target_must_sit_clearly_below_the_clubs_best() {
    // Young prospect: a small gap below the club's best is enough.
    assert!(ForeignUnsolicitedLoanTarget::looks_loanable(18, 90, 100));
    assert!(!ForeignUnsolicitedLoanTarget::looks_loanable(18, 98, 100));
    // Older player: needs a clear surplus gap to read as fringe.
    assert!(!ForeignUnsolicitedLoanTarget::looks_loanable(30, 90, 100));
    assert!(ForeignUnsolicitedLoanTarget::looks_loanable(30, 80, 100));
}

#[test]
fn foreign_development_band_tracks_age() {
    assert!(ForeignUnsolicitedLoanTarget::is_development(22));
    assert!(ForeignUnsolicitedLoanTarget::is_development(23));
    assert!(!ForeignUnsolicitedLoanTarget::is_development(24));
}

#[test]
fn development_loan_bypasses_level_floors() {
    // A young keeper (low CA) dropping from a giant parent (rep 8000,
    // best keeper 145) to a tiny club (avg 90, rep 400) would fail both
    // the squad-average floor and the reputation-drop floor — but as a
    // development loan he clears the level gate, because the caller's
    // minutes gate is the real "will he play here" check. This is the
    // case that left U18/U20 keepers stranded.
    assert!(UnsolicitedLoanTarget::clears_level_gate(
        90,
        &Fx::level(60, 145, 8000, 400, true)
    ));
}

#[test]
fn cover_loan_keeps_level_floors() {
    // Non-development cover: both floors still apply.
    // Far below the borrower's squad average → blocked by the floor.
    assert!(!UnsolicitedLoanTarget::clears_level_gate(
        90,
        &Fx::level(60, 145, 8000, 3000, false)
    ));
    // Near the borrower's level AND a plausible (raw-player) rep drop
    // from a giant → allowed.
    assert!(UnsolicitedLoanTarget::clears_level_gate(
        90,
        &Fx::level(86, 130, 8000, 3000, false)
    ));
    // Near level, but a non-raw player dropping from a giant to a
    // minnow is implausible → blocked by the reputation gate.
    assert!(!UnsolicitedLoanTarget::clears_level_gate(
        118,
        &Fx::level(120, 125, 8000, 500, false)
    ));
}

/// B4 — the compatriot sweep saw nobody over 23.
///
/// The proactive foreign pickup is the ONLY branch an Elite or
/// Continental club runs on the compatriot slice, and it gated on the
/// cold-approach development age. `UnsettledAbroadScan` posts men up
/// to 25 and the manager-talk loan route reads the same 25, so a
/// posted 24- or 25-year-old compatriot passed every pool filter and
/// was then invisible to every big club in his own country —
/// reachable only by a National side with an open request in his
/// exact position.
///
/// Two bands, because they answer different questions: taking a
/// stranger's prospect off him is a development decision; taking back
/// one of your own league's exports, whose club has said he can go,
/// is a homecoming.
#[test]
fn a_posted_compatriot_is_taken_at_the_age_his_posting_used() {
    const BRAZIL: u32 = 55;
    const ENGLAND: u32 = 1;

    // A posted 24-year-old Brazilian, seen by a Brazilian club.
    assert!(LoanPipeline::home_pickup_age_ok(24, true, BRAZIL, BRAZIL));
    // …and 25, the oldest the posting model itself will name.
    assert!(LoanPipeline::home_pickup_age_ok(
        UnsettledAbroadScan::MAX_AGE,
        true,
        BRAZIL,
        BRAZIL
    ));
    // Past that the parent's answer is the market, not a loan.
    assert!(!LoanPipeline::home_pickup_age_ok(
        UnsettledAbroadScan::MAX_AGE + 1,
        true,
        BRAZIL,
        BRAZIL
    ));

    // The same 24-year-old at an ENGLISH club's door is an ordinary
    // cold development pickup, and keeps the tighter band.
    assert!(!LoanPipeline::home_pickup_age_ok(24, true, BRAZIL, ENGLAND));
    // …as does a Brazilian nobody has posted.
    assert!(!LoanPipeline::home_pickup_age_ok(24, false, BRAZIL, BRAZIL));
    // Development-age men are reachable either way.
    assert!(LoanPipeline::home_pickup_age_ok(
        UnsettledAbroadScan::DEVELOPMENT_AGE,
        false,
        BRAZIL,
        ENGLAND
    ));
}
