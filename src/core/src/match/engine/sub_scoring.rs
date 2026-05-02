//! Role- and game-state-aware substitution scoring (Section 6).
//!
//! The classic substitution loop in `substitutions.rs` handles the
//! fatigue / injury / youth-protection passes. This module layers a
//! tactical scoring on top so the coach can also pull "the right type
//! of player off and bring the right type on" — chasing → swap a
//! tired CB for a fresh attacker; protecting a lead → swap a luxury
//! forward for a defender / DM; etc.
//!
//! All scores are unitless [0.0, ~1.5] values; the higher the score
//! the stronger the case for the swap. The substitution loop combines
//! a `sub_off_score` and a `sub_in_score` to choose pairs.

use crate::r#match::{MatchPlayer, engine::coach::TacticalNeed};
use crate::{PlayerFieldPositionGroup, PlayerPositionType};

/// Score a player as a sub-off candidate. Higher = more urgent to
/// remove. Force-selected players are still respected by the loop;
/// this score is purely about tactical / fatigue / risk fit.
pub fn sub_off_score(
    player: &MatchPlayer,
    rating: f32,
    need: TacticalNeed,
    yellow_carded: bool,
) -> f32 {
    let cond_pct = (player.player_attributes.condition as f32 / 10_000.0).clamp(0.0, 1.0);
    let jaded = (player.player_attributes.jadedness as f32 / 10_000.0).clamp(0.0, 1.0);
    let mut s = 0.0;

    // Fatigue dimension.
    s += (1.0 - cond_pct) * 0.32;
    s += jaded * 0.14;

    // Performance dimension — clamp so we can't punish a 6.0 player
    // who simply hasn't done anything.
    let perf = ((6.2 - rating) / 2.0).clamp(0.0, 1.0);
    s += perf * 0.16;

    // Role exhaustion: high-press wingers / fullbacks / CMs at < 60%
    // condition are usually the first to be hooked.
    let pos_group = player.tactical_position.current_position.position_group();
    if cond_pct < 0.60
        && matches!(
            pos_group,
            PlayerFieldPositionGroup::Forward | PlayerFieldPositionGroup::Midfielder
        )
    {
        s += 0.08;
    }
    if cond_pct < 0.55 && pos_group == PlayerFieldPositionGroup::Defender {
        s += 0.05;
    }

    // Yellow-card risk: a yellow + high aggression in a defensive role
    // is a clear "get him off before he sees red" trigger.
    if yellow_carded
        && player.skills.mental.aggression >= 14.0
        && matches!(
            pos_group,
            PlayerFieldPositionGroup::Defender | PlayerFieldPositionGroup::Midfielder
        )
    {
        s += 0.12;
    }

    // Tactical mismatch: chasing → defenders / DMs less needed; defending
    // a lead → luxury forwards less needed.
    s += match (need, pos_group) {
        (TacticalNeed::Chasing, PlayerFieldPositionGroup::Defender) => 0.08,
        (TacticalNeed::ProtectingLead, PlayerFieldPositionGroup::Forward) => 0.08,
        _ => 0.0,
    };

    s
}

/// Score a substitute as a sub-in candidate for the given tactical need.
/// `position_fit` is in [0.0, 1.0] (1.0 = exact position match).
pub fn sub_in_score(
    sub: &MatchPlayer,
    need: TacticalNeed,
    position_fit: f32,
    development_priority: f32,
) -> f32 {
    let ca = (sub.player_attributes.current_ability as f32 / 200.0).clamp(0.0, 1.0);
    let cond = (sub.player_attributes.condition as f32 / 10_000.0).clamp(0.0, 1.0);
    let mut s = 0.30 * position_fit + 0.20 * ca + 0.14 * cond;

    let trait_fit = trait_fit_score(sub, need);
    let need_fit = need_fit_score(sub, need);

    s += 0.20 * need_fit;
    s += 0.10 * trait_fit;
    s += 0.06 * development_priority;
    s
}

fn need_fit_score(sub: &MatchPlayer, need: TacticalNeed) -> f32 {
    let s = &sub.skills;
    let pos_group = sub.tactical_position.current_position.position_group();
    match need {
        TacticalNeed::Chasing => {
            if !matches!(
                pos_group,
                PlayerFieldPositionGroup::Forward | PlayerFieldPositionGroup::Midfielder
            ) {
                return 0.1;
            }
            (s.physical.pace * 0.30
                + s.mental.off_the_ball * 0.25
                + s.technical.finishing * 0.25
                + s.technical.crossing * 0.20)
                / 20.0
        }
        TacticalNeed::ProtectingLead => {
            if !matches!(
                pos_group,
                PlayerFieldPositionGroup::Defender | PlayerFieldPositionGroup::Midfielder
            ) {
                return 0.1;
            }
            (s.mental.positioning * 0.30
                + s.technical.tackling * 0.25
                + s.mental.concentration * 0.25
                + s.mental.work_rate * 0.20)
                / 20.0
        }
        TacticalNeed::LosingMidfield => {
            if pos_group != PlayerFieldPositionGroup::Midfielder {
                return 0.1;
            }
            (s.technical.passing * 0.30
                + s.mental.vision * 0.25
                + s.mental.decisions * 0.25
                + s.mental.composure * 0.20)
                / 20.0
        }
        TacticalNeed::BeingPressed => {
            (s.mental.composure * 0.30
                + s.technical.first_touch * 0.30
                + s.technical.passing * 0.20
                + s.technical.technique * 0.20)
                / 20.0
        }
        TacticalNeed::NeedingCrosses => {
            let wide = matches!(
                sub.tactical_position.current_position,
                PlayerPositionType::WingbackLeft
                    | PlayerPositionType::WingbackRight
                    | PlayerPositionType::MidfielderLeft
                    | PlayerPositionType::MidfielderRight
                    | PlayerPositionType::ForwardLeft
                    | PlayerPositionType::ForwardRight
            );
            if !wide {
                return 0.2;
            }
            (s.technical.crossing * 0.40 + s.physical.pace * 0.30 + s.physical.stamina * 0.30)
                / 20.0
        }
        TacticalNeed::Fatigue => 0.5,
    }
}

fn trait_fit_score(sub: &MatchPlayer, need: TacticalNeed) -> f32 {
    use crate::club::player::traits::PlayerTrait;
    let mut s: f32 = 0.0;
    match need {
        TacticalNeed::Chasing => {
            if sub.has_trait(PlayerTrait::GetsIntoOppositionArea) {
                s += 0.3;
            }
            if sub.has_trait(PlayerTrait::ArrivesLateInOppositionArea) {
                s += 0.2;
            }
            if sub.has_trait(PlayerTrait::RunsWithBallOften) {
                s += 0.2;
            }
            if sub.has_trait(PlayerTrait::PowersShots) || sub.has_trait(PlayerTrait::PlacesShots) {
                s += 0.1;
            }
        }
        TacticalNeed::ProtectingLead => {
            if sub.has_trait(PlayerTrait::StaysBack) {
                s += 0.3;
            }
            if sub.has_trait(PlayerTrait::MarkTightly) {
                s += 0.2;
            }
            if sub.has_trait(PlayerTrait::StaysOnFeet) {
                s += 0.2;
            }
        }
        TacticalNeed::LosingMidfield => {
            if sub.has_trait(PlayerTrait::Playmaker) {
                s += 0.4;
            }
            if sub.has_trait(PlayerTrait::PlaysShortPasses)
                || sub.has_trait(PlayerTrait::TriesThroughBalls)
                || sub.has_trait(PlayerTrait::LikesToSwitchPlay)
            {
                s += 0.2;
            }
        }
        TacticalNeed::BeingPressed => {
            if sub.has_trait(PlayerTrait::Playmaker) {
                s += 0.2;
            }
            if sub.has_trait(PlayerTrait::PlaysShortPasses) {
                s += 0.2;
            }
        }
        TacticalNeed::NeedingCrosses => {
            if sub.has_trait(PlayerTrait::HugsLine) {
                s += 0.4;
            }
            if sub.has_trait(PlayerTrait::CurlsBall) {
                s += 0.2;
            }
        }
        TacticalNeed::Fatigue => {}
    }
    s.clamp(0.0, 1.0)
}

/// Sub-timing windows in minutes — used by callers to gate when each
/// substitution slot may be used. Real coaches stagger their tactical
/// changes around the 60–80 minute window; injuries and red-card
/// fallout are exceptions.
pub fn allowed_in_window(sub_index: u8, match_minute: u32, force_critical: bool) -> bool {
    if force_critical {
        return match_minute >= 5;
    }
    match sub_index {
        0 => match_minute >= 55 && match_minute <= 88,
        1 => match_minute >= 65 && match_minute <= 88,
        2 => match_minute >= 75 && match_minute <= 92,
        _ => match_minute >= 85,
    }
}
