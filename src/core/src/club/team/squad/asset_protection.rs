//! Central squad-asset protection policy.
//!
//! Every automatic squad-disposal path (loan-out identification, the
//! free-transfer release sweep, the transfer-listing pass) used to make
//! its own ad-hoc judgement about whether a player was surplus. Early in
//! a simulation — before the monthly [`crate::PlayerSquadStatus`] pass has
//! run, while the season sample is still tiny, with key players away on
//! international duty — those independent judgements disagreed and useful
//! senior players were loaned out or walked for free.
//!
//! This module is the single source of truth for "what is this player to
//! his club?". [`SquadAssetProtection::classify`] (and the batch
//! [`SquadAssetContext`]) turn observable, data-driven signals into one
//! [`SquadAssetClass`], and the disposal paths gate on that class instead
//! of re-deriving the answer.
//!
//! Core principles encoded here:
//!   * A coach never sees the hidden `current_ability` (CA) digit, so the
//!     classifier never reads it either. A player's standing is measured by
//!     his **observable level** — [`AbilityEstimator::observable_level`],
//!     which fuses his visible (position-weighted) skill, his match results,
//!     his training performance and a reputation benefit-of-the-doubt —
//!     exactly the evidence a coach actually has. Rank within a position
//!     group and the squad/group averages are all computed from that assessed
//!     level, so a player is judged on how he plays and applies himself, not
//!     on a number nobody can see. (Reputation also drives a separate,
//!     intra-squad top-quartile "recognised name" rescue below — the absolute
//!     nudge in the level and this relative rescue are complementary.)
//!   * `NotYetSet` is **unknown, not surplus** — when the formal squad
//!     status hasn't been assigned yet the role is inferred from that
//!     observable-level rank, reputation, age, position scarcity and
//!     prior-season minutes, and a player we genuinely cannot place is
//!     [`SquadAssetClass::UnknownNeedsEvaluation`] (free-transfer protected),
//!     never surplus.
//!   * Early-season low minutes are not evidence of being unwanted — the
//!     assessed level's results channel stays silent until a real sample
//!     exists (a benched player reads at his visible skill), and
//!     [`SquadEvidenceContext`] lets the appearance-driven disposal paths
//!     suppress themselves while the sample is small.
//!   * `KeyPlayer` / `FirstTeamRegular` (and their inferred equivalents)
//!     are always protected from loan and free transfer.
//!
//! Everything is a method on a struct (no free functions) and every type
//! is reached through a `use` at the file header (no inline paths), per
//! project convention.

use std::collections::HashMap;

use chrono::NaiveDate;

use crate::club::staff::perception::{AbilityEstimator, PotentialEstimator};
use crate::{Club, Person, Player, PlayerCollection, PlayerFieldPositionGroup, PlayerSquadStatus};

/// What a player is to his club, derived from observable signals. Ordered
/// loosely from most to least protected. The disposal paths read the
/// `is_*` predicates rather than matching variants directly so the policy
/// stays in one place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SquadAssetClass {
    /// A key player — the de-facto best in his position group, a recognised
    /// name at the club, or an explicit `KeyPlayer`. Never auto-loaned,
    /// never auto-listed, never auto-released.
    CorePlayer,
    /// A first-team-useful player: an explicit `FirstTeamRegular`, a
    /// top-of-group starter, a recent regular, or a recognised squad name.
    /// Protected from loan and free transfer just like a core player.
    FirstTeamUseful,
    /// Genuine rotation depth — near the squad/group level. Not loan- or
    /// list-protected (he can be loaned for development or sold if surplus),
    /// but conservative enough that he is never simply released for free.
    RotationUseful,
    /// A young player below the group level with believed upside — the
    /// development-loan profile. Eligible for a development loan; never the
    /// free-transfer scrapheap.
    ProspectDevelopment,
    /// Clearly below team level with no upside (or an undistinguished
    /// declining veteran): the only class that may be auto-released for
    /// free, listed, or loaned as pure surplus.
    TrueSurplus,
    /// Not enough signal to place him — and crucially this is **not**
    /// surplus. Protected from free transfer; the club must evaluate him
    /// (let the monthly squad-status pass run) before moving him on.
    UnknownNeedsEvaluation,
}

impl SquadAssetClass {
    /// First-team core: never auto-loaned out and never auto-transfer-listed
    /// by a numeric squad-management sweep. The player can still be sold if
    /// he himself asks out / is listed / is unhappy — those explicit paths
    /// are evaluated before any protection.
    pub fn is_first_team_protected(self) -> bool {
        matches!(
            self,
            SquadAssetClass::CorePlayer | SquadAssetClass::FirstTeamUseful
        )
    }

    /// Free transfer is the most conservative action a club can take, so
    /// only a genuinely surplus player may be auto-released. Everything
    /// else — including the deliberately conservative
    /// [`SquadAssetClass::UnknownNeedsEvaluation`] — is kept or, at most,
    /// transfer-listed.
    pub fn is_free_transfer_protected(self) -> bool {
        !matches!(self, SquadAssetClass::TrueSurplus)
    }

    /// Stable label for debug output / decision diagnostics.
    pub fn label(self) -> &'static str {
        match self {
            SquadAssetClass::CorePlayer => "core_player",
            SquadAssetClass::FirstTeamUseful => "first_team_useful",
            SquadAssetClass::RotationUseful => "rotation_useful",
            SquadAssetClass::ProspectDevelopment => "prospect_development",
            SquadAssetClass::TrueSurplus => "true_surplus",
            SquadAssetClass::UnknownNeedsEvaluation => "unknown_needs_evaluation",
        }
    }
}

/// How much evidence the current season has produced about a club's
/// players. Built from the squad's actual appearance record — no season
/// calendar required — so "early season" means "the club's most-used
/// player has barely featured", which is exactly the low-sample regime
/// the disposal paths must not over-read.
#[derive(Debug, Clone, Copy)]
pub struct SquadEvidenceContext {
    /// Maximum official appearances (league + cups) across the senior
    /// squad — a proxy for how many official matches the club has played
    /// this season.
    club_matches_proxy: u16,
}

impl SquadEvidenceContext {
    /// Below this many club matches the season hasn't produced a
    /// meaningful sample: a player's current minutes say little about
    /// whether the club wants him. Mirrors the brief's "fewer than 8-10
    /// official club matches" guidance.
    pub const LOW_EVIDENCE_MATCHES: u16 = 8;

    /// Build the season-sample context from the club's main squad. `date`
    /// is accepted for signature stability and future season-boundary use;
    /// the appearance proxy is calendar-independent and the load-bearing
    /// signal.
    pub fn current_season_sample(date: NaiveDate, club: &Club) -> Self {
        let _ = date;
        club.teams
            .main()
            .or_else(|| club.teams.teams.first())
            .map(|team| Self::from_squad(&team.players))
            .unwrap_or(SquadEvidenceContext {
                club_matches_proxy: 0,
            })
    }

    /// Build the season-sample context directly from one squad's roster —
    /// the team the classifying coach actually manages. The match-count
    /// proxy is the busiest non-loanee's official appearances, the same
    /// signal `current_season_sample` reads off the main team.
    pub fn from_squad(players: &PlayerCollection) -> Self {
        let club_matches_proxy = players
            .iter()
            .filter(|p| !p.is_on_loan())
            .map(Self::official_appearances)
            .max()
            .unwrap_or(0);
        SquadEvidenceContext { club_matches_proxy }
    }

    /// Official (league + cup) appearances for one player this season.
    fn official_appearances(player: &Player) -> u16 {
        player.statistics.played
            + player.statistics.played_subs
            + player.cup_statistics.played
            + player.cup_statistics.played_subs
    }

    /// The match-count proxy underlying the early-season judgement.
    pub fn club_matches_proxy(self) -> u16 {
        self.club_matches_proxy
    }

    /// True while the season is too young to read a player's current
    /// minutes as evidence of his standing at the club.
    pub fn is_early_season(self) -> bool {
        self.club_matches_proxy < Self::LOW_EVIDENCE_MATCHES
    }
}

/// Precomputed squad context that classifies every player against the same
/// snapshot. Built once per club per pass (mirrors the chemistry /
/// adaptation "build context once, share across players" pattern) so the
/// classifier never re-walks the roster per call.
pub struct SquadAssetContext {
    /// Observable-level values ([`AbilityEstimator::observable_level`]) per
    /// position group on the senior squad. Used for rank ("how many at my
    /// position are strictly better in the coach's eyes") and the group
    /// average. Never the hidden CA. Order within a group is irrelevant.
    group_levels: HashMap<PlayerFieldPositionGroup, Vec<u8>>,
    /// Mean observable level of the senior squad — the "team level" the
    /// classifier measures a player's assessed level against.
    squad_avg_level: u8,
    /// Mean *raw* current ability of the senior squad. Not used by the
    /// classifier itself (which is deliberately CA-blind) — retained only
    /// for the external release-eligibility gate, whose own numeric checks
    /// still compare a player's raw CA to a raw squad average, so its inputs
    /// stay internally consistent. Exposed via [`Self::squad_avg_ability`].
    squad_avg_ability: u8,
    /// Reputation value (max of current / home) at the squad's top-quartile
    /// boundary. A player strictly above it is a recognised name. `i16::MAX`
    /// for an empty squad so nobody is ever falsely flagged.
    top_quartile_reputation: i16,
    /// Season-sample evidence, carried so callers can suppress
    /// appearance-driven decisions while the sample is small.
    evidence: SquadEvidenceContext,
}

impl SquadAssetContext {
    /// A player counts as "top of his group" if at most this many group
    /// peers are strictly better (rank 0-2 = a top-three option).
    const TOP_GROUP_RANK: usize = 2;
    /// Minimum real position group size before the top-three rule applies —
    /// in a two-man group only the rank-0 starter is protected by rank.
    const MIN_GROUP_FOR_TOP_RANK: usize = 3;
    /// A top-group player at most this far below the group average still
    /// counts as "at his group's level".
    const NEAR_GROUP_GAP: i16 = 10;
    /// Oldest age still treated as a development prospect for the
    /// inferred-class ladder.
    const PROSPECT_MAX_AGE: u8 = 23;
    /// Believed-ceiling gap over current ability marking a genuine prospect.
    const CEILING_GAP: u8 = 8;
    /// A player within this much of the group/squad level is useful
    /// rotation depth rather than surplus.
    const ROTATION_GAP: i16 = 15;
    /// A player this far below the squad average has no squad role — the
    /// only quality gap that admits an automatic exit. Kept equal to the
    /// release gate's quality gap so the two agree.
    const SURPLUS_GAP: i16 = 25;
    /// Softer surplus gap for an old, clearly-declining player.
    const VETERAN_AGE: u8 = 35;
    const VETERAN_SURPLUS_GAP: i16 = 15;
    /// Official games in the most-recent completed season at or above which
    /// the player was a genuine regular and is first-team useful regardless
    /// of his current sample.
    const REGULAR_LAST_SEASON: u16 = 12;

    /// Build the classifier context from a club's senior (main) squad.
    pub fn build(club: &Club, date: NaiveDate) -> Self {
        let _ = date;
        match club.teams.main().or_else(|| club.teams.teams.first()) {
            Some(team) => Self::for_squad(&team.players),
            None => Self::for_squad(&PlayerCollection::new(Vec::new())),
        }
    }

    /// Build the classifier context from a single squad's roster — the
    /// team the classifying coach manages. `build` calls this on the club's
    /// main team; the head-coach contract-cleanup pass calls it directly
    /// with the team it is iterating, since it has no `Club` handle. The
    /// "team level" is then that squad's average, which is exactly the bar
    /// a reserve / youth coach measures his own deadwood against.
    pub fn for_squad(players: &PlayerCollection) -> Self {
        let mut group_levels: HashMap<PlayerFieldPositionGroup, Vec<u8>> = HashMap::new();
        let mut reputations: Vec<i16> = Vec::new();
        let mut level_sum: u32 = 0;
        let mut ca_sum: u32 = 0;
        let mut count: u32 = 0;

        for player in players.iter() {
            // Loanees belong to their parent club — they are not this
            // club's assets and must not skew the level / rank picture.
            if player.is_on_loan() {
                continue;
            }
            let group = player.position().position_group();
            // The rank / average picture is built from the coach-observable
            // level, never the hidden CA.
            let level = AbilityEstimator::observable_level(player);
            group_levels.entry(group).or_default().push(level);
            level_sum += level as u32;
            // Raw CA average kept solely for the external release gate.
            ca_sum += player.player_attributes.current_ability as u32;
            count += 1;
            reputations.push(Self::display_reputation(player));
        }

        let squad_avg_level = level_sum.checked_div(count).map(|avg| avg as u8).unwrap_or(0);
        let squad_avg_ability = ca_sum.checked_div(count).map(|avg| avg as u8).unwrap_or(0);
        let top_quartile_reputation = Self::top_quartile(&mut reputations);
        let evidence = SquadEvidenceContext::from_squad(players);

        SquadAssetContext {
            group_levels,
            squad_avg_level,
            squad_avg_ability,
            top_quartile_reputation,
            evidence,
        }
    }

    /// The senior-squad average *raw* current ability. This is **not** what
    /// the classifier ranks by (that is [`Self::squad_avg_level`]); it exists
    /// only for the external release-eligibility gate, whose numeric checks
    /// still work in raw-CA units. Prefer [`Self::squad_avg_level`] for any
    /// coach-facing "how does this player compare to the squad" judgement.
    pub fn squad_avg_ability(&self) -> u8 {
        self.squad_avg_ability
    }

    /// The senior-squad average *observable level* — the coach-visible "team
    /// level" the classifier measures each player's assessed level against.
    /// Callers making a surplus / keep decision should compare a player's
    /// [`AbilityEstimator::observable_level`] to this, so both sides of the
    /// comparison are in the same CA-blind units.
    pub fn squad_avg_level(&self) -> u8 {
        self.squad_avg_level
    }

    /// Season-sample view — lets a caller suppress appearance-driven
    /// disposal while the current sample is too small to trust.
    pub fn evidence(&self) -> SquadEvidenceContext {
        self.evidence
    }

    /// Convenience: is the season too young to read current minutes?
    pub fn is_early_season(&self) -> bool {
        self.evidence.is_early_season()
    }

    /// Classify one player against this squad snapshot.
    pub fn classify(&self, player: &Player, date: NaiveDate) -> SquadAssetClass {
        let Some(contract) = player.contract.as_ref() else {
            // A clubless player on the roster is a free agent, not a squad
            // asset — leave him for the dedicated free-agent flow.
            return SquadAssetClass::UnknownNeedsEvaluation;
        };

        // Explicit club designations win — the club has already decided.
        match contract.squad_status {
            PlayerSquadStatus::KeyPlayer => return SquadAssetClass::CorePlayer,
            PlayerSquadStatus::FirstTeamRegular => return SquadAssetClass::FirstTeamUseful,
            PlayerSquadStatus::FirstTeamSquadRotation => return SquadAssetClass::RotationUseful,
            PlayerSquadStatus::HotProspectForTheFuture | PlayerSquadStatus::DecentYoungster => {
                return SquadAssetClass::ProspectDevelopment;
            }
            // NotNeeded / Invalid are explicit "surplus / cleanup" decisions —
            // but a young player with genuine upside who is merely buried on
            // the depth chart is a development-loan asset, not free-transfer
            // scrapheap. Rescue that profile before honoring the surplus label
            // (otherwise a monthly CA-rank pass that stamps NotNeeded on a
            // deep-squad prospect makes him free-release-eligible).
            PlayerSquadStatus::NotNeeded | PlayerSquadStatus::Invalid => {
                // Explicit surplus — but a young player who merely projects as
                // a development prospect (buried, below his group's level, with
                // room to grow) is a loan asset, not free-transfer scrapheap.
                // Rescue exactly that inference; every other profile honors the
                // surplus label.
                if matches!(
                    self.infer(player, date),
                    SquadAssetClass::ProspectDevelopment
                ) {
                    return SquadAssetClass::ProspectDevelopment;
                }
                return SquadAssetClass::TrueSurplus;
            }
            // Backup and not-yet-evaluated fall through to inference: a
            // backup can be a useful #2 or genuine surplus, and `NotYetSet`
            // must be inferred — never treated as surplus by default.
            PlayerSquadStatus::MainBackupPlayer
            | PlayerSquadStatus::NotYetSet
            | PlayerSquadStatus::SquadStatusCount => {}
        }

        self.infer(player, date)
    }

    /// Infer the class from observable signals when there is no decisive
    /// formal status. Ranks by the coach-observable level (visible skill +
    /// results + training), reputation, age, potential and prior-season
    /// minutes — never the hidden CA, and never the current (possibly tiny)
    /// appearance count, so it is robust to early-season / international-duty
    /// gaps.
    fn infer(&self, player: &Player, date: NaiveDate) -> SquadAssetClass {
        let group = player.position().position_group();
        let level = AbilityEstimator::observable_level(player);
        let age = player.age(date);

        let group_size = self.group_size(group);
        let higher_in_group = self.higher_level_in_group(group, level);
        let group_avg = self.group_avg(group).unwrap_or(level) as i16;
        let squad_avg = self.squad_avg_level as i16;

        // De-facto starter — best in a genuinely contested position group.
        if group_size >= 2 && higher_in_group == 0 {
            return SquadAssetClass::CorePlayer;
        }

        // A recognised name at this club (top reputation tier) is a
        // first-team asset even with a thin current sample — the Zobnin
        // case: form may have dipped but standing has not.
        if self.is_high_reputation_for_club(player) {
            return SquadAssetClass::FirstTeamUseful;
        }

        // Was a genuine regular last completed season → first-team useful
        // regardless of how few games this (new) season has produced.
        if Self::was_recent_regular(player) {
            return SquadAssetClass::FirstTeamUseful;
        }

        // Top two-three of a real position group, at his group's level.
        if group_size >= Self::MIN_GROUP_FOR_TOP_RANK
            && higher_in_group <= Self::TOP_GROUP_RANK
            && (level as i16) >= group_avg - Self::NEAR_GROUP_GAP
        {
            return SquadAssetClass::FirstTeamUseful;
        }

        // Young and below his group's level — the development-loan profile.
        // A believed-high ceiling confirms it; failing that, being clearly
        // below the group is itself room to grow for a young player. Both
        // routes mean the same thing for disposal: loanable for development,
        // never the free-transfer scrapheap.
        let believed_upside = PotentialEstimator::observable_ceiling(player, date)
            > level.saturating_add(Self::CEILING_GAP);
        let clearly_below_group = (level as i16) <= group_avg - Self::NEAR_GROUP_GAP;
        if age <= Self::PROSPECT_MAX_AGE
            && (level as i16) < group_avg
            && (believed_upside || clearly_below_group)
        {
            return SquadAssetClass::ProspectDevelopment;
        }

        // Near the group or squad level → useful rotation depth.
        if (level as i16) >= group_avg - Self::ROTATION_GAP
            || (level as i16) >= squad_avg - Self::ROTATION_GAP
        {
            return SquadAssetClass::RotationUseful;
        }

        // Clearly below team level with no upside, or an undistinguished
        // declining veteran → genuine surplus. Mirrors the release gate's
        // quality gaps (in observable-level units) so the two agree.
        let clearly_below = (level as i16) <= squad_avg - Self::SURPLUS_GAP;
        let veteran_done =
            age >= Self::VETERAN_AGE && (level as i16) <= squad_avg - Self::VETERAN_SURPLUS_GAP;
        if clearly_below || veteran_done {
            return SquadAssetClass::TrueSurplus;
        }

        // Genuinely ambiguous — and, by design, NOT surplus.
        SquadAssetClass::UnknownNeedsEvaluation
    }

    fn group_size(&self, group: PlayerFieldPositionGroup) -> usize {
        self.group_levels.get(&group).map(|v| v.len()).unwrap_or(0)
    }

    /// Number of senior squad-mates in the same position group with a
    /// strictly higher observable level — the player's depth rank in the
    /// coach's eyes (0 = the best option at his position).
    fn higher_level_in_group(&self, group: PlayerFieldPositionGroup, level: u8) -> usize {
        self.group_levels
            .get(&group)
            .map(|v| v.iter().filter(|&&l| l > level).count())
            .unwrap_or(0)
    }

    fn group_avg(&self, group: PlayerFieldPositionGroup) -> Option<u8> {
        let levels = self.group_levels.get(&group)?;
        if levels.is_empty() {
            return None;
        }
        let sum: u32 = levels.iter().map(|&l| l as u32).sum();
        Some((sum / levels.len() as u32) as u8)
    }

    /// True when the player's reputation sits in the squad's top quartile —
    /// a recognised name at this club. Scale-independent (intra-squad), so
    /// it works regardless of how player and club reputations are scaled.
    fn is_high_reputation_for_club(&self, player: &Player) -> bool {
        Self::display_reputation(player) > self.top_quartile_reputation
    }

    /// The reputation value used for the squad percentile — the higher of
    /// current and home reputation, so a declining-but-famous player keeps
    /// his standing.
    fn display_reputation(player: &Player) -> i16 {
        player
            .player_attributes
            .current_reputation
            .max(player.player_attributes.home_reputation)
    }

    /// Top-quartile reputation boundary from a list of squad reputations.
    /// Sorts descending and returns the value at the 25% index; a player
    /// strictly above it is in the top quartile. `i16::MAX` for an empty
    /// squad so the high-rep test never fires spuriously.
    fn top_quartile(reputations: &mut [i16]) -> i16 {
        if reputations.is_empty() {
            return i16::MAX;
        }
        reputations.sort_unstable_by(|a, b| b.cmp(a));
        let idx = (reputations.len() / 4).min(reputations.len() - 1);
        reputations[idx]
    }

    /// True when the player logged a regular's worth of official games in
    /// his most-recent completed season (parent + loan spells summed).
    fn was_recent_regular(player: &Player) -> bool {
        let latest = player
            .statistics_history
            .items
            .iter()
            .map(|h| h.season.start_year)
            .max();
        let Some(year) = latest else {
            return false;
        };
        let games: u16 = player
            .statistics_history
            .items
            .iter()
            .filter(|h| h.season.start_year == year)
            .map(|h| h.statistics.total_games())
            .sum();
        games >= Self::REGULAR_LAST_SEASON
    }
}

/// One-shot facade for callers that classify a single player and don't
/// already hold a [`SquadAssetContext`]. Builds the context internally;
/// batch callers (the loan-out sweep) should build the context once and
/// reuse it.
pub struct SquadAssetProtection;

impl SquadAssetProtection {
    /// Classify `player` against `club`'s senior squad on `date`.
    pub fn classify(player: &Player, club: &Club, date: NaiveDate) -> SquadAssetClass {
        SquadAssetContext::build(club, date).classify(player, date)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::academy::ClubAcademy;
    use crate::club::player::core::builder::PlayerBuilder;
    use crate::league::Season;
    use crate::shared::Location;
    use crate::shared::fullname::FullName;
    use crate::{
        ClubColors, ClubFacilities, ClubFinances, ClubStatus, PersonAttributes, PlayerAttributes,
        PlayerClubContract, PlayerCollection, PlayerPosition, PlayerPositionType, PlayerPositions,
        PlayerSkills, PlayerStatistics, PlayerStatisticsHistoryItem, StaffCollection, TeamBuilder,
        TeamCollection, TeamReputation, TeamType, TrainingSchedule,
    };
    use chrono::{Datelike, NaiveDate, NaiveTime};

    /// Fixtures for the asset-protection classifier. Wrapped in a unit
    /// struct per the project's no-free-helpers convention.
    struct Fx;

    impl Fx {
        fn date() -> NaiveDate {
            NaiveDate::from_ymd_opt(2026, 9, 5).unwrap()
        }

        /// Flat skills tuned so the position-weighted visible ability lands
        /// on `target`. The classifier now ranks by observable level (skills
        /// + results + training), never the hidden CA digit, so fixtures must
        /// express quality through skills. Inverts `skill_to_ability`.
        fn skills_for(target: u8) -> PlayerSkills {
            PlayerSkills::flat_for_ability(target)
        }

        /// A contracted player whose *visible* ability (via skills) is `ca`;
        /// `current_ability` is also stamped to `ca` but is deliberately not
        /// read by the classifier. `rep` sets BOTH current and home
        /// reputation so the squad-percentile test is deterministic; `status`
        /// is the formal squad status (use `NotYetSet` to exercise inference).
        fn player(
            id: u32,
            position: PlayerPositionType,
            ca: u8,
            age: u8,
            rep: i16,
            status: PlayerSquadStatus,
        ) -> Player {
            let mut attrs = PlayerAttributes::default();
            attrs.current_ability = ca;
            attrs.potential_ability = ca;
            attrs.current_reputation = rep;
            attrs.home_reputation = rep;
            let birth_year = Self::date().year() - age as i32;
            let mut contract =
                PlayerClubContract::new(50_000, NaiveDate::from_ymd_opt(2030, 6, 30).unwrap());
            contract.squad_status = status;
            PlayerBuilder::new()
                .id(id)
                .full_name(FullName::new("Test".to_string(), format!("P{id}")))
                .birth_date(NaiveDate::from_ymd_opt(birth_year, 1, 1).unwrap())
                .country_id(1)
                .attributes(PersonAttributes::default())
                .skills(Fx::skills_for(ca))
                .positions(PlayerPositions {
                    positions: vec![PlayerPosition {
                        position,
                        level: 18,
                    }],
                })
                .player_attributes(attrs)
                .contract(Some(contract))
                .build()
                .unwrap()
        }

        /// A young prospect with believed upside. The upside is expressed
        /// through *person* attributes (professionalism / ambition), which
        /// the observable-ceiling estimate reads but which do NOT feed the
        /// visible skill ability — so the prospect stays clearly below his
        /// group's level (a development profile) rather than being lifted out
        /// of it by his own mentals.
        fn prospect(id: u32, position: PlayerPositionType, ca: u8, age: u8) -> Player {
            let mut p = Self::player(id, position, ca, age, 200, PlayerSquadStatus::NotYetSet);
            p.player_attributes.potential_ability = ca.saturating_add(40);
            p.attributes.professionalism = 18.0;
            p.attributes.ambition = 18.0;
            p
        }

        /// Give a player a current-season league record at a fixed average
        /// rating, driving the weighted ledger so the observable level's
        /// results channel reads a real sample.
        fn with_form(mut p: Player, games: u16, rating: f32) -> Player {
            p.statistics.played = games;
            p.statistics.rating_points = rating * games as f32;
            p.statistics.rating_weight = games as f32;
            p.statistics.average_rating = rating;
            p
        }

        fn season_row(year: u16, games: u16) -> PlayerStatisticsHistoryItem {
            let mut stats = PlayerStatistics::default();
            stats.played = games;
            PlayerStatisticsHistoryItem {
                season: Season::new(year),
                team_name: "T".to_string(),
                team_slug: "t".to_string(),
                team_reputation: 0,
                league_name: "L".to_string(),
                league_slug: "l".to_string(),
                is_loan: false,
                transfer_fee: None,
                statistics: stats,
                seq_id: year as u32,
            }
        }

        fn club(players: Vec<Player>) -> Club {
            let team = TeamBuilder::new()
                .id(10)
                .league_id(Some(1))
                .club_id(100)
                .name("Main".to_string())
                .slug("main".to_string())
                .team_type(TeamType::Main)
                .players(PlayerCollection::new(players))
                .staffs(StaffCollection::new(Vec::new()))
                .reputation(TeamReputation::new(6000, 6000, 6000))
                .training_schedule(TrainingSchedule::new(
                    NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
                    NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
                ))
                .build()
                .unwrap();
            Club::new(
                100,
                "Club".to_string(),
                Location::new(1),
                ClubFinances::new(10_000_000, Vec::new()),
                ClubAcademy::new(3),
                ClubStatus::Professional,
                ClubColors::default(),
                TeamCollection::new(vec![team]),
                ClubFacilities::default(),
            )
        }

        /// A midfield-heavy squad with a spread of abilities and a couple
        /// of recognised (high-reputation) names. Ids 1-3 are the better
        /// midfielders; id 1 the best. Default reputations are low (1000),
        /// so a high-rep test player stands out in the percentile.
        fn squad_with(extra: Vec<Player>) -> Vec<Player> {
            let mut players = vec![
                Fx::player(
                    1,
                    PlayerPositionType::MidfielderCenter,
                    130,
                    27,
                    1000,
                    PlayerSquadStatus::NotYetSet,
                ),
                Fx::player(
                    2,
                    PlayerPositionType::MidfielderCenter,
                    122,
                    26,
                    1000,
                    PlayerSquadStatus::NotYetSet,
                ),
                Fx::player(
                    3,
                    PlayerPositionType::MidfielderCenter,
                    118,
                    28,
                    1000,
                    PlayerSquadStatus::NotYetSet,
                ),
                Fx::player(
                    4,
                    PlayerPositionType::MidfielderCenter,
                    105,
                    24,
                    1000,
                    PlayerSquadStatus::NotYetSet,
                ),
                Fx::player(
                    5,
                    PlayerPositionType::Goalkeeper,
                    120,
                    29,
                    1000,
                    PlayerSquadStatus::NotYetSet,
                ),
            ];
            players.extend(extra);
            players
        }
    }

    // ── formal-status mapping ───────────────────────────────────────────

    #[test]
    fn formal_statuses_map_directly() {
        let club = Fx::club(Fx::squad_with(vec![]));
        let ctx = SquadAssetContext::build(&club, Fx::date());

        let key = Fx::player(
            90,
            PlayerPositionType::Striker,
            120,
            27,
            1000,
            PlayerSquadStatus::KeyPlayer,
        );
        assert_eq!(ctx.classify(&key, Fx::date()), SquadAssetClass::CorePlayer);

        let regular = Fx::player(
            91,
            PlayerPositionType::Striker,
            120,
            27,
            1000,
            PlayerSquadStatus::FirstTeamRegular,
        );
        assert_eq!(
            ctx.classify(&regular, Fx::date()),
            SquadAssetClass::FirstTeamUseful
        );

        let not_needed = Fx::player(
            92,
            PlayerPositionType::Striker,
            120,
            27,
            1000,
            PlayerSquadStatus::NotNeeded,
        );
        assert_eq!(
            ctx.classify(&not_needed, Fx::date()),
            SquadAssetClass::TrueSurplus
        );
    }

    // ── NotYetSet inference (the headline cases) ────────────────────────

    #[test]
    fn notyetset_de_facto_starter_is_core() {
        // Id 1 is the best midfielder; with NotYetSet he must still be read
        // as the de-facto starter, not surplus.
        let club = Fx::club(Fx::squad_with(vec![]));
        let ctx = SquadAssetContext::build(&club, Fx::date());
        let best = &club.teams.teams[0].players.players[0];
        assert_eq!(best.id, 1);
        assert_eq!(ctx.classify(best, Fx::date()), SquadAssetClass::CorePlayer);
    }

    #[test]
    fn notyetset_high_reputation_senior_is_first_team_useful() {
        // The Zobnin case: CA below the de-facto starters and NotYetSet,
        // but a recognised name (top-quartile reputation) — must be a
        // protected first-team asset, never surplus.
        let star = Fx::player(
            50,
            PlayerPositionType::MidfielderCenter,
            111,
            31,
            9000,
            PlayerSquadStatus::NotYetSet,
        );
        let club = Fx::club(Fx::squad_with(vec![star]));
        let ctx = SquadAssetContext::build(&club, Fx::date());
        let star_ref = club.teams.teams[0]
            .players
            .players
            .iter()
            .find(|p| p.id == 50)
            .unwrap();
        let class = ctx.classify(star_ref, Fx::date());
        assert_eq!(class, SquadAssetClass::FirstTeamUseful);
        assert!(class.is_first_team_protected());
        assert!(class.is_free_transfer_protected());
    }

    #[test]
    fn notyetset_recent_regular_is_first_team_useful() {
        // No reputation edge, mid CA, NotYetSet — but a full regular season
        // behind him. Prior-season minutes carry the classification.
        let mut regular = Fx::player(
            60,
            PlayerPositionType::MidfielderCenter,
            112,
            26,
            1000,
            PlayerSquadStatus::NotYetSet,
        );
        regular
            .statistics_history
            .items
            .push(Fx::season_row(2025, 30));
        let club = Fx::club(Fx::squad_with(vec![regular]));
        let ctx = SquadAssetContext::build(&club, Fx::date());
        let r = club.teams.teams[0]
            .players
            .players
            .iter()
            .find(|p| p.id == 60)
            .unwrap();
        assert_eq!(
            ctx.classify(r, Fx::date()),
            SquadAssetClass::FirstTeamUseful
        );
    }

    #[test]
    fn notyetset_blocked_young_prospect_is_development() {
        // Young, below the group level, with believed upside, buried behind
        // better midfielders → development profile (loanable, not surplus).
        let prospect = Fx::prospect(70, PlayerPositionType::MidfielderCenter, 100, 19);
        let club = Fx::club(Fx::squad_with(vec![prospect]));
        let ctx = SquadAssetContext::build(&club, Fx::date());
        let p = club.teams.teams[0]
            .players
            .players
            .iter()
            .find(|p| p.id == 70)
            .unwrap();
        let class = ctx.classify(p, Fx::date());
        assert_eq!(class, SquadAssetClass::ProspectDevelopment);
        // A prospect is loanable for development, but never free-transferred.
        assert!(!class.is_first_team_protected());
        assert!(class.is_free_transfer_protected());
    }

    #[test]
    fn notneeded_young_prospect_is_rescued_for_development() {
        // Explicit NotNeeded on a young player with real upside must NOT go
        // straight to free-transfer scrapheap — the guard keeps him a
        // development-loan asset (otherwise a CA-rank pass stamping NotNeeded
        // on a buried prospect makes him free-release-eligible).
        let mut prospect = Fx::prospect(71, PlayerPositionType::MidfielderCenter, 100, 19);
        prospect.contract.as_mut().unwrap().squad_status = PlayerSquadStatus::NotNeeded;
        let club = Fx::club(Fx::squad_with(vec![prospect]));
        let ctx = SquadAssetContext::build(&club, Fx::date());
        let p = club.teams.teams[0]
            .players
            .players
            .iter()
            .find(|p| p.id == 71)
            .unwrap();
        let class = ctx.classify(p, Fx::date());
        assert_eq!(class, SquadAssetClass::ProspectDevelopment);
        assert!(class.is_free_transfer_protected());
    }

    #[test]
    fn notneeded_ageing_player_is_still_true_surplus() {
        // An ageing NotNeeded player with no ceiling headroom stays genuine
        // surplus — the rescue is only for young players with upside.
        let mut vet = Fx::player(
            72,
            PlayerPositionType::MidfielderCenter,
            90,
            33,
            500,
            PlayerSquadStatus::NotNeeded,
        );
        vet.player_attributes.potential_ability = 90;
        let club = Fx::club(Fx::squad_with(vec![vet]));
        let ctx = SquadAssetContext::build(&club, Fx::date());
        let v = club.teams.teams[0]
            .players
            .players
            .iter()
            .find(|p| p.id == 72)
            .unwrap();
        assert_eq!(ctx.classify(v, Fx::date()), SquadAssetClass::TrueSurplus);
    }

    #[test]
    fn notyetset_clearly_below_low_rep_is_true_surplus() {
        // Old, well below the squad average, no reputation, no upside,
        // NotYetSet → genuine surplus that the disposal paths may move on.
        let fringe = Fx::player(
            80,
            PlayerPositionType::MidfielderCenter,
            85,
            33,
            500,
            PlayerSquadStatus::NotYetSet,
        );
        let club = Fx::club(Fx::squad_with(vec![fringe]));
        let ctx = SquadAssetContext::build(&club, Fx::date());
        let f = club.teams.teams[0]
            .players
            .players
            .iter()
            .find(|p| p.id == 80)
            .unwrap();
        let class = ctx.classify(f, Fx::date());
        assert_eq!(class, SquadAssetClass::TrueSurplus);
        assert!(!class.is_free_transfer_protected());
    }

    #[test]
    fn notyetset_mid_player_is_rotation_not_surplus() {
        // Squad-average-ish, unremarkable reputation, no prior season — not
        // a starter, but clearly NOT surplus. Must land on a protected,
        // non-disposable class.
        let rotation = Fx::player(
            81,
            PlayerPositionType::MidfielderCenter,
            114,
            27,
            1000,
            PlayerSquadStatus::NotYetSet,
        );
        let club = Fx::club(Fx::squad_with(vec![rotation]));
        let ctx = SquadAssetContext::build(&club, Fx::date());
        let r = club.teams.teams[0]
            .players
            .players
            .iter()
            .find(|p| p.id == 81)
            .unwrap();
        let class = ctx.classify(r, Fx::date());
        assert_ne!(class, SquadAssetClass::TrueSurplus);
        assert!(class.is_free_transfer_protected());
    }

    // ── observable level, not the CA digit ──────────────────────────────

    #[test]
    fn high_ca_digit_with_poor_observable_level_is_surplus() {
        // The headline case. This player's hidden CA is stamped at 130 — the
        // top of the squad — but on the pitch he looks like an 85 (low
        // visible skill, no form). The coach never sees the digit, only the
        // level, so he is genuine surplus, NOT the de-facto starter his CA
        // would have made him under a CA-ranked classifier.
        let mut impostor = Fx::player(
            60,
            PlayerPositionType::MidfielderCenter,
            85,
            30,
            500,
            PlayerSquadStatus::NotYetSet,
        );
        impostor.player_attributes.current_ability = 130;
        let club = Fx::club(Fx::squad_with(vec![impostor]));
        let ctx = SquadAssetContext::build(&club, Fx::date());
        let p = club.teams.teams[0]
            .players
            .players
            .iter()
            .find(|p| p.id == 60)
            .unwrap();
        assert_eq!(ctx.classify(p, Fx::date()), SquadAssetClass::TrueSurplus);
    }

    #[test]
    fn strong_form_rescues_a_fringe_player_from_surplus() {
        // Same visible skill (a borderline-surplus 88), same everything —
        // except one has torn the league up all season. His results lift his
        // assessed level out of the surplus band (he is kept / evaluated,
        // never walked), while his no-form twin is genuine surplus. Results
        // drive the judgement, not the identical underlying skill.
        let make = |form: Option<(u16, f32)>| {
            let mut p = Fx::player(
                60,
                PlayerPositionType::MidfielderCenter,
                88,
                27,
                1000,
                PlayerSquadStatus::NotYetSet,
            );
            if let Some((games, rating)) = form {
                p = Fx::with_form(p, games, rating);
            }
            Fx::club(Fx::squad_with(vec![p]))
        };

        let no_form = make(None);
        let ctx = SquadAssetContext::build(&no_form, Fx::date());
        let p = no_form.teams.teams[0]
            .players
            .players
            .iter()
            .find(|p| p.id == 60)
            .unwrap();
        assert_eq!(
            ctx.classify(p, Fx::date()),
            SquadAssetClass::TrueSurplus,
            "no-form fringe player is surplus"
        );

        let in_form = make(Some((30, 7.8)));
        let ctx = SquadAssetContext::build(&in_form, Fx::date());
        let p = in_form.teams.teams[0]
            .players
            .players
            .iter()
            .find(|p| p.id == 60)
            .unwrap();
        let class = ctx.classify(p, Fx::date());
        assert_ne!(
            class,
            SquadAssetClass::TrueSurplus,
            "a strong performer must not be surplus"
        );
        assert!(class.is_free_transfer_protected());
    }

    #[test]
    fn training_performance_shifts_the_surplus_judgement() {
        // Two identical fringe players with no match minutes; the only fresh
        // signal is training. The diligent one is assessed strictly more
        // favourably than the slacker — training is the coach's read on the
        // players the surplus judgement most concerns, the ones who never get
        // on. `protection_rank` orders the classes most-protected first.
        let protection_rank = |c: SquadAssetClass| -> u8 {
            match c {
                SquadAssetClass::CorePlayer => 5,
                SquadAssetClass::FirstTeamUseful => 4,
                SquadAssetClass::RotationUseful => 3,
                SquadAssetClass::UnknownNeedsEvaluation => 2,
                SquadAssetClass::ProspectDevelopment => 1,
                SquadAssetClass::TrueSurplus => 0,
            }
        };
        let classify_with_training = |train: f32| {
            let mut p = Fx::player(
                60,
                PlayerPositionType::MidfielderCenter,
                95,
                27,
                1000,
                PlayerSquadStatus::NotYetSet,
            );
            p.training.training_performance = train;
            let club = Fx::club(Fx::squad_with(vec![p]));
            let ctx = SquadAssetContext::build(&club, Fx::date());
            let p = club.teams.teams[0]
                .players
                .players
                .iter()
                .find(|p| p.id == 60)
                .unwrap();
            ctx.classify(p, Fx::date())
        };

        let slacker = classify_with_training(2.0);
        let grafter = classify_with_training(18.0);
        assert!(
            protection_rank(grafter) > protection_rank(slacker),
            "the hard trainer ({grafter:?}) must be assessed above the slacker ({slacker:?})",
        );
    }

    // ── evidence context ────────────────────────────────────────────────

    #[test]
    fn early_season_detected_from_thin_appearance_sample() {
        // A squad whose busiest player has only a handful of official games
        // is in the low-evidence regime.
        let club = Fx::club(Fx::squad_with(vec![]));
        let evidence = SquadEvidenceContext::current_season_sample(Fx::date(), &club);
        assert!(evidence.is_early_season());
        assert!(evidence.club_matches_proxy() < SquadEvidenceContext::LOW_EVIDENCE_MATCHES);
    }

    #[test]
    fn established_season_detected_once_matches_accumulate() {
        let mut players = Fx::squad_with(vec![]);
        // Give the busiest player a full league sample.
        players[0].statistics.played = 20;
        let club = Fx::club(players);
        let evidence = SquadEvidenceContext::current_season_sample(Fx::date(), &club);
        assert!(!evidence.is_early_season());
    }

    // ── class predicate semantics ───────────────────────────────────────

    #[test]
    fn predicate_semantics() {
        assert!(SquadAssetClass::CorePlayer.is_first_team_protected());
        assert!(SquadAssetClass::FirstTeamUseful.is_first_team_protected());
        assert!(!SquadAssetClass::RotationUseful.is_first_team_protected());
        assert!(!SquadAssetClass::ProspectDevelopment.is_first_team_protected());

        // Everything except genuine surplus is free-transfer protected.
        assert!(SquadAssetClass::UnknownNeedsEvaluation.is_free_transfer_protected());
        assert!(SquadAssetClass::RotationUseful.is_free_transfer_protected());
        assert!(!SquadAssetClass::TrueSurplus.is_free_transfer_protected());
    }

    #[test]
    fn facade_matches_context() {
        let club = Fx::club(Fx::squad_with(vec![]));
        let best = &club.teams.teams[0].players.players[0];
        assert_eq!(
            SquadAssetProtection::classify(best, &club, Fx::date()),
            SquadAssetContext::build(&club, Fx::date()).classify(best, Fx::date())
        );
    }
}
