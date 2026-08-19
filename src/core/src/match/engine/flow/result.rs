use crate::league::LeagueMatch;
use crate::r#match::engine::zones::ZoneStats;
use crate::r#match::player::statistics::MatchStatisticType;
use crate::r#match::result::{GOAL_CLIP_POST_ROLL_MS, GOAL_CLIP_PRE_ROLL_MS};
use crate::r#match::squad::OmittedPlayer;
use crate::r#match::{MatchSquad, ResultMatchPositionData};
use crate::{MatchTacticType, PlayerFieldPositionGroup, PlayerPositionType};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubstitutionInfo {
    pub team_id: u32,
    pub player_out_id: u32,
    pub player_in_id: u32,
    pub match_time_ms: u64,
    /// Why the substitution fired. Defaulted to `Discretionary` for
    /// backward compatibility with callers that don't yet tag the
    /// pass that produced the sub. Drives the morale-side
    /// `SubstitutionFrustration` gate: critical injury / youth
    /// protection / fatigue rotation never qualify as frustration.
    #[serde(default)]
    pub reason: SubstitutionReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SubstitutionReason {
    /// Forced injury swap (in-match condition collapse / rolled injury).
    /// Never a frustration trigger.
    CriticalInjury,
    /// Under-17 condition / jadedness guard. Protective, not punitive.
    YouthProtection,
    /// Discretionary scored-pair swap (tactical / fatigue / development).
    /// The frustration-event detector inspects rating + minute to decide
    /// whether the player was hooked while playing well or pulled early
    /// in a big match — vs a routine 75th-minute fatigue swap that the
    /// player shrugs off.
    #[default]
    Discretionary,
    /// The side lost its goalkeeper (red card, typically) and burned a
    /// substitution to bring the bench keeper on for an outfielder.
    /// The sacrificed player is a victim of circumstance, not coach
    /// doubt — never a frustration trigger.
    GoalkeeperEmergency,
}

/// Final physical state of a player at the moment they left the pitch
/// (substitution time or full time). The match engine drains the
/// `MatchPlayer` copy's condition tick-by-tick during the sim; without
/// this snapshot the persisted `Player` only sees minute counts and
/// load — never the actual end-of-match energy. Post-match exertion
/// feeds `final_match_energy` into a duration-scaled depletion model
/// so the persisted condition reflects how empty the tank really got.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct PlayerMatchPhysicalSnapshot {
    pub player_id: u32,
    /// Minutes spent on the pitch. Computed from `entry_match_time_ms`
    /// to the exit time (substitution) or current time (full time).
    pub minutes_played: f32,
    /// Condition (0..10000) at kickoff or at the moment this player
    /// entered the pitch as a substitute.
    pub starting_condition: i16,
    /// Condition (0..10000) at the moment this player left the pitch
    /// or at full time. Engine floor is 1500 (15%) — values this low
    /// indicate the player was running on fumes, which the post-match
    /// formula uses to amplify the persisted condition drop.
    pub final_match_energy: i16,
    /// Engine-side hint for high-intensity work done by this player —
    /// sprint distance, pressure bursts, etc. Currently seeded from
    /// the position-group default share (0.05..0.32) so the model has
    /// a starting value before the engine grows per-player tracking.
    pub high_intensity_load_hint: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerMatchEndStats {
    pub shots_on_target: u16,
    pub shots_total: u16,
    pub passes_attempted: u16,
    pub passes_completed: u16,
    pub tackles: u16,
    pub interceptions: u16,
    pub saves: u16,
    /// Shots-on-target the player (typically a GK) had to deal with —
    /// `saves` + goals conceded. Drives the save-percentage component
    /// of the rating helper.
    pub shots_faced: u16,
    pub goals: u16,
    pub assists: u16,
    /// Public/effective match rating. This is the value every downstream
    /// reader (match page DTO, awards, scouting showcase, league stat
    /// rebuild, season averages) consumes. The engine writes the pure
    /// stat-line value here at first; the league pipeline then overwrites
    /// it with the settlement-adjusted public rating in
    /// `process_match_events` so there's a single canonical field across
    /// the codebase. The original engine rating is preserved on
    /// `raw_match_rating` for diagnostics / calibration scripts.
    pub match_rating: f32,
    /// Pure stat-line rating produced by `RatingContext::calculate()`,
    /// frozen before settlement / chemistry / personality adjustments
    /// rewrite `match_rating`. Kept reachable for calibration tests and
    /// debug surfaces that need the unfiltered engine verdict. Defaults
    /// to `0.0` when deserialised from a save written before this field
    /// existed — readers should treat `0.0` as "raw rating not recorded
    /// separately" and fall back to `match_rating`.
    #[serde(default)]
    pub raw_match_rating: f32,
    /// Sum of expected goals from this player's shots in this match.
    pub xg: f32,
    /// Player's position group for position-aware rating calculation.
    pub position_group: PlayerFieldPositionGroup,
    /// Fouls committed by the player in this match.
    pub fouls: u16,
    /// Yellow cards received (0, 1, or 2).
    pub yellow_cards: u16,
    /// 1 if the player was sent off (either two yellows or direct red).
    pub red_cards: u16,
    /// Match minutes played. Used by the rating helper to dampen event
    /// bonuses for short cameos.
    pub minutes_played: u16,
    /// Modern build-up / chance-creation stats — feed the rating helper
    /// and end-of-match calibration. All zero for legacy callers.
    pub key_passes: u16,
    pub progressive_passes: u16,
    pub progressive_carries: u16,
    pub successful_dribbles: u16,
    pub attempted_dribbles: u16,
    pub successful_pressures: u16,
    /// Total close-range pressures applied — superset of
    /// `successful_pressures`. Used for a small "pressing volume" credit
    /// that's worth less per event than a successful pressure.
    pub pressures: u16,
    pub blocks: u16,
    pub clearances: u16,
    /// Completed passes finishing inside the opposition penalty area —
    /// chance-creation indicator independent of the eventual shot.
    pub passes_into_box: u16,
    pub crosses_attempted: u16,
    pub crosses_completed: u16,
    /// xG of all shots in possessions this player participated in. Used
    /// for build-up credit (small) without double-counting goals.
    pub xg_chain: f32,
    /// xG of build-up chains excluding the player's own shots / assists.
    /// Pure "made the chance happen" signal.
    pub xg_buildup: f32,
    /// First-touch resolutions that fluffed the ball.
    pub miscontrols: u16,
    /// First-touch resolutions in the heavy-touch band — kept the ball
    /// alive but gave it away in tempo.
    pub heavy_touches: u16,
    /// Cumulative pitch-units carried under control. Tie-breaker only.
    pub carry_distance: u32,
    pub errors_leading_to_shot: u16,
    pub errors_leading_to_goal: u16,
    /// (GK) Post-shot xG faced minus goals conceded. Positive values
    /// indicate above-expectation shot-stopping.
    pub xg_prevented: f32,
    /// (GK) Chance value of every shot on target the keeper had to deal
    /// with, saved or conceded — the expectation term of the rating's
    /// goals-prevented model. `0.0` on stat lines written before the
    /// counter existed (and on hand-built fixtures), in which case the
    /// rating falls back to a flat per-shot conversion baseline.
    #[serde(default)]
    pub xg_faced: f32,
    /// Offside calls against this player.
    pub offsides: u16,
    /// Auto-goals scored by this player. Treated as a -1.0 base
    /// penalty in the rating with an extra -0.30 because the OG sits
    /// inside the player's own goal mouth.
    pub own_goals: u16,
    /// Per-zone counters mirrored from `MatchPlayerStatistics`. Used
    /// by the rating helper to apply zone-aware multipliers without
    /// re-deriving the action stream.
    pub zone_stats: ZoneStats,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PenaltyShootoutKick {
    pub team_id: u32,
    pub taker_id: u32,
    pub goalkeeper_id: Option<u32>,
    pub round: u8,
    pub scored: bool,
    pub sudden_death: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MatchResultRaw {
    pub score: Option<Score>,

    /// Position-replay payload. NEVER serialised over the worker wire
    /// — the bincode payload would balloon to many MB per match and the
    /// recorder is only enabled for the local viewer anyway. On the
    /// receive side an empty `ResultMatchPositionData::empty()` is
    /// substituted in.
    #[serde(skip, default = "ResultMatchPositionData::empty")]
    pub position_data: ResultMatchPositionData,

    pub left_team_players: FieldSquad,
    pub right_team_players: FieldSquad,

    pub match_time_ms: u64,
    pub additional_time_ms: u64,

    pub player_stats: HashMap<u32, PlayerMatchEndStats>,

    pub substitutions: Vec<SubstitutionInfo>,

    /// The near misses worth watching again — [`HighlightSelector::PER_TEAM`]
    /// per side at most, already shortlisted by the time this is built. The
    /// recording carries a clip around each one, so the two lists are written
    /// together in `build_result` and must never be filtered apart afterwards:
    /// a marker with no footage under it is a seek that lands in a grey zone.
    ///
    /// Defaulted for results written before chances were kept at all.
    #[serde(default)]
    pub chances: Vec<ChanceDetail>,

    /// Final physical snapshot per player who appeared in this match.
    /// Populated for every player on the pitch at full time AND every
    /// player who was substituted off (snapshot taken at the swap).
    /// Consumed by `LeagueResult::apply_post_match_physical_effects`
    /// to feed `final_match_energy` into the new condition-drop
    /// formula. Missing entries fall back to the legacy minutes-only
    /// path so callers that don't construct `MatchResultRaw` via the
    /// engine continue to work.
    pub physical_snapshots: HashMap<u32, PlayerMatchPhysicalSnapshot>,

    pub penalty_shootout: Vec<PenaltyShootoutKick>,

    pub player_of_the_match_id: Option<u32>,

    /// The shape each team STARTED the match in (taken from
    /// `MatchSquad::tactics` at kickoff). Lets the result and the
    /// downstream `MatchHistory` show a "starting → final" tactical
    /// summary instead of just "what shape was on the pitch at the
    /// final whistle".
    pub starting_home_tactic: Option<MatchTacticType>,
    pub starting_away_tactic: Option<MatchTacticType>,
    /// The shape each team finished the match in. Differs from the
    /// starting shape when the in-match coach probed a situational
    /// override (`evaluate_situational_shape`) — e.g. 4-4-2 → 4-3-3
    /// chasing a deficit, 4-4-2 → 4-5-1 protecting a lead. Surfaced so
    /// the league pipeline can record it on `MatchHistory` and the
    /// web tactics view can render "Plan: 4-4-2 — Last used: 4-3-3
    /// (chase) vs Spurs".
    pub final_home_tactic: Option<MatchTacticType>,
    pub final_away_tactic: Option<MatchTacticType>,
    /// Sim-minute at which the FIRST shape change fired for either
    /// side (whichever came first). `None` when neither side changed
    /// shape during the match. Stored as the marker the web view uses
    /// to label a chip with "shifted at min X".
    pub shape_change_minute: Option<u8>,
}

impl Clone for MatchResultRaw {
    fn clone(&self) -> Self {
        MatchResultRaw {
            score: self.score.clone(),
            position_data: self.position_data.clone(),
            left_team_players: self.left_team_players.clone(),
            right_team_players: self.right_team_players.clone(),
            match_time_ms: self.match_time_ms,
            additional_time_ms: self.additional_time_ms,
            player_stats: self.player_stats.clone(),
            substitutions: self.substitutions.clone(),
            chances: self.chances.clone(),
            physical_snapshots: self.physical_snapshots.clone(),
            penalty_shootout: self.penalty_shootout.clone(),
            player_of_the_match_id: self.player_of_the_match_id,
            starting_home_tactic: self.starting_home_tactic,
            starting_away_tactic: self.starting_away_tactic,
            final_home_tactic: self.final_home_tactic,
            final_away_tactic: self.final_away_tactic,
            shape_change_minute: self.shape_change_minute,
        }
    }
}

impl MatchResultRaw {
    pub fn with_match_time(match_time_ms: u64) -> Self {
        MatchResultRaw {
            score: None,
            position_data: ResultMatchPositionData::new(),
            left_team_players: FieldSquad::new(),
            right_team_players: FieldSquad::new(),
            match_time_ms,
            additional_time_ms: 0,
            player_stats: HashMap::new(),
            substitutions: Vec::new(),
            chances: Vec::new(),
            physical_snapshots: HashMap::new(),
            penalty_shootout: Vec::new(),
            player_of_the_match_id: None,
            starting_home_tactic: None,
            starting_away_tactic: None,
            final_home_tactic: None,
            final_away_tactic: None,
            shape_change_minute: None,
        }
    }

    pub fn copy_without_data_positions(&self) -> Self {
        MatchResultRaw {
            score: self.score.clone(),
            position_data: ResultMatchPositionData::new(),
            left_team_players: self.left_team_players.clone(),
            right_team_players: self.right_team_players.clone(),
            match_time_ms: self.match_time_ms,
            additional_time_ms: self.additional_time_ms,
            player_stats: self.player_stats.clone(),
            substitutions: self.substitutions.clone(),
            chances: self.chances.clone(),
            physical_snapshots: self.physical_snapshots.clone(),
            penalty_shootout: self.penalty_shootout.clone(),
            player_of_the_match_id: self.player_of_the_match_id,
            starting_home_tactic: self.starting_home_tactic,
            starting_away_tactic: self.starting_away_tactic,
            final_home_tactic: self.final_home_tactic,
            final_away_tactic: self.final_away_tactic,
            shape_change_minute: self.shape_change_minute,
        }
    }

    pub fn write_team_players(
        &mut self,
        home_team_players: &FieldSquad,
        away_team_players: &FieldSquad,
    ) {
        self.left_team_players = home_team_players.clone();
        self.right_team_players = away_team_players.clone();
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldSquad {
    pub team_id: u32,
    pub main: Vec<u32>,
    pub substitutes: Vec<u32>,
    pub substitutes_used: Vec<u32>,
    /// Important omissions surfaced at squad-selection time, threaded
    /// through the match engine so the post-match dispatcher can call
    /// `Player::on_match_dropped_with_context` with the right
    /// explanation. Empty when nothing notable happened.
    pub selection_omissions: Vec<OmittedPlayer>,
    /// Slot each starter was assigned at kickoff. Drives the post-
    /// match coach-memory observation's `role_fit` reading so a
    /// player pressed into an emergency role (e.g. a midfielder
    /// starting at fullback) registers as an out-of-position
    /// appearance instead of a natural-slot start. Empty for
    /// FieldSquads built outside the squad selector flow (tests,
    /// dev_match harness) — the dispatcher falls back to assuming
    /// a natural-role start in that case.
    #[serde(default)]
    pub starter_slots: Vec<(u32, PlayerPositionType)>,
}

impl FieldSquad {
    pub fn new() -> Self {
        FieldSquad {
            team_id: 0,
            main: Vec::new(),
            substitutes: Vec::new(),
            substitutes_used: Vec::new(),
            selection_omissions: Vec::new(),
            starter_slots: Vec::new(),
        }
    }

    pub fn from_team(squad: &MatchSquad) -> Self {
        FieldSquad {
            team_id: squad.team_id,
            main: squad.main_squad.iter().map(|p| p.id).collect(),
            substitutes: squad.substitutes.iter().map(|p| p.id).collect(),
            substitutes_used: Vec::new(),
            selection_omissions: squad.selection_omissions.clone(),
            starter_slots: squad
                .main_squad
                .iter()
                .map(|p| (p.id, p.tactical_position.current_position))
                .collect(),
        }
    }

    pub fn mark_substitute_used(&mut self, player_id: u32) {
        if self.substitutes.contains(&player_id) && !self.substitutes_used.contains(&player_id) {
            self.substitutes_used.push(player_id);
        }
    }

    pub fn count(&self) -> usize {
        self.main.len() + self.substitutes.len()
    }

    /// Slot the starter played at kickoff, or `None` if the
    /// `starter_slots` map was not populated (legacy paths) or the
    /// player wasn't in the starting eleven.
    pub fn starter_slot(&self, player_id: u32) -> Option<PlayerPositionType> {
        self.starter_slots
            .iter()
            .find(|(id, _)| *id == player_id)
            .map(|(_, slot)| *slot)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Score {
    pub home_team: TeamScore,
    pub away_team: TeamScore,

    pub details: Vec<GoalDetail>,

    /// Penalty-shootout tally (0 if no shootout took place).
    pub home_shootout: u8,
    pub away_shootout: u8,
}

/// Outcome of a match from the home team's perspective, considering
/// both the regulation (+ extra time) score and the shootout tally.
/// Distinct from `club::MatchOutcome`, which is team-relative (Win/Draw/Loss).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchResultOutcome {
    HomeWin,
    AwayWin,
    Draw,
}

impl Score {
    /// True if the regulation + extra-time score is level.
    pub fn is_tied(&self) -> bool {
        self.home_team.get() == self.away_team.get()
    }

    /// Did a shootout take place? (Either side has shootout goals recorded.)
    pub fn had_shootout(&self) -> bool {
        self.home_shootout > 0 || self.away_shootout > 0
    }

    /// True outcome — accounts for penalty shootout tiebreak in knockouts.
    /// Regulation-only consumers (league table, points) should use
    /// `home_team.get()` / `away_team.get()` directly; those stay as-is.
    pub fn outcome(&self) -> MatchResultOutcome {
        let h = self.home_team.get();
        let a = self.away_team.get();
        if h > a {
            return MatchResultOutcome::HomeWin;
        }
        if a > h {
            return MatchResultOutcome::AwayWin;
        }
        // Regulation tied — use shootout if any kick was taken.
        if self.had_shootout() {
            if self.home_shootout > self.away_shootout {
                MatchResultOutcome::HomeWin
            } else if self.away_shootout > self.home_shootout {
                MatchResultOutcome::AwayWin
            } else {
                // Shootout technically can't end tied, but belt-and-braces.
                MatchResultOutcome::Draw
            }
        } else {
            MatchResultOutcome::Draw
        }
    }
}

#[derive(Debug)]
pub struct TeamScore {
    pub team_id: u32,
    score: AtomicU8,
}

impl Clone for TeamScore {
    fn clone(&self) -> Self {
        TeamScore {
            team_id: self.team_id,
            score: AtomicU8::new(self.score.load(Ordering::Relaxed)),
        }
    }
}

/// Custom (de)serialization because `AtomicU8` is not serde-friendly.
/// On the wire `TeamScore` is just `(team_id, score)`; the deserialised
/// value carries the snapshot in a fresh atomic.
impl serde::Serialize for TeamScore {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut s = serializer.serialize_struct("TeamScore", 2)?;
        s.serialize_field("team_id", &self.team_id)?;
        s.serialize_field("score", &self.score.load(Ordering::Relaxed))?;
        s.end()
    }
}

impl<'de> serde::Deserialize<'de> for TeamScore {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(serde::Deserialize)]
        struct Wire {
            team_id: u32,
            score: u8,
        }
        let w = Wire::deserialize(deserializer)?;
        Ok(TeamScore {
            team_id: w.team_id,
            score: AtomicU8::new(w.score),
        })
    }
}

impl TeamScore {
    pub fn new(team_id: u32) -> Self {
        TeamScore {
            team_id,
            score: AtomicU8::new(0),
        }
    }

    pub fn new_with_score(team_id: u32, score: u8) -> Self {
        TeamScore {
            team_id,
            score: AtomicU8::new(score),
        }
    }

    pub fn get(&self) -> u8 {
        self.score.load(Ordering::Relaxed)
    }
}
impl From<&TeamScore> for TeamScore {
    fn from(team_score: &TeamScore) -> Self {
        TeamScore::new_with_score(team_score.team_id, team_score.score.load(Ordering::Relaxed))
    }
}

impl PartialEq<Self> for TeamScore {
    fn eq(&self, other: &Self) -> bool {
        self.score.load(Ordering::Relaxed) == other.score.load(Ordering::Relaxed)
    }
}

impl PartialOrd for TeamScore {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        let left_score = self.score.load(Ordering::Relaxed);
        let other_score = other.score.load(Ordering::Relaxed);

        Some(left_score.cmp(&other_score))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoalDetail {
    pub player_id: u32,
    pub stat_type: MatchStatisticType,
    pub is_auto_goal: bool,
    pub time: u64,
}

/// A goal-scoring situation that did not end in a goal — the save, the post,
/// the one dragged wide from six yards.
///
/// Recorded for every strike that clears [`HighlightSelector::MIN_XG`], which
/// is more of them than any match sheet wants; [`HighlightSelector`] cuts that
/// down to the handful worth keeping once the whistle has gone and the whole
/// match is in view. None of it can be decided while it is happening: how good
/// a chance was is only answerable against the other chances the same team had.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChanceDetail {
    pub player_id: u32,
    /// The side that created it — the shooter's, always. There is no own-goal
    /// case here: a strike at your own net is not a chance anybody wants to
    /// watch twice.
    pub team_id: u32,
    /// Milliseconds into the match, taken the instant the ball was struck. The
    /// same clock a [`GoalDetail`] is stamped with, and the same one the
    /// recorder cuts its clips against — the two have to agree exactly or a
    /// marker points at footage that was never kept.
    pub time: u64,
    /// Recorded (Opta-scale) expected goals of the strike. Population mean is
    /// around 0.11, so anything past 0.25 is a genuinely big chance.
    pub xg: f32,
    /// Whether the strike was aimed between the posts and under the bar, as
    /// judged the moment it left the boot — NOT where it finished. A shot
    /// aimed on target and blocked by a defender still reads `true`.
    pub on_target: bool,
}

/// Which of a match's near misses are worth a marker and a clip.
///
/// Every shot is a candidate and most of them are nothing; the point of this is
/// to end up with the two or three moments per side a highlight reel would
/// actually carry. Three rules, in the order of what they protect:
///
/// 1. **Quality.** Ranked on xG and nothing else — how the chance ended is
///    beside the point, because a header cleared off the line and a sitter
///    dragged wide were the same chance until somebody made a decision.
/// 2. **A quota per side.** Otherwise a one-sided match marks eight moments for
///    the team on top and none at all for the side that spent it defending.
/// 3. **Spacing.** Two markers a rebound apart land on the same pixel of the
///    timeline and describe one passage of play twice. The gap is a fraction of
///    the match rather than a fixed number of seconds, because the rail IS the
///    match: it has to hold at ninety minutes and at the five-minute halves a
///    debug build plays.
pub struct HighlightSelector;

impl HighlightSelector {
    /// At most this many per side. Two or three is a highlight reel; six is the
    /// match again.
    pub const PER_TEAM: usize = 3;

    /// Recorded xG a strike has to reach to count as a chance at all.
    ///
    /// Low on purpose, and it can afford to be: the quota above takes the BEST
    /// of what clears it, so a permissive bar costs a good match nothing and
    /// only decides what a bad one falls back on. Its whole job is to keep out
    /// the strike that was never going in — the hopeful thirty-yarder, the
    /// half-blocked stab — not to define a chance.
    ///
    /// ⚠ CALIBRATED AGAINST THE ENGINE'S OWN SCALE, NOT AN OPTA ONE. Recorded
    /// xG runs through `XG_REPORT_SCALE`, and `dev_match stats 20 14 14` puts
    /// the population at 25.5 shots a match with a MEAN of 0.050 and a 90th
    /// percentile of 0.108 — so 0.035 is about the median attempt, not the
    /// "big chance" the same number would mean in a published model. At this
    /// bar, 40 matches yield 12.4 candidates each and the shortlist keeps 2.3
    /// per side, with 78% of team-matches getting two markers or more and 8%
    /// getting none (a side that genuinely never threatened). Anything that
    /// moves the recorded-xG scale moves this with it — re-measure, don't
    /// convert.
    pub const MIN_XG: f32 = 0.035;

    /// How far apart two markers have to be, as a divisor of the match length —
    /// a thirtieth of ninety minutes is three of them.
    const SPACING_DIVISOR: u64 = 30;

    /// Trims `chances` to the shortlist and returns the timestamps that
    /// survived, in the order they happened — which is exactly the list the
    /// recorder needs to know which of its provisional clips to keep.
    ///
    /// `goals` are already on the reel and are never dropped; they take part
    /// here only by pushing chances out of the seconds around them. A save two
    /// seconds before a goal is not a separate moment, it is the goal's
    /// build-up.
    pub fn select(
        chances: &mut Vec<ChanceDetail>,
        goals: &[GoalDetail],
        total_match_time: u64,
    ) -> Vec<u64> {
        let spacing = (total_match_time / Self::SPACING_DIVISOR)
            .max(GOAL_CLIP_PRE_ROLL_MS + GOAL_CLIP_POST_ROLL_MS);

        // Everything already on the reel, which is what the spacing rule
        // measures against. Goals join it up front; each chance joins it as it
        // is taken, so three chances can never bunch either.
        let mut taken: Vec<u64> = goals
            .iter()
            .filter(|goal| goal.stat_type == MatchStatisticType::Goal)
            .map(|goal| goal.time)
            .collect();

        let mut ranked: Vec<usize> = (0..chances.len())
            .filter(|&index| chances[index].xg >= Self::MIN_XG)
            .collect();
        // Best first, and the earlier of two equal chances first — so the
        // shortlist is a function of the match rather than of the order some
        // earlier sort happened to leave things in.
        ranked.sort_by(|&a, &b| {
            chances[b]
                .xg
                .total_cmp(&chances[a].xg)
                .then(chances[a].time.cmp(&chances[b].time))
        });

        let mut kept = vec![false; chances.len()];
        let mut per_team: HashMap<u32, usize> = HashMap::new();
        for index in ranked {
            let chance = &chances[index];
            let quota = per_team.entry(chance.team_id).or_insert(0);
            if *quota >= Self::PER_TEAM {
                continue;
            }
            if taken
                .iter()
                .any(|already| already.abs_diff(chance.time) < spacing)
            {
                continue;
            }
            *quota += 1;
            taken.push(chance.time);
            kept[index] = true;
        }

        let mut index = 0;
        chances.retain(|_| {
            let keep = kept[index];
            index += 1;
            keep
        });
        chances.sort_by_key(|chance| chance.time);
        chances.iter().map(|chance| chance.time).collect()
    }
}

impl Score {
    pub fn new(home_team_id: u32, away_team_id: u32) -> Self {
        Score {
            home_team: TeamScore::new(home_team_id),
            away_team: TeamScore::new(away_team_id),
            details: Vec::new(),
            home_shootout: 0,
            away_shootout: 0,
        }
    }

    pub fn add_goal_detail(&mut self, goal_detail: GoalDetail) {
        self.details.push(goal_detail)
    }

    pub fn detail(&self) -> &[GoalDetail] {
        &self.details
    }

    pub fn increment_home_goals(&self) {
        self.home_team.score.fetch_add(1, Ordering::Relaxed);
    }

    pub fn increment_away_goals(&self) {
        self.away_team.score.fetch_add(1, Ordering::Relaxed);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchResult {
    pub id: String,
    pub league_id: u32,
    pub league_slug: String,
    pub home_team_id: u32,
    pub away_team_id: u32,
    pub details: Option<MatchResultRaw>,
    pub score: Score,
    pub friendly: bool,
}

impl MatchResult {
    pub fn copy_without_data_positions(&self) -> Self {
        MatchResult {
            id: String::from(&self.id),
            league_id: self.league_id,
            league_slug: String::from(&self.league_slug),
            home_team_id: self.home_team_id,
            away_team_id: self.away_team_id,
            details: if self.details.is_some() {
                Some(self.details.as_ref().unwrap().copy_without_data_positions())
            } else {
                None
            },
            score: self.score.clone(),
            friendly: self.friendly,
        }
    }
}

impl From<&LeagueMatch> for MatchResult {
    fn from(m: &LeagueMatch) -> Self {
        MatchResult {
            id: m.id.clone(),
            league_id: m.league_id,
            league_slug: m.league_slug.clone(),
            home_team_id: m.home_team_id,
            away_team_id: m.away_team_id,
            score: Score::new(m.home_team_id, m.away_team_id),
            details: None,
            friendly: false,
        }
    }
}

impl PartialEq for MatchResult {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

/// What reaches the match sheet out of a match's near misses.
///
/// Every rule here is one that only shows up in aggregate, which is why none of
/// them can be checked by looking at a single shot: a quota that doesn't bind
/// leaves a one-sided match with six markers for one team, a spacing rule that
/// doesn't bind puts three of them on the same pixel, and a goal window that
/// doesn't bind marks the saved shot that the rebound went in from as though it
/// were a separate moment.
#[cfg(test)]
mod highlight_selector_tests {
    use super::*;

    const HOME: u32 = 1;
    const AWAY: u32 = 2;
    /// Ninety minutes, so the spacing rule is three of them.
    const FULL_TIME: u64 = 90 * 60_000;

    fn chance(team_id: u32, minute: u64, xg: f32) -> ChanceDetail {
        ChanceDetail {
            player_id: team_id * 100 + minute as u32,
            team_id,
            time: minute * 60_000,
            xg,
            on_target: true,
        }
    }

    fn goal(minute: u64) -> GoalDetail {
        GoalDetail {
            player_id: 999,
            stat_type: MatchStatisticType::Goal,
            is_auto_goal: false,
            time: minute * 60_000,
        }
    }

    fn minutes(chances: &[ChanceDetail]) -> Vec<u64> {
        chances.iter().map(|c| c.time / 60_000).collect()
    }

    #[test]
    fn the_best_chances_are_the_ones_kept() {
        let mut chances = vec![
            chance(HOME, 10, 0.10),
            chance(HOME, 25, 0.40),
            chance(HOME, 40, 0.15),
            chance(HOME, 55, 0.30),
        ];
        let kept = HighlightSelector::select(&mut chances, &[], FULL_TIME);

        assert_eq!(
            minutes(&chances),
            vec![25, 40, 55],
            "the shortlist is not the three best chances"
        );
        // …and chronological, because that is the order the timeline draws
        // them in and the order the recorder walks its clips.
        assert_eq!(kept, vec![25 * 60_000, 40 * 60_000, 55 * 60_000]);
    }

    #[test]
    fn each_side_gets_its_own_quota() {
        // Everything the home side had is better than anything the away side
        // did — and the away side still gets its best moments. A team that
        // spent the match defending has near misses too, and a reel that only
        // shows the better team's is a highlight package of half a match.
        let mut chances = vec![
            chance(HOME, 5, 0.50),
            chance(HOME, 15, 0.48),
            chance(HOME, 25, 0.46),
            chance(HOME, 35, 0.44),
            chance(AWAY, 45, 0.12),
            chance(AWAY, 60, 0.11),
        ];
        HighlightSelector::select(&mut chances, &[], FULL_TIME);

        let home_kept = chances.iter().filter(|c| c.team_id == HOME).count();
        let away_kept = chances.iter().filter(|c| c.team_id == AWAY).count();
        assert_eq!(home_kept, HighlightSelector::PER_TEAM);
        assert_eq!(away_kept, 2, "the away side's own chances were crowded out");
    }

    #[test]
    fn one_passage_of_play_is_marked_once() {
        // A shot, the rebound, and the follow-up: three chances inside twenty
        // seconds. They are one moment, they would share one clip, and three
        // markers on top of each other is three seeks to the same footage.
        let mut chances = vec![
            ChanceDetail {
                time: 30 * 60_000,
                ..chance(HOME, 30, 0.30)
            },
            ChanceDetail {
                time: 30 * 60_000 + 8_000,
                ..chance(HOME, 30, 0.45)
            },
            ChanceDetail {
                time: 30 * 60_000 + 19_000,
                ..chance(HOME, 30, 0.28)
            },
        ];
        HighlightSelector::select(&mut chances, &[], FULL_TIME);

        assert_eq!(chances.len(), 1, "the same scramble was marked three times");
        assert_eq!(
            chances[0].time,
            30 * 60_000 + 8_000,
            "the moment kept was not the best of the three"
        );
    }

    #[test]
    fn the_shot_a_goal_came_from_is_not_a_separate_moment() {
        // The save at 70:00 and the goal off the rebound two seconds later are
        // one passage of play, and the goal is the part of it worth a marker.
        // The chance an hour earlier is untouched by the goal and survives.
        let mut chances = vec![
            chance(HOME, 10, 0.20),
            ChanceDetail {
                time: 70 * 60_000,
                ..chance(HOME, 70, 0.55)
            },
        ];
        HighlightSelector::select(&mut chances, &[goal(70)], FULL_TIME);

        assert_eq!(
            minutes(&chances),
            vec![10],
            "the build-up to a goal was marked as a chance of its own"
        );
    }

    #[test]
    fn a_hopeful_effort_is_not_a_chance() {
        // Under the bar it is not a goal-scoring situation, it is a shot. A
        // match of nothing but these gets no markers rather than three bad
        // ones — the reel says "nobody threatened", which is the truth.
        let mut chances = vec![
            chance(HOME, 10, HighlightSelector::MIN_XG - 0.01),
            chance(HOME, 30, 0.02),
            chance(AWAY, 50, 0.01),
        ];
        let kept = HighlightSelector::select(&mut chances, &[], FULL_TIME);

        assert!(chances.is_empty());
        assert!(kept.is_empty());
    }

    #[test]
    fn spacing_scales_with_the_match_rather_than_the_clock() {
        // A debug build plays five-minute halves. A fixed three-minute gap
        // would allow one marker per half there and the reel would be empty;
        // the rule is a fraction of the match, so a ten-minute match spaces its
        // markers twenty seconds apart and keeps all three.
        const SHORT: u64 = 10 * 60_000;
        let mut chances = vec![
            ChanceDetail {
                time: 60_000,
                ..chance(HOME, 1, 0.30)
            },
            ChanceDetail {
                time: 120_000,
                ..chance(HOME, 2, 0.35)
            },
            ChanceDetail {
                time: 180_000,
                ..chance(HOME, 3, 0.25)
            },
        ];
        HighlightSelector::select(&mut chances, &[], SHORT);

        assert_eq!(chances.len(), 3, "a short match lost its whole reel");
    }

    #[test]
    fn markers_never_land_closer_than_a_clip_apart() {
        // The floor under the spacing rule. However short the match, two clips
        // that overlap are one segment, and two markers over one segment are a
        // lie about how many moments it holds.
        const VERY_SHORT: u64 = 60_000;
        let mut chances = vec![
            ChanceDetail {
                time: 20_000,
                ..chance(HOME, 0, 0.30)
            },
            ChanceDetail {
                time: 25_000,
                ..chance(HOME, 0, 0.35)
            },
        ];
        HighlightSelector::select(&mut chances, &[], VERY_SHORT);

        assert_eq!(chances.len(), 1);
        assert_eq!(chances[0].time, 25_000);
    }
}
