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

/// The destination floors are prices now, not walls. A young keeper
/// dropping from a giant (rep 8000) to a tiny club (rep 400) used to
/// fail both the squad-average floor and the reputation-drop floor
/// outright — the case that left U18/U20 keepers stranded. He is
/// discounted for it instead, which is what lets the move happen at all.
#[test]
fn a_raw_youngsters_deep_drop_is_discounted_rather_than_refused() {
    let deep = BorrowerAppetite::of(&LevelFx::reading(0.0, 400.0 / 8000.0, 0.1));
    assert!(
        deep.level_floor > 0.0,
        "the old floors made this exactly zero"
    );
    assert!(deep.score > 0.0);
}

/// …and a near-ready player taking the same drop is discounted harder,
/// because the floors he is held to rise with how ready he already is.
#[test]
fn a_ready_player_pays_more_for_the_same_drop() {
    let raw = BorrowerAppetite::of(&LevelFx::reading(0.0, 0.3, 0.5));
    let ready = BorrowerAppetite::of(&LevelFx::reading(1.0, 0.3, 0.5));
    assert!(ready.level_floor < raw.level_floor);
}

/// A peer-level destination pays nothing at all for the level.
#[test]
fn a_peer_level_destination_is_not_discounted() {
    let peer = BorrowerAppetite::of(&LevelFx::reading(1.0, 0.95, 0.95));
    assert_eq!(peer.level_floor, 1.0);
}

/// Fixtures for the level term: one reading with everything but the two
/// ratios and the readiness held neutral, so each test moves one thing.
struct LevelFx;

impl LevelFx {
    fn reading(readiness: f32, standing_ratio: f32, league_ratio: f32) -> BorrowerReading {
        BorrowerReading {
            base_by_tier: 1.0,
            season_phase: 1.0,
            group: PlayerFieldPositionGroup::Goalkeeper,
            count: 2,
            ideal_depth: 3,
            best_here: 90,
            candidate: 90,
            clearly_better_ahead: 0,
            allowed_ahead: 1,
            band_here: 0.9,
            band_target: 0.9,
            readiness,
            standing_ratio,
            league_ratio,
            need: 0.6,
        }
    }
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
