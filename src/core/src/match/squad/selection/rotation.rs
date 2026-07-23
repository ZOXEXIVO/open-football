//! Development / rotation squad selection — the selector behind friendlies
//! and development-league fixtures (the friendly-flagged U18..U23 leagues).
//!
//! Academy football is a minutes economy, not a results contest. A real
//! youth coach plans a season of appearances: every squad member carries a
//! share of the team's matches sized by how much the club believes in him
//! (assessed potential + observable level — never raw PA), final-year
//! players get showcase minutes ahead of the pro-contract decision, and
//! keepers rotate in multi-match blocks rather than alternating weekly or
//! monopolising the gloves. Match stakes still matter — the closer a
//! fixture sits to a must-win, the more selection slides back toward the
//! strongest XI — but a development side never fully abandons the plan.
//!
//! The previous selector keyed rotation on `days_since_last_match` alone.
//! That signal collapses in a weekly league: everyone in the matchday 18
//! resets on the same day (a sub cameo counts), ties fell through to
//! condition and ability, and the same XI — and the same goalkeeper —
//! played every match all season. The minutes ledger below is the fix: it
//! reads actual season appearance counts across every competition bucket,
//! so a player falling behind his planned share builds selection pressure
//! that no tie-break can hide.

use crate::club::staff::perception::{
    AbilityEstimator, CoachProfile, DevelopmentFormEvidence, PotentialEstimator,
};
use crate::club::{PlayerFieldPositionGroup, PlayerPositionType};
use crate::r#match::player::MatchPlayer;
use crate::utils::DateUtils;
use crate::{Player, PlayerStatusType, Tactics, TeamType};
use chrono::NaiveDate;
use std::cmp::Ordering;
use std::collections::HashMap;

use super::helpers;
use super::helpers::KeeperAvailability;

/// Development-fixture selector. Built once per side per matchday by the
/// public rotation API in `mod.rs`; owns the whole XI + bench pick.
pub(crate) struct DevelopmentSelection<'a> {
    pub team_id: u32,
    pub tactics: &'a Tactics,
    pub date: NaiveDate,
    pub team_type: TeamType,
    /// 0.0 = pure development fixture, 1.0 = must-win. Slides weight off
    /// the minutes plan onto observable quality.
    pub stakes: f32,
    /// Matches this team has played this season (busiest-player estimate,
    /// see [`MatchInvolvement::team_matches_estimate`]).
    pub team_matches: f32,
    /// The coach running the development side. His potential judgement
    /// decides how far the minutes plan chases assessed upside over
    /// observable level — a poor judge backs what he can see today.
    pub coach: CoachProfile,
    /// Ids of development guests in the candidate pool — underutilized
    /// visitors from an older, fixture-less club squad. Guests hold no
    /// season share in the minutes plan: they carry a fixed match-practice
    /// pull instead, so they collect occasional runs without displacing
    /// the roster's own development (see `DevelopmentPlan::build` and
    /// `KeeperRotationPlan::pick`).
    pub guest_ids: &'a [u32],
}

impl DevelopmentSelection<'_> {
    fn is_guest(&self, player_id: u32) -> bool {
        self.guest_ids.contains(&player_id)
    }

    /// Below this condition percentage a player is scored negative — the
    /// same protective floor the selector has always used.
    const SOFT_CONDITION_FLOOR: f32 = 20.0;
    /// Positional familiarity stays the dominant structural term: the XI
    /// must still be a coherent team, whoever is owed minutes.
    const FIT_WEIGHT: f32 = 0.55;
    /// Points per match of appearance deficit — the development engine.
    /// Sized so ~2 missed matches overtake an equal-fit incumbent, while
    /// a real positional mismatch needs a season-scale deficit to force.
    const DEFICIT_WEIGHT: f32 = 1.1;
    /// Rest keeps a small nudge for freshness between otherwise-equal
    /// picks; the ledger, not idle days, drives rotation now.
    const REST_WEIGHT: f32 = 0.08;
    const CONDITION_WEIGHT: f32 = 0.12;
    /// Quality floor: merit always counts a little, so training well and
    /// playing well still earn extra minutes at any stakes level.
    const QUALITY_WEIGHT_BASE: f32 = 0.10;
    /// Extra quality weight at full stakes — a youth cup final fields the
    /// strongest XI, like a real academy would.
    const QUALITY_WEIGHT_STAKES: f32 = 0.55;
    /// Bench ordering leans a touch more on rest / quality than the XI
    /// (the bench is where next week's starters recover and cameo).
    const BENCH_REST_WEIGHT: f32 = 0.10;
    const BENCH_QUALITY_BASE: f32 = 0.12;
    const BENCH_QUALITY_STAKES: f32 = 0.50;

    /// Starting XI: planned keeper first, then per-slot picks balancing
    /// positional fit against each candidate's minutes deficit.
    pub(crate) fn select_starting_eleven(&self, available: &[&Player]) -> Vec<MatchPlayer> {
        let mut squad: Vec<MatchPlayer> = Vec::with_capacity(helpers::DEFAULT_SQUAD_SIZE);
        let mut used_ids: Vec<u32> = Vec::new();

        if let Some(gk) = KeeperRotationPlan::pick(self, available, &used_ids) {
            squad.push(MatchPlayer::from_player(
                self.team_id,
                gk,
                PlayerPositionType::Goalkeeper,
                false,
            ));
            used_ids.push(gk.id);
        } else if let Some(any) = helpers::pick_best_unused(available, &used_ids) {
            // No real keeper on the roster at all — same emergency
            // outfielder-in-goal fallback as the competitive path.
            squad.push(MatchPlayer::from_player(
                self.team_id,
                any,
                PlayerPositionType::Goalkeeper,
                false,
            ));
            used_ids.push(any.id);
        }

        let outfield: Vec<&Player> = available
            .iter()
            .filter(|p| !p.positions.is_goalkeeper())
            .copied()
            .collect();
        let plan = DevelopmentPlan::build(self, &outfield, helpers::DEFAULT_SQUAD_SIZE - 1);

        for &pos in self.tactics.positions().iter() {
            if pos == PlayerPositionType::Goalkeeper {
                continue;
            }
            let target_group = pos.position_group();

            let best = outfield
                .iter()
                .filter(|p| !used_ids.contains(&p.id))
                .max_by(|a, b| {
                    let sa = self.slot_score(&plan, a, pos, target_group);
                    let sb = self.slot_score(&plan, b, pos, target_group);
                    sa.partial_cmp(&sb).unwrap_or(Ordering::Equal)
                })
                .copied();

            if let Some(player) = best {
                squad.push(MatchPlayer::from_player(self.team_id, player, pos, false));
                used_ids.push(player.id);
            }
        }

        // Fill any leftover slots with the most underplayed remaining
        // outfielders.
        while squad.len() < helpers::DEFAULT_SQUAD_SIZE {
            let best = outfield
                .iter()
                .filter(|p| !used_ids.contains(&p.id))
                .max_by(|a, b| {
                    let sa = self.overall_score(&plan, a);
                    let sb = self.overall_score(&plan, b);
                    sa.partial_cmp(&sb).unwrap_or(Ordering::Equal)
                })
                .copied();

            match best {
                Some(player) => {
                    let pos = helpers::best_tactical_position(player, self.tactics);
                    squad.push(MatchPlayer::from_player(self.team_id, player, pos, false));
                    used_ids.push(player.id);
                }
                None => break,
            }
        }

        // Last resort — any player, most rested first.
        while squad.len() < helpers::DEFAULT_SQUAD_SIZE {
            let best = available
                .iter()
                .filter(|p| !used_ids.contains(&p.id))
                .max_by(|a, b| {
                    let sa = a.player_attributes.days_since_last_match;
                    let sb = b.player_attributes.days_since_last_match;
                    sa.cmp(&sb)
                })
                .copied();

            match best {
                Some(player) => {
                    let pos = helpers::best_tactical_position(player, self.tactics);
                    squad.push(MatchPlayer::from_player(self.team_id, player, pos, false));
                    used_ids.push(player.id);
                }
                None => break,
            }
        }

        squad
    }

    /// Bench: next keeper by the same rotation plan, then the most
    /// underplayed remaining players — the bench is how fringe squad
    /// members collect cameo minutes between starts.
    pub(crate) fn select_substitutes(&self, remaining: &[&Player]) -> Vec<MatchPlayer> {
        let mut subs: Vec<MatchPlayer> = Vec::with_capacity(helpers::DEFAULT_BENCH_SIZE);
        let mut used_ids: Vec<u32> = Vec::new();

        if let Some(gk) = KeeperRotationPlan::pick(self, remaining, &used_ids) {
            subs.push(MatchPlayer::from_player(
                self.team_id,
                gk,
                PlayerPositionType::Goalkeeper,
                false,
            ));
            used_ids.push(gk.id);
        }

        let plan = DevelopmentPlan::build(self, remaining, helpers::DEFAULT_SQUAD_SIZE - 1);
        while subs.len() < helpers::DEFAULT_BENCH_SIZE {
            let best = remaining
                .iter()
                .filter(|p| !used_ids.contains(&p.id))
                .max_by(|a, b| {
                    let sa = self.bench_score(&plan, a);
                    let sb = self.bench_score(&plan, b);
                    sa.partial_cmp(&sb).unwrap_or(Ordering::Equal)
                })
                .copied();

            match best {
                Some(player) => {
                    let pos = helpers::best_tactical_position(player, self.tactics);
                    subs.push(MatchPlayer::from_player(self.team_id, player, pos, false));
                    used_ids.push(player.id);
                }
                None => break,
            }
        }

        subs
    }

    fn slot_score(
        &self,
        plan: &DevelopmentPlan,
        player: &Player,
        slot_position: PlayerPositionType,
        slot_group: PlayerFieldPositionGroup,
    ) -> f32 {
        let condition_pct = player.player_attributes.condition_percentage() as f32;
        if condition_pct < Self::SOFT_CONDITION_FLOOR {
            let deficit = (Self::SOFT_CONDITION_FLOOR - condition_pct) / Self::SOFT_CONDITION_FLOOR;
            return -(deficit * 30.0);
        }

        let profile = plan.profile(player.id);
        let development = (1.0 - self.stakes).max(0.0);

        let rest = self.rest_points(player);
        // `position_fit_score` returns a positional level — a quality
        // grade. Own players compete on it directly (within one squad it
        // is the real hierarchy); a guest keeps only his familiarity
        // fraction, re-based onto the own squad's mean grade, so a
        // stronger visitor slots in as a positional peer, not a superior.
        let raw_fit = helpers::position_fit_score(player, slot_position, slot_group);
        let fit = if self.is_guest(player.id) {
            (raw_fit / DevelopmentPlan::primary_position_level(player))
                * plan.own_mean_position_level
        } else {
            raw_fit
        };
        let condition_norm = (condition_pct / 100.0).clamp(0.0, 1.0);

        fit * Self::FIT_WEIGHT
            + profile.deficit * Self::DEFICIT_WEIGHT * development
            + rest * Self::REST_WEIGHT
            + condition_norm * 20.0 * Self::CONDITION_WEIGHT
            + profile.quality
                * 20.0
                * (Self::QUALITY_WEIGHT_BASE + Self::QUALITY_WEIGHT_STAKES * self.stakes)
            + profile.form * development
            + RotationWantAway::adjustment(player, self.is_guest(player.id))
    }

    /// Freshness nudge between otherwise-equal picks. A guest's idle weeks
    /// are the reason he is visiting at all, not rest earned between
    /// starts — crediting them would hand every visitor the maximum rest
    /// edge over the roster's weekly-playing own players, so guests score
    /// zero here.
    fn rest_points(&self, player: &Player) -> f32 {
        if self.is_guest(player.id) {
            return 0.0;
        }
        let days = player.player_attributes.days_since_last_match as f32;
        (days / 14.0).min(1.0) * 20.0
    }

    /// Position-blind score for the fill pass — deficit-led, with the
    /// same protective floors as the slot score.
    fn overall_score(&self, plan: &DevelopmentPlan, player: &Player) -> f32 {
        let condition_pct = player.player_attributes.condition_percentage() as f32;
        if condition_pct < Self::SOFT_CONDITION_FLOOR {
            let deficit = (Self::SOFT_CONDITION_FLOOR - condition_pct) / Self::SOFT_CONDITION_FLOOR;
            return -(deficit * 30.0);
        }

        let profile = plan.profile(player.id);
        let development = (1.0 - self.stakes).max(0.0);

        let rest = self.rest_points(player);
        let condition_norm = (condition_pct / 100.0).clamp(0.0, 1.0);

        profile.deficit * Self::DEFICIT_WEIGHT * development
            + rest * Self::REST_WEIGHT
            + condition_norm * 20.0 * Self::CONDITION_WEIGHT
            + profile.quality
                * 20.0
                * (Self::QUALITY_WEIGHT_BASE + Self::QUALITY_WEIGHT_STAKES * self.stakes)
            + profile.form * development
            + RotationWantAway::adjustment(player, self.is_guest(player.id))
    }

    fn bench_score(&self, plan: &DevelopmentPlan, player: &Player) -> f32 {
        let condition_pct = player.player_attributes.condition_percentage() as f32;
        if condition_pct < Self::SOFT_CONDITION_FLOOR {
            let deficit = (Self::SOFT_CONDITION_FLOOR - condition_pct) / Self::SOFT_CONDITION_FLOOR;
            return -(deficit * 30.0);
        }

        let profile = plan.profile(player.id);
        let development = (1.0 - self.stakes).max(0.0);

        let rest = self.rest_points(player);
        let condition_norm = (condition_pct / 100.0).clamp(0.0, 1.0);

        profile.deficit * Self::DEFICIT_WEIGHT * development
            + rest * Self::BENCH_REST_WEIGHT
            + condition_norm * 20.0 * Self::CONDITION_WEIGHT
            + profile.quality
                * 20.0
                * (Self::BENCH_QUALITY_BASE + Self::BENCH_QUALITY_STAKES * self.stakes)
            + profile.form * development
            + RotationWantAway::adjustment(player, self.is_guest(player.id))
    }
}

/// Stakes derivation for the development selector: how far a fixture
/// leans back toward a strongest-XI pick. Development fixtures sit at
/// zero; the slider only engages once match importance crosses into the
/// managed-minutes band, and a friendly is capped — a development coach
/// never fully abandons the plan for a friendly-flagged fixture.
pub(crate) struct DevelopmentStakes;

impl DevelopmentStakes {
    const IMPORTANCE_FLOOR: f32 = 0.45;
    const IMPORTANCE_SPAN: f32 = 0.45;
    const FRIENDLY_CAP: f32 = 0.4;

    pub(crate) fn from_context(match_importance: f32, is_friendly: bool) -> f32 {
        let stakes =
            ((match_importance - Self::IMPORTANCE_FLOOR) / Self::IMPORTANCE_SPAN).clamp(0.0, 1.0);
        if is_friendly {
            stakes.min(Self::FRIENDLY_CAP)
        } else {
            stakes
        }
    }
}

/// Season appearance ledger across every competition bucket a
/// development player features in — his own league (the friendly bucket
/// for youth sides), official league minutes from senior call-ups booked
/// to his spell, and cup runs. A kid already getting senior football
/// needs fewer development starts, exactly like a real academy.
pub(crate) struct MatchInvolvement;

impl MatchInvolvement {
    /// Weight of a substitute cameo relative to a start.
    const SUB_APP_WEIGHT: f32 = 0.5;

    pub(crate) fn starts(player: &Player) -> f32 {
        (player.statistics.played
            + player.cup_statistics.played
            + player.friendly_statistics.played) as f32
    }

    fn sub_apps(player: &Player) -> f32 {
        (player.statistics.played_subs
            + player.cup_statistics.played_subs
            + player.friendly_statistics.played_subs) as f32
    }

    pub(crate) fn involvement(player: &Player) -> f32 {
        Self::starts(player) + Self::SUB_APP_WEIGHT * Self::sub_apps(player)
    }

    /// How many matches this team has played this season: the busiest
    /// player features (start or cameo) in virtually every game, so his
    /// appearance total is a season-length estimate that needs no extra
    /// bookkeeping.
    pub(crate) fn team_matches_estimate(players: &[&Player]) -> f32 {
        players
            .iter()
            .map(|p| Self::starts(p) + Self::sub_apps(p))
            .fold(0.0, f32::max)
    }
}

/// Season minutes plan for a candidate pool: per-player development
/// priority (assessed potential + observable level), the appearance
/// share that priority earns, and the deficit against what the player
/// has actually been given. Built once per selection pass.
struct DevelopmentPlan {
    profiles: HashMap<u32, CandidateProfile>,
    /// Mean primary position level (1-20 grade scale) of the team's own
    /// candidates. `position_fit_score` doubles as a quality grade — it
    /// returns the player's positional level — so a guest's fit is
    /// re-based onto this mean (keeping only his familiarity fraction),
    /// exactly like [`Self::GUEST_QUALITY`] pegs his quality standing.
    own_mean_position_level: f32,
}

#[derive(Clone, Copy)]
struct CandidateProfile {
    /// Matches behind (positive) or ahead of (negative) the season plan.
    deficit: f32,
    /// Observable current level normalised over the pool, 0..1.
    quality: f32,
    /// Merit nudge from the regressed season rating, ± points.
    form: f32,
}

impl CandidateProfile {
    /// Neutral profile for a player outside the pool the plan was built
    /// over (e.g. a keeper reached by the last-resort fill).
    const NEUTRAL: CandidateProfile = CandidateProfile {
        deficit: 0.0,
        quality: 0.5,
        form: 0.0,
    };
}

impl DevelopmentPlan {
    /// Deficit is expressed in matches and capped so a long-absent player
    /// re-enters the XI without erasing positional structure entirely.
    const DEFICIT_CAP: f32 = 6.0;
    /// Priority band: even the squad's least-backed player keeps a real
    /// minutes share (0.6), the most-backed reaches 1.4 before the
    /// final-year showcase bump.
    const PRIORITY_FLOOR: f32 = 0.6;
    const PRIORITY_SPAN: f32 = 0.8;
    /// Bounds on how much of the priority blend chases assessed
    /// potential rather than observable level, scaled by the coach's
    /// potential judgement: a poor judge still knows the obvious
    /// wonderkid (0.30), a sharp one plans minutes around upside (0.65).
    const CEILING_SHARE_FLOOR: f32 = 0.30;
    const CEILING_SHARE_SPAN: f32 = 0.35;
    /// Final-year showcase multiplier — the age-cap season is the club's
    /// last look before the pro-contract / release decision.
    const SHOWCASE_BUMP: f32 = 1.12;
    const FORM_BASELINE: f32 = 6.7;
    const FORM_SCALE: f32 = 0.9;
    const FORM_CAP: f32 = 0.9;
    const FORM_MIN_APPS: u16 = 5;
    /// Standing deficit granted to a development guest — an underutilized
    /// visitor from an older, fixture-less club squad (or a shortfall
    /// fill-in borrowed from another squad). A guest holds no season share
    /// (his appearance ledger belongs to his own squad's economy, and a
    /// rotating supply of zero-involvement visitors would otherwise
    /// accumulate unbounded claims); instead he carries this fixed pull,
    /// sized under one match of own-player deficit so he loses to any own
    /// player on or behind plan and collects starts only from genuine
    /// slack — squads running ahead of plan, or holes the roster can't
    /// fill itself.
    const GUEST_PRACTICE_PULL: f32 = 0.6;
    /// Quality standing of a guest: pegged to the middle of the own-squad
    /// hierarchy. Visitors from an older squad are usually the strongest
    /// players in the pool, and letting them top the quality norm would
    /// have the development side field the visitor for his ability — the
    /// inverse of what the visit is for.
    const GUEST_QUALITY: f32 = 0.5;
    /// Fallback position-level grade when a plan has no own candidates to
    /// average over.
    const NEUTRAL_POSITION_LEVEL: f32 = 12.0;

    fn build(sel: &DevelopmentSelection<'_>, pool: &[&Player], slots: usize) -> Self {
        let mut profiles: HashMap<u32, CandidateProfile> = HashMap::with_capacity(pool.len());
        if pool.is_empty() {
            return DevelopmentPlan {
                profiles,
                own_mean_position_level: Self::NEUTRAL_POSITION_LEVEL,
            };
        }

        // The season plan — shares, quality hierarchy, expected appearances
        // — is drawn over the team's own members only. Guests neither hold
        // a share nor dilute the roster's math; they enter the profiles
        // with fixed pull and mid-pool quality below.
        let own: Vec<&Player> = pool
            .iter()
            .filter(|p| !sel.is_guest(p.id))
            .copied()
            .collect();

        let ceilings: Vec<f32> = own
            .iter()
            .map(|p| PotentialEstimator::observable_ceiling(p, sel.date) as f32)
            .collect();
        let levels: Vec<f32> = own
            .iter()
            .map(|p| AbilityEstimator::observable_level(p) as f32)
            .collect();
        let ceiling_norm = RangeNorm::over(&ceilings);
        let level_norm = RangeNorm::over(&levels);

        let age_cap = sel.team_type.development_age_cap();
        let ceiling_share = Self::CEILING_SHARE_FLOOR
            + Self::CEILING_SHARE_SPAN * sel.coach.potential_accuracy.clamp(0.0, 1.0);
        let weights: Vec<f32> = own
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let mut w = Self::PRIORITY_FLOOR
                    + Self::PRIORITY_SPAN
                        * (ceiling_share * ceiling_norm.of(ceilings[i])
                            + (1.0 - ceiling_share) * level_norm.of(levels[i]));
                if let Some(cap) = age_cap {
                    if DateUtils::age(p.birth_date, sel.date) >= cap {
                        w *= Self::SHOWCASE_BUMP;
                    }
                }
                w
            })
            .collect();
        let mean_weight = if weights.is_empty() {
            1.0
        } else {
            weights.iter().sum::<f32>() / weights.len() as f32
        };

        let slot_share = slots as f32 / own.len().max(1) as f32;
        for (i, p) in own.iter().enumerate() {
            let expected_share =
                (slot_share * weights[i] / mean_weight.max(f32::EPSILON)).min(1.0);
            let deficit = (expected_share * sel.team_matches - MatchInvolvement::involvement(p))
                .clamp(-Self::DEFICIT_CAP, Self::DEFICIT_CAP);
            profiles.insert(
                p.id,
                CandidateProfile {
                    deficit,
                    quality: level_norm.of(levels[i]),
                    form: Self::form_points(p),
                },
            );
        }
        for p in pool.iter().filter(|p| sel.is_guest(p.id)) {
            profiles.insert(
                p.id,
                CandidateProfile {
                    deficit: Self::GUEST_PRACTICE_PULL,
                    quality: Self::GUEST_QUALITY,
                    form: Self::form_points(p),
                },
            );
        }

        let own_mean_position_level = if own.is_empty() {
            Self::NEUTRAL_POSITION_LEVEL
        } else {
            own.iter()
                .map(|p| Self::primary_position_level(p))
                .sum::<f32>()
                / own.len() as f32
        };

        DevelopmentPlan {
            profiles,
            own_mean_position_level,
        }
    }

    fn primary_position_level(player: &Player) -> f32 {
        player
            .positions
            .positions
            .iter()
            .map(|p| p.level)
            .max()
            .unwrap_or(0)
            .max(1) as f32
    }

    fn profile(&self, id: u32) -> CandidateProfile {
        self.profiles
            .get(&id)
            .copied()
            .unwrap_or(CandidateProfile::NEUTRAL)
    }

    /// Merit read from the season bucket that actually carries this
    /// squad's fixtures (see [`DevelopmentFormEvidence`]), regressed for
    /// small samples.
    fn form_points(player: &Player) -> f32 {
        if DevelopmentFormEvidence::games(player) < Self::FORM_MIN_APPS {
            return 0.0;
        }
        let rating = DevelopmentFormEvidence::regressed_rating(player);
        if rating <= 0.0 {
            return 0.0;
        }
        ((rating - Self::FORM_BASELINE) * Self::FORM_SCALE).clamp(-Self::FORM_CAP, Self::FORM_CAP)
    }
}

/// Min-max normaliser over a pool of raw values; collapses to 0.5 when
/// the pool is flat so no term dominates by accident.
struct RangeNorm {
    min: f32,
    span: f32,
}

impl RangeNorm {
    fn over(values: &[f32]) -> RangeNorm {
        let min = values.iter().copied().fold(f32::INFINITY, f32::min);
        let max = values.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        RangeNorm { min, span: max - min }
    }

    fn of(&self, value: f32) -> f32 {
        if self.span <= f32::EPSILON {
            0.5
        } else {
            ((value - self.min) / self.span).clamp(0.0, 1.0)
        }
    }
}

/// Goalkeeper block rotation. Real academies share the gloves in runs of
/// consecutive matches: a keeper needs a run to build rhythm, but the
/// backup needs real starts too — nobody develops on the bench. Every
/// fit keeper gets a target share of starts sized by his assessed
/// potential and level; the plan tracks the actual split and holds the
/// incumbent for a few matches before the benched keeper's deficit flips
/// the gloves — producing multi-match blocks instead of a coin-flip
/// alternation or a season-long monopoly (the old condition-first pick
/// locked one keeper in for the whole season: playing keeps sharpness
/// up, sharpness lifts the recovery target, so the incumbent always
/// arrived on matchday a hair fresher and won the primary sort key).
struct KeeperRotationPlan;

impl KeeperRotationPlan {
    /// Condition at/above which a keeper is a preferred pick; between it
    /// and the hard floor he still beats an emergency outfielder.
    const PREFERRED_CONDITION: u32 = 20;
    /// Days since his last match at/below which a keeper counts as the
    /// current holder of the gloves.
    const INCUMBENT_MAX_IDLE_DAYS: u16 = 9;
    /// Score bonus holding the incumbent in goal — sets the block length
    /// (the benched keeper's deficit needs about this many matches to
    /// flip the gloves).
    const BLOCK_HOLD: f32 = 2.2;
    const DEFICIT_CAP: f32 = 8.0;
    const PRIORITY_FLOOR: f32 = 0.6;
    const PRIORITY_SPAN: f32 = 0.8;
    /// Keeper priority leans harder on the coach's potential read than
    /// the outfield plan — a single-slot position magnifies judgement.
    const CEILING_SHARE_FLOOR: f32 = 0.25;
    const CEILING_SHARE_SPAN: f32 = 0.40;
    const SHOWCASE_BUMP: f32 = 1.10;
    const QUALITY_WEIGHT_BASE: f32 = 0.15;
    const QUALITY_WEIGHT_STAKES: f32 = 0.65;
    /// Condition stays a half-point nudge, never the sort key.
    const CONDITION_NUDGE: f32 = 0.5;
    /// Standing claim of a guest keeper visiting from a fixture-less older
    /// squad. Deliberately under [`Self::BLOCK_HOLD`]: a settled incumbent
    /// on his plan keeps the gloves, and an own keeper behind plan always
    /// reclaims them; the visitor gets his run-out when the gloves are
    /// unsettled — after a fixture break, or at a block handover. Guests
    /// hold no target share of their own — one shirt per match against an
    /// open-ended supply of idle visitors would let the deficit economy
    /// hand the gloves to a fresh guest every week.
    const GUEST_PRACTICE_PULL: f32 = 1.6;
    /// Guest quality is pegged to the middle of the own-keeper hierarchy —
    /// an older visitor is usually the strongest keeper in the pool, and
    /// letting him top the quality norm would hand the gloves to whoever
    /// visits (see `DevelopmentPlan::GUEST_QUALITY`).
    const GUEST_QUALITY: f32 = 0.5;

    fn pick<'p>(
        sel: &DevelopmentSelection<'_>,
        pool: &[&'p Player],
        used_ids: &[u32],
    ) -> Option<&'p Player> {
        let eligible = |min_condition: u32| -> Vec<&'p Player> {
            pool.iter()
                .filter(|p| !used_ids.contains(&p.id))
                // Competitive-strict availability on purpose: a suspended
                // keeper isn't handed a development start — youth-league
                // bans bind inside their own competition.
                .filter(|p| KeeperAvailability::is_fallback_available(p, false))
                .filter(|p| p.player_attributes.condition_percentage() >= min_condition)
                .copied()
                .collect()
        };
        let mut candidates = eligible(Self::PREFERRED_CONDITION);
        if candidates.is_empty() {
            candidates = eligible(helpers::HARD_CONDITION_FLOOR);
        }
        match candidates.len() {
            0 => return None,
            1 => return Some(candidates[0]),
            _ => {}
        }

        // The share plan — priority weights, quality hierarchy, target
        // splits — is drawn over the squad's own keepers only. A guest
        // neither claims a share nor shrinks the residents'; he enters
        // scoring with fixed pull and mid-hierarchy quality.
        let own_idx: Vec<usize> = candidates
            .iter()
            .enumerate()
            .filter(|(_, p)| !sel.is_guest(p.id))
            .map(|(i, _)| i)
            .collect();
        let own_ceilings: Vec<f32> = own_idx
            .iter()
            .map(|&i| PotentialEstimator::observable_ceiling(candidates[i], sel.date) as f32)
            .collect();
        let own_levels: Vec<f32> = own_idx
            .iter()
            .map(|&i| AbilityEstimator::observable_level(candidates[i]) as f32)
            .collect();
        let ceiling_norm = RangeNorm::over(&own_ceilings);
        let level_norm = RangeNorm::over(&own_levels);
        let age_cap = sel.team_type.development_age_cap();
        let ceiling_share = Self::CEILING_SHARE_FLOOR
            + Self::CEILING_SHARE_SPAN * sel.coach.potential_accuracy.clamp(0.0, 1.0);

        let own_weights: Vec<f32> = own_idx
            .iter()
            .enumerate()
            .map(|(k, &i)| {
                let mut w = Self::PRIORITY_FLOOR
                    + Self::PRIORITY_SPAN
                        * (ceiling_share * ceiling_norm.of(own_ceilings[k])
                            + (1.0 - ceiling_share) * level_norm.of(own_levels[k]));
                if let Some(cap) = age_cap {
                    if DateUtils::age(candidates[i].birth_date, sel.date) >= cap {
                        w *= Self::SHOWCASE_BUMP;
                    }
                }
                w
            })
            .collect();
        let weight_sum: f32 = own_weights.iter().sum();

        // Targets are measured against the TEAM's season length, not the
        // own keepers' start sum. The two were identical before guests
        // existed (one own keeper per match), but a guest start freezes
        // the own-start sum — measured against it, the benched own keeper
        // never builds the deficit that evicts the visitor, and a supply
        // of fresh guests can chain the gloves indefinitely. Against the
        // season length, every guest start leaves an own keeper further
        // behind his target, pulling the gloves back to the roster.
        let season_starts = sel.team_matches;

        let mut standing = vec![Self::GUEST_PRACTICE_PULL; candidates.len()];
        let mut quality = vec![Self::GUEST_QUALITY; candidates.len()];
        for (k, &i) in own_idx.iter().enumerate() {
            let target_share = own_weights[k] / weight_sum.max(f32::EPSILON);
            standing[i] = (target_share * season_starts - MatchInvolvement::starts(candidates[i]))
                .clamp(-Self::DEFICIT_CAP, Self::DEFICIT_CAP);
            quality[i] = level_norm.of(own_levels[k]);
        }

        let incumbent_id = candidates
            .iter()
            .min_by_key(|p| p.player_attributes.days_since_last_match)
            .filter(|p| p.player_attributes.days_since_last_match <= Self::INCUMBENT_MAX_IDLE_DAYS)
            .map(|p| p.id);

        let development = (1.0 - sel.stakes).max(0.0);
        let score = |i: usize, p: &Player| -> f32 {
            let hold = if incumbent_id == Some(p.id) {
                Self::BLOCK_HOLD
            } else {
                0.0
            };
            let condition_norm =
                (p.player_attributes.condition_percentage() as f32 / 100.0).clamp(0.0, 1.0);
            (standing[i] + hold) * development
                + quality[i]
                    * 20.0
                    * (Self::QUALITY_WEIGHT_BASE + Self::QUALITY_WEIGHT_STAKES * sel.stakes)
                + condition_norm * Self::CONDITION_NUDGE
        };

        candidates
            .iter()
            .enumerate()
            .max_by(|(ia, a), (ib, b)| {
                score(*ia, a)
                    .partial_cmp(&score(*ib, b))
                    .unwrap_or(Ordering::Equal)
            })
            .map(|(_, p)| *p)
    }
}

/// Want-away nudge for rotation / development selection. Unlike the
/// competitive path it has no disaffection arm — a development fixture is
/// exactly where a listed player should get minutes to stay sharp. It
/// only (a) gives a small keep-sharp pull to a listed / transfer-requested
/// / unhappy player with no imminent move, and (b) protects a
/// near-transfer (`Bid`/`Trn`) player from injury in a meaningless game.
struct RotationWantAway;

impl RotationWantAway {
    /// Small keep-sharp pull for a want-away player getting rotation minutes.
    const KEEP_SHARP: f32 = 2.0;
    /// Protection strong enough to bench a near-sold player from a rotation /
    /// development XI whenever there is anyone else to field.
    const PROTECT_NEAR_TRANSFER: f32 = -12.0;

    fn adjustment(player: &Player, is_guest: bool) -> f32 {
        if player.statuses.has(PlayerStatusType::Trn) || player.statuses.has(PlayerStatusType::Bid)
        {
            return Self::PROTECT_NEAR_TRANSFER;
        }
        // A guest's entire standing is his fixed practice pull — the
        // fixture-less squads that send guests are exactly where listed
        // players sit, so stacking keep-sharp on top would hand nearly
        // every visitor a second, larger pull and crowd out the team's
        // own development. Injury protection above still applies.
        if is_guest {
            return 0.0;
        }
        let want_away = player.statuses.has(PlayerStatusType::Lst)
            || player.statuses.has(PlayerStatusType::Req)
            || player.statuses.has(PlayerStatusType::Unh);
        if want_away { Self::KEEP_SHARP } else { 0.0 }
    }
}
