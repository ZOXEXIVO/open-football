//! The coach's standing intention for each player in his squad.
//!
//! Everything the coach layer did before this was a *reaction*: a
//! per-fixture strategy, a per-match memory update, a signed nudge folded
//! into one selection. Nothing anywhere recorded what the coach actually
//! meant to do with a man over a season — who he was building around, who
//! he was grooming, whose competition the domestic cup was, and who he had
//! simply stopped counting on. So the squad had no shape and no direction:
//! each week's team was re-derived from ability and form, and a player's
//! standing never changed except as an accident of those two.
//!
//! [`CoachSquadPlan`] is that missing intention, held on the coach (not
//! the player) because it is his opinion, revised monthly and on his
//! arrival — a new manager really does tear up his predecessor's plan.
//!
//! It is deliberately a PLAN, not a mechanism: nothing here moves a
//! player. Consumers read it and decide — selection gives the cup keeper
//! his competition, the renewal desk declines to re-sign a man who is not
//! in the plans, the listing desk knows who is in the shop window. That
//! keeps the plan honest and removable: with an empty plan every consumer
//! falls back to exactly the behaviour it had before.

use rustc_hash::FxHashMap;

use chrono::NaiveDate;

use crate::club::person::Person;
use crate::club::player::PlayerUsage;
use crate::club::player::calculators::PlayerValueCalculator;
use crate::club::player::statistics::{FootballRecord, StuckCareerScan};
use crate::club::staff::coach::chart::SquadDepthChart;
use crate::club::staff::coach::memory::CoachMemoryStore;
use crate::club::staff::coach::shares::{
    PlanFloor, PlanFloorKind, ShareClimate, ShirtCandidate, ShirtShares,
};
use crate::club::staff::coach::standing::{StandingRead, StandingRung};
use crate::club::staff::goalkeeping::{KeeperRoomPlan, KeeperTier};
use crate::club::staff::perception::{AbilityEstimator, PotentialEstimator};
use crate::{
    MatchTacticType, Player, PlayerCollection, PlayerFieldPositionGroup, PlayerPositionType,
    Tactics,
};

/// What the coach intends to do with a player this season.
///
/// Ordered loosely from most to least central so a consumer can compare
/// standing without a lookup table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum PlannedRole {
    /// The team is built around him.
    Cornerstone,
    /// In the strongest available XI.
    Starter,
    /// Genuinely rotated — a real share of competitive minutes.
    Rotation,
    /// The deputy whose competition is the domestic cup. A real role with
    /// real matches, and the honest answer for a second-choice keeper
    /// (where only one man can play the league).
    CupKeeper,
    /// Being groomed to take a specific incumbent's place.
    SuccessionHeir,
    /// A prospect on a development path — minutes where the stakes allow.
    DevelopmentPathway,
    /// Squad depth: he plays when others cannot.
    Cover,
    /// The club would do business at the right price.
    ShopWindow,
    /// No future here. The honest end of the ladder, and the one the
    /// player is entitled to be told about.
    NotInPlans,
}

impl PlannedRole {
    /// Roles that commit the coach to giving the player real football.
    /// A plan that promises minutes is a promise the verifier can check.
    pub fn promises_minutes(self) -> bool {
        matches!(
            self,
            Self::Cornerstone
                | Self::Starter
                | Self::Rotation
                | Self::CupKeeper
                | Self::SuccessionHeir
                | Self::DevelopmentPathway
        )
    }

    /// Roles under which the club has no reason to offer fresh terms.
    pub fn is_exit_path(self) -> bool {
        matches!(self, Self::ShopWindow | Self::NotInPlans)
    }

    /// Every role, most central first. The order is the ladder, and it is
    /// what makes "a step down" a well-defined move.
    pub const LADDER: [PlannedRole; 9] = [
        Self::Cornerstone,
        Self::Starter,
        Self::Rotation,
        Self::CupKeeper,
        Self::SuccessionHeir,
        Self::DevelopmentPathway,
        Self::Cover,
        Self::ShopWindow,
        Self::NotInPlans,
    ];

    /// Packed form, for the dossier. A *lower* value is a higher role.
    #[inline]
    pub fn as_u8(self) -> u8 {
        self as u8
    }

    /// Inverse of [`Self::as_u8`]. An unknown value reads as
    /// [`PlannedRole::Cornerstone`] only because zero is the discriminant it
    /// holds; every caller passes a value this type produced.
    pub fn from_u8(value: u8) -> Self {
        Self::LADDER
            .get(value as usize)
            .copied()
            .unwrap_or(Self::NotInPlans)
    }

    /// One rung more central, saturating at the top.
    pub fn promoted(self) -> Self {
        Self::from_u8(self.as_u8().saturating_sub(1))
    }

    /// One rung less central, saturating at the bottom.
    pub fn demoted(self) -> Self {
        Self::from_u8((self.as_u8() + 1).min(Self::NotInPlans.as_u8()))
    }

    /// True when `self` is at least as central as `other`.
    #[inline]
    pub fn is_at_least(self, other: PlannedRole) -> bool {
        self.as_u8() <= other.as_u8()
    }

    /// Stable key for the events feed / UI.
    pub fn as_i18n_key(self) -> &'static str {
        match self {
            Self::Cornerstone => "planned_role_cornerstone",
            Self::Starter => "planned_role_starter",
            Self::Rotation => "planned_role_rotation",
            Self::CupKeeper => "planned_role_cup_keeper",
            Self::SuccessionHeir => "planned_role_succession_heir",
            Self::DevelopmentPathway => "planned_role_development_pathway",
            Self::Cover => "planned_role_cover",
            Self::ShopWindow => "planned_role_shop_window",
            Self::NotInPlans => "planned_role_not_in_plans",
        }
    }
}

/// One player's place in the plan.
#[derive(Debug, Clone, Copy)]
pub struct PlayerPlanEntry {
    pub role: PlannedRole,
    /// When the coach last committed to this role — so a consumer can
    /// tell a fresh decision from a standing one, and so a change of
    /// role is visible as an event rather than a silent re-derivation.
    pub set_on: NaiveDate,
    /// For [`PlannedRole::SuccessionHeir`], the incumbent he is being
    /// groomed to replace.
    pub succeeds: Option<u32>,
    /// Progress belongs to this intention, and survives unchanged reviews.
    pub baseline: PlayerUsage,
    /// The shirt of the formation he is planned for.
    pub shirt: Option<PlayerPositionType>,
    /// Men planned ahead of him for it: 0 is the first choice.
    pub place: u8,
    /// The share of the side's football the plan gives him: 1.0 is one
    /// shirt for every minute of every fixture.
    pub share: f32,
    /// A commitment the plan could not keep, because a stronger one asked
    /// for the same shirt.
    pub unmet_floor: Option<PlanFloorKind>,
}

impl PlayerPlanEntry {
    /// Football the plan owes him, in matches: what his share would have
    /// given him over the fixtures he was eligible for since his role was
    /// set, less what he has played. Negative when he is ahead of it.
    pub fn owed(&self, player: &Player) -> f32 {
        let progress = PlayerUsage::of(player).since(self.baseline);
        self.share * f32::from(progress.eligible) - progress.minutes as f32 / 90.0
    }

    /// The part of the football the plan has given him so far that he has
    /// not had: nothing when he has played it all, 1.0 when none of it.
    pub fn shortfall(&self, player: &Player) -> f32 {
        let progress = PlayerUsage::of(player).since(self.baseline);
        let given = self.share * f32::from(progress.eligible);
        if given <= 0.0 {
            return 0.0;
        }
        ((given - progress.minutes as f32 / 90.0) / given).clamp(0.0, 1.0)
    }
}

/// What a coach plans from: the squad in front of him, the shape he plays,
/// and the season the club expects.
pub struct PlanReading<'a> {
    pub players: &'a PlayerCollection,
    pub tactics: &'a Tactics,
    pub keepers: Option<&'a KeeperRoomPlan>,
    pub memory: Option<&'a CoachMemoryStore>,
    pub climate: ShareClimate,
    /// The coach whose plan it is, for the promises he made himself.
    pub coach_id: u32,
    /// How far the board's wish to see a man sold binds a manager who has
    /// frozen him out, 0..1.
    pub board_hold: f32,
    pub today: NaiveDate,
}

impl<'a> PlanReading<'a> {
    /// A reading with nothing known beyond the squad and its shape.
    pub fn of(players: &'a PlayerCollection, tactics: &'a Tactics, today: NaiveDate) -> Self {
        PlanReading {
            players,
            tactics,
            keepers: None,
            memory: None,
            climate: ShareClimate::ONE_COMPETITION,
            coach_id: 0,
            board_hold: 1.0,
            today,
        }
    }
}

/// Every player the coach currently holds an opinion about.
#[derive(Debug, Clone, Default)]
pub struct CoachSquadPlan {
    entries: FxHashMap<u32, PlayerPlanEntry>,
    last_revised: Option<NaiveDate>,
    /// The shape the plan was drawn for.
    formation: Option<MatchTacticType>,
}

impl CoachSquadPlan {
    /// How long a plan stands before the coach revisits it.
    const REVISION_DAYS: i64 = 30;
    /// A first choice this near the best first choice in the squad is a man
    /// the team is built around.
    const CORNERSTONE_GAP: f32 = 5.0;
    /// Oldest a groomed heir can be, and the age from which the man ahead
    /// of him is worth succeeding.
    const HEIR_MAX_AGE: u8 = 24;
    const INCUMBENT_AGEING_FROM: u8 = 31;
    /// Believed growth, in levels, at which a man behind a first choice is
    /// on a development path.
    const DEVELOPMENT_GROWTH: u8 = 8;
    /// A stalled man this far below the squad has nothing left to offer it.
    const NOT_IN_PLANS_GAP: f32 = 20.0;

    pub fn new() -> Self {
        Self::default()
    }

    pub fn role_of(&self, player_id: u32) -> Option<PlannedRole> {
        self.entries.get(&player_id).map(|e| e.role)
    }

    pub fn entry(&self, player_id: u32) -> Option<&PlayerPlanEntry> {
        self.entries.get(&player_id)
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The football the plan gives out, summed over the squad.
    pub fn total_share(&self) -> f32 {
        self.entries.values().map(|e| e.share).sum()
    }

    /// True when the standing plan is old enough to be worth revisiting.
    pub fn due_for_revision(&self, today: NaiveDate) -> bool {
        match self.last_revised {
            None => true,
            Some(last) => (today - last).num_days() >= Self::REVISION_DAYS,
        }
    }

    /// True when the plan no longer describes the squad: a month has
    /// passed, the shape has changed, a man has joined or left, or a role
    /// was set by hand and has not been drawn onto the chart.
    pub fn is_stale(&self, players: &PlayerCollection, tactics: &Tactics, today: NaiveDate) -> bool {
        self.due_for_revision(today)
            || self.formation != Some(tactics.tactic_type)
            || players.players.len() != self.entries.len()
            || players
                .iter()
                .any(|p| self.entries.get(&p.id).is_none_or(|e| e.shirt.is_none()))
    }

    /// The squad has changed under the plan: the next daily look redraws it.
    pub fn mark_stale(&mut self) {
        self.last_revised = None;
    }

    /// Set a role directly, outside the revision.
    ///
    /// The revision is the ordinary route and this is the exception: a
    /// manager who has decided a man has no future does not wait until the
    /// first of next month to act on it, and a reunion floor has to be in
    /// place before the first team sheet. The next drawing of the chart
    /// gives the role its shirt and its share and leaves the role alone.
    pub fn force_role(&mut self, player_id: u32, role: PlannedRole, today: NaiveDate) {
        self.entries.insert(
            player_id,
            PlayerPlanEntry {
                role,
                set_on: today,
                succeeds: None,
                baseline: Default::default(),
                shirt: None,
                place: 0,
                share: 0.0,
                unmet_floor: None,
            },
        );
    }

    /// Drop one player from the plan.
    ///
    /// Called when a spell ends: a plan for a man who has left the club is
    /// not an opinion, it is a stale row that the renewal desk and the
    /// listing sweep would both read as current.
    pub fn remove(&mut self, player_id: u32) {
        self.entries.remove(&player_id);
    }

    /// A new manager inherits a squad, not a plan.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.last_revised = None;
        self.formation = None;
    }

    /// Re-derive the plan across a squad, returning the players whose role
    /// CHANGED so the caller can tell them.
    ///
    /// The plan is drawn on the formation: a first choice for every shirt
    /// and the men behind him, from what the coach can observe — assessed
    /// level and ceiling, how well each man plays the position, and where
    /// each stands with him. Never from hidden ability. Each shirt's
    /// football is then divided between the men planned for it, so the
    /// shares of a squad add up to the football its formation plays.
    pub fn revise(&mut self, reading: &PlanReading<'_>) -> Vec<(u32, PlannedRole)> {
        let today = reading.today;
        let squad: Vec<&Player> = reading.players.iter().collect();
        let chart = SquadDepthChart::build(
            &squad,
            reading.tactics,
            reading.keepers,
            reading.memory,
            reading.climate.side,
            today,
        );
        let by_id: FxHashMap<u32, &Player> = squad.iter().map(|p| (p.id, *p)).collect();
        let standing_of =
            |id: u32| reading.memory.and_then(|store| store.standing_of(id));

        let mut drafted: FxHashMap<u32, PlayerPlanEntry> = FxHashMap::default();
        for depth in chart.shirts() {
            let keeper = depth.slot == PlayerPositionType::Goalkeeper;
            let men: Vec<&Player> = depth
                .men
                .iter()
                .filter_map(|(id, _)| by_id.get(id).copied())
                .collect();
            let roles: Vec<(PlannedRole, Option<u32>)> = men
                .iter()
                .map(|player| {
                    // The club's decision to sell him is the fact; the
                    // coach's picture of the shirt does not argue with it.
                    if player.is_on_the_market() {
                        return (PlannedRole::ShopWindow, None);
                    }
                    let set_by_hand = self
                        .entries
                        .get(&player.id)
                        .filter(|standing| standing.shirt.is_none())
                        .map(|standing| standing.role);
                    if let Some(role) = set_by_hand {
                        return (role, None);
                    }
                    let place = chart.place_of(player.id).map(|p| p.place).unwrap_or(0);
                    let (derived, succeeds) = Self::keeper_role(player, reading.keepers)
                        .map(|role| (role, None))
                        .unwrap_or_else(|| Self::derive_role(player, &chart, &by_id, today));
                    (
                        StandingRead::bound_role(derived, standing_of(player.id), place),
                        succeeds,
                    )
                })
                .collect();

            // A man the coach is finished with holds a place in no shirt's
            // football. One the club is selling keeps what his place earns
            // him, with the shop window as a floor under it, unless the
            // coach has frozen him out: then the window is all he is given.
            let sharing: Vec<usize> = (0..men.len())
                .filter(|i| roles[*i].0 != PlannedRole::NotInPlans)
                .collect();
            let for_sale = |i: usize| roles[i].0 == PlannedRole::ShopWindow;
            let frozen_out = |i: usize| {
                standing_of(men[i].id)
                    .is_some_and(|standing| standing.rung == StandingRung::FrozenOut)
            };
            let dividing: Vec<usize> = sharing
                .iter()
                .copied()
                .filter(|i| !(for_sale(*i) && frozen_out(*i)))
                .collect();
            let candidates: Vec<ShirtCandidate> = dividing
                .iter()
                .map(|&i| ShirtCandidate {
                    standing: depth.men[i].1,
                    belief: PotentialEstimator::believed_growth(men[i], today),
                })
                .collect();
            // The gloves are shared as the goalkeeping order declares them,
            // where the club has one; the coach's own eye otherwise.
            let declared: Option<Vec<f32>> = reading.keepers.filter(|_| keeper).and_then(|room| {
                let raw: Vec<f32> = dividing
                    .iter()
                    .map(|&i| room.glove_share(men[i].id).unwrap_or(0.0))
                    .collect();
                let total: f32 = raw.iter().sum();
                (total > 0.0).then(|| raw.iter().map(|share| share / total).collect())
            });
            let divided = declared
                .unwrap_or_else(|| ShirtShares::divide(&reading.climate, &candidates, keeper));
            let mut shares: Vec<f32> = sharing
                .iter()
                .map(|i| {
                    dividing
                        .iter()
                        .position(|d| d == i)
                        .map_or(0.0, |at| divided[at])
                })
                .collect();
            let floors: Vec<Option<PlanFloor>> = sharing
                .iter()
                .map(|&i| {
                    let window = for_sale(i)
                        .then(|| Self::shop_window(men[i], &reading.climate, keeper))
                        .flatten()
                        .map(|share| {
                            let demanded =
                                men[i].decision_history.sale_was_demanded_by_the_board();
                            if frozen_out(i) && !demanded {
                                share * reading.board_hold.clamp(0.0, 1.0)
                            } else {
                                share
                            }
                        })
                        .filter(|share| *share > 0.0)
                        .map(|share| PlanFloor {
                            kind: PlanFloorKind::ShopWindow,
                            share,
                        });
                    PlanFloor::of(men[i], reading.coach_id, &reading.climate, today)
                        .into_iter()
                        .chain(window)
                        .max_by(|a, b| a.share.total_cmp(&b.share))
                })
                .collect();
            let unmet = PlanFloor::apply(&mut shares, &floors);

            for (i, player) in men.iter().enumerate() {
                let slot = sharing.iter().position(|s| *s == i);
                let (role, succeeds) = roles[i];
                drafted.insert(
                    player.id,
                    PlayerPlanEntry {
                        role,
                        set_on: today,
                        succeeds,
                        baseline: PlayerUsage::of(player),
                        shirt: Some(depth.slot),
                        place: i.min(u8::MAX as usize) as u8,
                        share: slot.map(|s| shares[s]).unwrap_or(0.0),
                        unmet_floor: slot
                            .filter(|s| unmet.contains(s))
                            .and_then(|s| floors[s].map(|f| f.kind)),
                    },
                );
            }
        }

        // A role that has not changed keeps the day it was set and the
        // ledger opened on it; only what the role is planned for moves.
        let mut changes = Vec::new();
        for (id, mut entry) in drafted {
            match self.entries.get(&id) {
                Some(standing) if standing.role == entry.role => {
                    entry.set_on = standing.set_on;
                    entry.baseline = standing.baseline;
                }
                _ => changes.push((id, entry.role)),
            }
            self.entries.insert(id, entry);
        }
        // Players who have left the squad are no longer the coach's
        // problem — drop them so the plan cannot grow without bound.
        self.entries.retain(|id, _| by_id.contains_key(id));

        self.last_revised = Some(today);
        self.formation = Some(reading.tactics.tactic_type);
        changes.sort_unstable_by_key(|(id, _)| *id);
        changes
    }

    /// The football a man the club is selling is given to be seen in:
    /// up to a rotation player's share, by how far one more appearance
    /// this season still moves his price.
    fn shop_window(player: &Player, climate: &ShareClimate, keeper: bool) -> Option<f32> {
        let seen = FootballRecord::this_season(player).appearances;
        let share = ShirtShares::rotation(climate, keeper)
            * PlayerValueCalculator::football_leverage(seen);
        (share > 0.0).then_some(share)
    }

    /// A keeper's role, read off the goalkeeping department's plan.
    ///
    /// `None` for outfielders, for keepers the department has no view on,
    /// and whenever no department exists — the caller then falls back to
    /// the ordinary depth-chart read.
    ///
    /// The mapping is the point. A squad depth chart can only say "third
    /// best keeper here", which for a settled thirty-five-year-old third
    /// choice reads as a stalled career and used to put him in the shop
    /// window. The department says what he actually is: the cover, and the
    /// senior voice in the goalkeeping group.
    fn keeper_role(player: &Player, keepers: Option<&KeeperRoomPlan>) -> Option<PlannedRole> {
        let plan = keepers?;
        if player.position().position_group() != PlayerFieldPositionGroup::Goalkeeper {
            return None;
        }
        // A manager-pinned keeper is in the plans by definition, and the pin
        // outranks the specialist's opinion.
        if player.is_force_match_selection {
            return Some(PlannedRole::Starter);
        }
        if plan.heir() == Some(player.id) {
            return Some(PlannedRole::SuccessionHeir);
        }
        match plan.tier_of(player.id)? {
            KeeperTier::NumberOne => Some(PlannedRole::Starter),
            KeeperTier::Deputy => Some(PlannedRole::CupKeeper),
            KeeperTier::Third => Some(PlannedRole::Cover),
            KeeperTier::Pathway => Some(PlannedRole::DevelopmentPathway),
            // The department is watching him but has not put him on the
            // pathway, and it has no opinion about a keeper it wants moved
            // on. Both fall back to the ordinary read.
            KeeperTier::Academy | KeeperTier::Surplus => None,
        }
    }

    /// The coach's read of one player from where the chart puts him, and
    /// the incumbent he is groomed for when he is an heir.
    fn derive_role(
        player: &Player,
        chart: &SquadDepthChart,
        squad: &FxHashMap<u32, &Player>,
        today: NaiveDate,
    ) -> (PlannedRole, Option<u32>) {
        let Some(place) = chart.place_of(player.id) else {
            return (PlannedRole::Cover, None);
        };
        let is_goalkeeper = place.slot == PlayerPositionType::Goalkeeper;
        let age = player.age(today);
        let level = AbilityEstimator::observable_level(player);
        let ceiling = PotentialEstimator::observable_ceiling(player, today);

        // A manager-pinned player is in the plans by definition.
        if player.is_force_match_selection {
            return (PlannedRole::Starter, None);
        }

        if place.place > 0 {
            // ── The heir ──
            // A young player the staff believe will reach the level of the
            // ageing man ahead of him for the same shirt is being groomed,
            // not warehoused.
            let permanent_future = player
                .contract_loan
                .as_ref()
                .is_none_or(|loan| loan.loan_future_fee.is_some());
            let incumbent = chart
                .first_choice_ahead_of(player.id)
                .and_then(|(id, _)| squad.get(&id).copied());
            if let Some(incumbent) = incumbent
                && permanent_future
                && age <= Self::HEIR_MAX_AGE
                && incumbent.age(today) >= Self::INCUMBENT_AGEING_FROM
                && ceiling >= AbilityEstimator::observable_level(incumbent)
            {
                return (PlannedRole::SuccessionHeir, Some(incumbent.id));
            }

            // ── The prospect ──
            if ceiling > level.saturating_add(Self::DEVELOPMENT_GROWTH) {
                return (PlannedRole::DevelopmentPathway, None);
            }
        }

        // ── The stalled ──
        // Seasons here without first-team football, past the age where
        // that is a stage rather than a verdict. For a player with nothing
        // left to offer the honest answer is no future at all, and the
        // point of saying so is that the player finds out. Whether he is
        // sold is the club's decision, not the plan's.
        let stalled = !player.is_on_loan()
            && age > 23
            && StuckCareerScan::of(player, today).is_some_and(|scan| scan.stuck_years >= 2);
        if stalled && place.place > 0 {
            // A keeper is a special case: only one man can play, so a
            // settled senior deputy has a real role rather than a
            // stalled career — the cup is his.
            if is_goalkeeper && place.place == 1 && age >= 24 {
                return (PlannedRole::CupKeeper, None);
            }
            if place.standing + Self::NOT_IN_PLANS_GAP < chart.squad_standing() {
                return (PlannedRole::NotInPlans, None);
            }
        }

        // ── The picture at his shirt ──
        let role = match place.place {
            0 if place.standing + Self::CORNERSTONE_GAP >= chart.top_standing() => {
                PlannedRole::Cornerstone
            }
            0 => PlannedRole::Starter,
            // Only one keeper plays, so the second-choice keeper is the
            // cup keeper rather than a rotation option.
            1 if is_goalkeeper => PlannedRole::CupKeeper,
            1 => PlannedRole::Rotation,
            _ => PlannedRole::Cover,
        };
        (role, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::club::board::mandate::{MandateAuthor, MandatePurpose, SigningMandate};
    use crate::club::player::builder::PlayerBuilder;
    use crate::club::player::plan::PlayerPlan;
    use crate::club::staff::coach::memory::CoachMemory;
    use crate::club::staff::coach::standing::{CoachStanding, StandingRung};
    use crate::shared::fullname::FullName;
    use crate::{
        PersonAttributes, PlayerAttributes, PlayerClubContract, PlayerPosition, PlayerPositions,
        PlayerSkills, PlayerSquadStatus, PlayerStatCompetitionKind, PlayerStatLedgerEntry,
        PlayerStatistics, PlayerStatusType,
    };
    use chrono::{Datelike, Duration};
    use PlayerPositionType::*;

    /// Fixtures for the squad-plan derivation.
    struct Fx;

    impl Fx {
        fn date() -> NaiveDate {
            NaiveDate::from_ymd_opt(2026, 7, 1).unwrap()
        }

        fn versatile(id: u32, positions: &[(PlayerPositionType, u8)], ability: u8, age: u8) -> Player {
            let attrs = PlayerAttributes {
                current_ability: ability,
                potential_ability: ability,
                ..Default::default()
            };
            let mut contract =
                PlayerClubContract::new(50_000, NaiveDate::from_ymd_opt(2030, 6, 30).unwrap());
            contract.squad_status = PlayerSquadStatus::MainBackupPlayer;
            PlayerBuilder::new()
                .id(id)
                .full_name(FullName::new("T".into(), format!("P{id}")))
                .birth_date(
                    NaiveDate::from_ymd_opt(Self::date().year() - age as i32, 1, 1).unwrap(),
                )
                .country_id(1)
                .attributes(PersonAttributes::default())
                .skills(PlayerSkills::flat_for_ability(ability))
                .positions(PlayerPositions {
                    positions: positions
                        .iter()
                        .map(|&(position, level)| PlayerPosition { position, level })
                        .collect(),
                })
                .player_attributes(attrs)
                .contract(Some(contract))
                .build()
                .unwrap()
        }

        fn player(id: u32, pos: PlayerPositionType, ability: u8, age: u8) -> Player {
            Self::versatile(id, &[(pos, 18)], ability, age)
        }

        /// One natural for every shirt of the shape, ids from 1 in shirt
        /// order, all aged 27.
        fn eleven(shape: MatchTacticType, ability: u8) -> Vec<Player> {
            Tactics::new(shape)
                .positions()
                .iter()
                .enumerate()
                .map(|(i, &slot)| Self::player(i as u32 + 1, slot, ability, 27))
                .collect()
        }

        fn plan(players: Vec<Player>, shape: MatchTacticType) -> (CoachSquadPlan, PlayerCollection) {
            let squad = PlayerCollection::new(players);
            let mut plan = CoachSquadPlan::new();
            plan.revise(&PlanReading::of(&squad, &Tactics::new(shape), Self::date()));
            (plan, squad)
        }

        fn first_choices(plan: &CoachSquadPlan, squad: &PlayerCollection) -> Vec<u32> {
            let mut ids: Vec<u32> = squad
                .iter()
                .filter(|p| plan.entry(p.id).is_some_and(|e| e.place == 0))
                .map(|p| p.id)
                .collect();
            ids.sort_unstable();
            ids
        }

        /// Give a player N consecutive seasons of bench duty on the
        /// canonical ledger, so the stuck-career scan sees a stalled man.
        fn with_stalled_seasons(mut p: Player, seasons: u16) -> Player {
            for back in 0..seasons {
                p.statistics_history
                    .season_ledger
                    .push(PlayerStatLedgerEntry {
                        seq_id: 0,
                        season_start_year: 2025 - back,
                        team_slug: "t".into(),
                        team_name: "T".into(),
                        team_reputation: 8_000,
                        league_slug: "l".into(),
                        league_name: "L".into(),
                        competition_kind: PlayerStatCompetitionKind::League,
                        competition_slug: "l".into(),
                        is_loan: false,
                        transfer_fee: None,
                        coverage_days: None,
                        spell_end: None,
                        statistics: PlayerStatistics {
                            played: 1,
                            ..Default::default()
                        },
                    });
            }
            p
        }
    }

    #[test]
    fn a_four_four_two_names_eleven_first_choices_whose_shares_add_up_to_eleven() {
        // No two of the eleven are alike, so a rank inside a position
        // group would name one starter a line.
        let mut players: Vec<Player> = Tactics::new(MatchTacticType::T442)
            .positions()
            .iter()
            .enumerate()
            .map(|(i, &slot)| Fx::player(i as u32 + 1, slot, 150 - 2 * i as u8, 27))
            .collect();
        for (i, slot) in [Goalkeeper, DefenderCenterLeft, MidfielderCenterRight, ForwardLeft]
            .into_iter()
            .enumerate()
        {
            players.push(Fx::player(20 + i as u32, slot, 125, 26));
        }
        players.push(Fx::player(30, DefenderCenterRight, 110, 30));
        let (plan, squad) = Fx::plan(players, MatchTacticType::T442);

        assert_eq!(Fx::first_choices(&plan, &squad), (1..=11).collect::<Vec<u32>>());
        for id in 1..=11 {
            assert!(
                plan.role_of(id).unwrap().is_at_least(PlannedRole::Starter),
                "first choice {id} is planned as {:?}",
                plan.role_of(id)
            );
        }
        assert!((plan.total_share() - 11.0).abs() < 1e-3, "{}", plan.total_share());
    }

    #[test]
    fn a_back_four_has_four_first_choice_defenders_and_the_rest_behind() {
        let mut players = Fx::eleven(MatchTacticType::T442, 140);
        players.push(Fx::player(20, DefenderCenterLeft, 130, 26));
        players.push(Fx::player(21, DefenderCenterRight, 120, 26));
        let (plan, _) = Fx::plan(players, MatchTacticType::T442);

        for id in 2..=5 {
            let entry = plan.entry(id).unwrap();
            assert_eq!(entry.place, 0, "defender {id}");
            assert!(entry.role.is_at_least(PlannedRole::Starter), "defender {id}");
            assert!(entry.share > 0.7, "defender {id} is planned for {}", entry.share);
        }
        for id in [20, 21] {
            let entry = plan.entry(id).unwrap();
            assert!(entry.place >= 1 && !entry.role.is_at_least(PlannedRole::Starter));
        }
    }

    #[test]
    fn a_change_of_shape_is_a_new_plan_with_three_first_choice_defenders() {
        let central = [DefenderCenterLeft, DefenderCenter, DefenderCenterRight];
        let back_line = |plan: &CoachSquadPlan, squad: &PlayerCollection| {
            squad
                .iter()
                .filter_map(|p| plan.entry(p.id))
                .filter(|e| e.place == 0)
                .filter(|e| {
                    e.shirt.is_some_and(|s| central.contains(&s) || s == DefenderLeft || s == DefenderRight)
                })
                .count()
        };
        let mut players = Fx::eleven(MatchTacticType::T442, 140);
        players.push(Fx::player(20, DefenderCenter, 135, 26));
        let squad = PlayerCollection::new(players);
        let mut plan = CoachSquadPlan::new();
        plan.revise(&PlanReading::of(&squad, &Tactics::new(MatchTacticType::T442), Fx::date()));
        assert_eq!(back_line(&plan, &squad), 4);

        let three = Tactics::new(MatchTacticType::T352);
        assert!(plan.is_stale(&squad, &three, Fx::date()), "the shape changed today");
        plan.revise(&PlanReading::of(&squad, &three, Fx::date()));
        assert_eq!(back_line(&plan, &squad), 3);
        assert!(!plan.is_stale(&squad, &three, Fx::date()));
    }

    #[test]
    fn the_best_left_back_who_is_third_on_the_wing_is_planned_at_left_back() {
        let mut players = Fx::eleven(MatchTacticType::T442, 130);
        players[1] = Fx::player(2, DefenderLeft, 110, 27);
        players[5] = Fx::player(6, MidfielderLeft, 150, 27);
        players.push(Fx::player(20, MidfielderLeft, 145, 27));
        players.push(Fx::versatile(21, &[(DefenderLeft, 18), (MidfielderLeft, 18)], 140, 27));
        let (plan, _) = Fx::plan(players, MatchTacticType::T442);

        let entry = plan.entry(21).unwrap();
        assert_eq!(entry.shirt, Some(DefenderLeft));
        assert_eq!(entry.place, 0);
        assert_eq!(plan.entry(2).unwrap().place, 1, "the old left-back stands behind him");
    }

    #[test]
    fn a_trusted_man_keeps_his_shirt_until_the_newcomer_has_shown_more() {
        let trusted = |newcomer: u8| {
            let mut players = Fx::eleven(MatchTacticType::T442, 140);
            players.push(Fx::player(20, DefenderCenterLeft, newcomer, 26));
            let squad = PlayerCollection::new(players);
            let mut record = CoachMemory::default();
            record.player_id = 3;
            record.standing = CoachStanding {
                rung: StandingRung::Trusted,
                ..Default::default()
            };
            let mut memory = CoachMemoryStore::new();
            memory.seed(record);
            let mut plan = CoachSquadPlan::new();
            plan.revise(&PlanReading {
                memory: Some(&memory),
                ..PlanReading::of(&squad, &Tactics::new(MatchTacticType::T442), Fx::date())
            });
            (plan.entry(3).unwrap().place, plan.entry(20).unwrap().place)
        };
        assert_eq!(trusted(143), (0, 1), "marginally stronger: the incumbent keeps the shirt");
        assert_eq!(trusted(155), (1, 0), "clearly stronger: the newcomer takes it");
    }

    #[test]
    fn a_rotation_player_left_out_for_a_month_is_owed_more_than_a_match() {
        let mut players = Fx::eleven(MatchTacticType::T442, 140);
        players.push(Fx::player(20, MidfielderCenterLeft, 138, 26));
        let (plan, mut squad) = Fx::plan(players, MatchTacticType::T442);
        let entry = *plan.entry(20).unwrap();
        assert!(entry.share > 0.2, "a close second is planned for {}", entry.share);
        assert!(entry.role == PlannedRole::Rotation);

        let player = squad.players.iter_mut().find(|p| p.id == 20).unwrap();
        for _ in 0..5 {
            player.on_match_overlooked(false);
        }
        assert!(entry.owed(player) > 1.0, "owed {}", entry.owed(player));
    }

    #[test]
    fn a_starter_who_has_played_everything_is_ahead_of_his_share() {
        let mut players = Fx::eleven(MatchTacticType::T442, 140);
        players.push(Fx::player(20, MidfielderCenterLeft, 138, 26));
        let (plan, mut squad) = Fx::plan(players, MatchTacticType::T442);
        let entry = *plan.entry(7).unwrap();

        let player = squad.players.iter_mut().find(|p| p.id == 7).unwrap();
        for _ in 0..8 {
            player.happiness.note_official_appearance(true);
            player.happiness.official_minutes_since_join += 90;
        }
        assert!(entry.owed(player) < 0.0, "owed {}", entry.owed(player));
    }

    #[test]
    fn a_man_injured_for_six_weeks_is_owed_nothing_for_them() {
        let mut players = Fx::eleven(MatchTacticType::T442, 140);
        players.push(Fx::player(20, MidfielderCenterLeft, 138, 26));
        let (plan, mut squad) = Fx::plan(players, MatchTacticType::T442);
        let entry = *plan.entry(7).unwrap();

        let player = squad.players.iter_mut().find(|p| p.id == 7).unwrap();
        player.player_attributes.is_injured = true;
        for _ in 0..6 {
            player.on_match_overlooked(false);
        }
        assert_eq!(entry.owed(player), 0.0);
    }

    #[test]
    fn a_loan_with_agreed_appearances_sets_a_floor_whatever_his_rank() {
        let mut loanee = Fx::player(20, ForwardLeft, 100, 21);
        loanee.contract_loan = Some(
            PlayerClubContract::new_loan(50_000, Fx::date() + Duration::days(280), 9, 90, 1)
                .with_loan_min_appearances(20),
        );
        let mut players = Fx::eleven(MatchTacticType::T442, 140);
        players.push(Fx::player(21, ForwardLeft, 130, 27));
        players.push(loanee);
        let (plan, _) = Fx::plan(players, MatchTacticType::T442);

        let entry = plan.entry(20).unwrap();
        assert!(entry.place >= 1, "the chart puts him behind");
        assert!(
            entry.share >= 0.37,
            "twenty appearances in forty fixtures need more than {}",
            entry.share
        );
        assert_eq!(entry.unmet_floor, None);
    }

    #[test]
    fn a_marquee_signing_is_planned_at_a_starters_share_while_the_mandate_holds() {
        let mut signing = Fx::player(20, MidfielderCenterLeft, 132, 26);
        let mandate = SigningMandate::new(
            MandatePurpose::Starter,
            PlayerFieldPositionGroup::Midfielder,
            26,
            Fx::date() - Duration::days(30),
            MandateAuthor::Board,
        )
        .with_money(20_000_000.0, 2_000_000.0);
        signing.plan = Some(PlayerPlan::from_mandate(mandate, Fx::date() - Duration::days(30)));
        let mut players = Fx::eleven(MatchTacticType::T442, 140);
        players.push(signing);
        let (plan, _) = Fx::plan(players, MatchTacticType::T442);

        let entry = plan.entry(20).unwrap();
        assert_eq!(entry.place, 1, "the manager's chart has him second");
        assert!(entry.share >= 0.8, "the board bought a starter, planned for {}", entry.share);
        assert!(plan.entry(7).unwrap().share < 0.25, "the football comes from his shirt");
        assert!((plan.total_share() - 11.0).abs() < 1e-3);
    }

    #[test]
    fn a_plan_revised_twice_in_a_day_reports_a_changed_role_once() {
        let squad = PlayerCollection::new(Fx::eleven(MatchTacticType::T442, 140));
        let shape = Tactics::new(MatchTacticType::T442);
        let mut plan = CoachSquadPlan::new();
        assert!(plan.is_stale(&squad, &shape, Fx::date()));
        let first = plan.revise(&PlanReading::of(&squad, &shape, Fx::date()));
        assert_eq!(first.len(), 11, "a fresh plan is all new opinions");
        let again = plan.revise(&PlanReading::of(&squad, &shape, Fx::date()));
        assert!(again.is_empty(), "nothing changed, so there is nothing new to tell anyone");
        assert!(!plan.is_stale(&squad, &shape, Fx::date()));
    }

    #[test]
    fn a_join_and_a_leave_each_make_the_plan_stale() {
        let shape = Tactics::new(MatchTacticType::T442);
        let mut players = Fx::eleven(MatchTacticType::T442, 140);
        let (mut plan, squad) = Fx::plan(players.clone(), MatchTacticType::T442);
        assert!(!plan.is_stale(&squad, &shape, Fx::date()));

        players.push(Fx::player(20, ForwardLeft, 150, 25));
        let joined = PlayerCollection::new(players.clone());
        assert!(plan.is_stale(&joined, &shape, Fx::date()));
        plan.revise(&PlanReading::of(&joined, &shape, Fx::date()));
        assert_eq!(plan.entry(20).unwrap().place, 0, "the better man takes the shirt on the day");
        assert_eq!(plan.entry(10).unwrap().place, 1);

        players.retain(|p| p.id != 20);
        let left = PlayerCollection::new(players);
        assert!(plan.is_stale(&left, &shape, Fx::date()));
        plan.revise(&PlanReading::of(&left, &shape, Fx::date()));
        assert!(plan.role_of(20).is_none(), "a departed player is no longer the coach's problem");
        assert_eq!(plan.entry(10).unwrap().place, 0, "and the man behind him moves up");
    }

    /// Only one keeper can play the league, so a settled senior deputy
    /// has a real role rather than a stalled career — and naming it is
    /// what gets him the cup ties he was never given.
    #[test]
    fn the_second_keeper_is_the_cup_keeper() {
        let mut players = Fx::eleven(MatchTacticType::T442, 150);
        players.push(Fx::with_stalled_seasons(Fx::player(20, Goalkeeper, 130, 28), 4));
        let (plan, _) = Fx::plan(players, MatchTacticType::T442);
        assert!(
            plan.role_of(1)
                .is_some_and(|r| matches!(r, PlannedRole::Cornerstone | PlannedRole::Starter)),
            "the first-choice keeper is central to the plan"
        );
        assert_eq!(
            plan.role_of(20),
            Some(PlannedRole::CupKeeper),
            "a senior deputy keeper has a competition of his own, not a stalled career"
        );
        assert!(plan.entry(1).unwrap().share > 0.85, "a goalkeeper shares least");
    }

    /// An outfield player with the same stalled record has no such role —
    /// eleven men play every week, so there is no honest place for him.
    #[test]
    fn a_stalled_player_nobody_is_selling_is_not_in_the_shop_window() {
        let mut players = Fx::eleven(MatchTacticType::T442, 150);
        // Two seasons without football behind a man no better than he is,
        // and the club has not put him on the market.
        players.push(Fx::with_stalled_seasons(
            Fx::player(21, MidfielderCenterLeft, 150, 26),
            2,
        ));
        // The same story for a man the squad has left far behind.
        players.push(Fx::with_stalled_seasons(
            Fx::player(22, MidfielderCenterRight, 110, 27),
            4,
        ));
        let (plan, _) = Fx::plan(players, MatchTacticType::T442);
        assert_ne!(plan.role_of(21), Some(PlannedRole::ShopWindow));
        assert!(plan.entry(21).unwrap().share > 0.0, "he is still planned for");
        assert_eq!(plan.role_of(22), Some(PlannedRole::NotInPlans));
        assert_eq!(plan.entry(22).unwrap().share, 0.0);
        assert!((plan.total_share() - 11.0).abs() < 1e-3);
    }

    /// A 4-4-2 with a second man for the left of central midfield.
    fn squad_with_a_second(second: Player) -> Vec<Player> {
        let mut players = Fx::eleven(MatchTacticType::T442, 150);
        players.push(second);
        players
    }

    fn listed(mut player: Player) -> Player {
        player.statuses.add(Fx::date(), PlayerStatusType::Lst);
        player
    }

    /// `starts` league starts this season at an ordinary rating.
    fn seen(mut player: Player, starts: u16) -> Player {
        player.statistics.played = starts;
        for _ in 0..starts {
            player.statistics.record_match_rating(6.8, 90, true);
        }
        player
    }

    #[test]
    fn a_player_the_club_lists_holds_the_shop_window_until_a_move_is_agreed() {
        let players = squad_with_a_second(listed(Fx::player(20, MidfielderCenterLeft, 146, 27)));
        let (mut plan, mut squad) = Fx::plan(players, MatchTacticType::T442);
        assert_eq!(plan.role_of(20), Some(PlannedRole::ShopWindow));

        // The club accepts a bid: the window closes.
        squad
            .players
            .iter_mut()
            .find(|p| p.id == 20)
            .unwrap()
            .statuses
            .add(Fx::date(), PlayerStatusType::Bid);
        plan.revise(&PlanReading::of(&squad, &Tactics::new(MatchTacticType::T442), Fx::date()));
        assert_ne!(plan.role_of(20), Some(PlannedRole::ShopWindow));
    }

    #[test]
    fn a_regular_listed_before_the_season_keeps_a_real_share_of_the_football() {
        // First choice for his shirt, listed in the summer.
        let squad = |listed_regular: bool| {
            let mut players = Fx::eleven(MatchTacticType::T442, 150);
            if listed_regular {
                let regular = players.iter().position(|p| p.id == 7).unwrap();
                players[regular] = listed(players[regular].clone());
            }
            players.push(Fx::player(20, MidfielderCenterLeft, 146, 26));
            Fx::plan(players, MatchTacticType::T442).0
        };
        let (kept, for_sale) = (squad(false), squad(true));
        let entry = for_sale.entry(7).unwrap();
        assert_eq!(entry.role, PlannedRole::ShopWindow);
        assert_eq!(entry.place, 0, "still the best man for the shirt");
        assert_eq!(
            entry.share,
            kept.entry(7).unwrap().share,
            "the window is a floor under what his place earns him"
        );
        assert!((for_sale.total_share() - 11.0).abs() < 1e-3);
    }

    #[test]
    fn a_man_behind_the_first_choice_is_shown_at_a_rotation_players_share_when_listed() {
        let players = squad_with_a_second(listed(Fx::player(20, MidfielderCenterLeft, 130, 27)));
        let (plan, _) = Fx::plan(players, MatchTacticType::T442);
        let rotation = ShirtShares::rotation(&ShareClimate::ONE_COMPETITION, false);
        let entry = plan.entry(20).unwrap();
        assert!((entry.share - rotation).abs() < 1e-4, "{} against {rotation}", entry.share);
        assert!(entry.share >= 0.15, "a real share: {}", entry.share);
        assert!((plan.total_share() - 11.0).abs() < 1e-3, "the first choice makes room");
    }

    #[test]
    fn the_window_is_sized_by_how_little_he_has_been_seen() {
        let man = |starts: u16| seen(Fx::player(20, MidfielderCenterLeft, 120, 27), starts);
        let share_of = |player: Player| {
            Fx::plan(squad_with_a_second(player), MatchTacticType::T442)
                .0
                .entry(20)
                .unwrap()
                .share
        };
        let earned = share_of(man(28));
        let (unseen, some, all_season) =
            (share_of(listed(man(0))), share_of(listed(man(1))), share_of(listed(man(28))));
        assert!(unseen > some && some > all_season, "{unseen} {some} {all_season}");
        assert_eq!(
            all_season, earned,
            "a man the market has watched all season is given what his place earns him"
        );
    }

    fn frozen_out(player_id: u32) -> CoachMemoryStore {
        let mut record = CoachMemory::default();
        record.player_id = player_id;
        record.standing = CoachStanding {
            rung: StandingRung::FrozenOut,
            ..Default::default()
        };
        let mut memory = CoachMemoryStore::new();
        memory.seed(record);
        memory
    }

    fn window_share(player: Player, memory: &CoachMemoryStore, board_hold: f32) -> f32 {
        let squad = PlayerCollection::new(squad_with_a_second(player));
        let mut plan = CoachSquadPlan::new();
        plan.revise(&PlanReading {
            memory: Some(memory),
            board_hold,
            ..PlanReading::of(&squad, &Tactics::new(MatchTacticType::T442), Fx::date())
        });
        assert_eq!(plan.role_of(20), Some(PlannedRole::ShopWindow));
        plan.entry(20).unwrap().share
    }

    #[test]
    fn a_frozen_out_man_is_shown_as_far_as_the_board_holds_the_manager_to_it() {
        let memory = frozen_out(20);
        let man = || listed(Fx::player(20, MidfielderCenterLeft, 146, 27));
        let rotation = ShirtShares::rotation(&ShareClimate::ONE_COMPETITION, false);

        let secure = window_share(man(), &memory, 0.0);
        let pressed = window_share(man(), &memory, 0.5);
        assert_eq!(secure, 0.0, "a manager sure of his board shows him to nobody");
        assert!((pressed - rotation * 0.5).abs() < 1e-4);

        // The board demands the sale itself.
        let mut demanded = man();
        demanded.decision_history.add(
            Fx::date(),
            "dec_board_forced_sale".to_string(),
            "dec_reason_wage_control".to_string(),
            "dec_decided_board".to_string(),
        );
        let forced = window_share(demanded, &memory, 0.0);
        assert!((forced - rotation).abs() < 1e-4, "the full share, whatever he thinks of him");
    }

    #[test]
    fn a_team_opens_and_closes_the_window_the_day_the_club_decides() {
        use crate::{
            StaffCollection, TeamBuilder, TeamReputation, TeamType, TrainingSchedule,
        };
        use chrono::NaiveTime;

        let mut team = TeamBuilder::new()
            .id(1)
            .league_id(Some(1))
            .club_id(1)
            .name("team".to_string())
            .slug("team".to_string())
            .team_type(TeamType::Main)
            .players(PlayerCollection::new(squad_with_a_second(Fx::player(
                20,
                MidfielderCenterLeft,
                146,
                27,
            ))))
            .staffs(StaffCollection::new(Vec::new()))
            .reputation(TeamReputation::new(100, 100, 100))
            .training_schedule(TrainingSchedule::new(
                NaiveTime::from_hms_opt(10, 0, 0).unwrap(),
                NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
            ))
            .build()
            .unwrap();
        let role = |team: &crate::Team| team.staffs.head_coach().squad_plan.role_of(20);
        team.revise_coach_squad_plan(Fx::date(), None);
        assert_ne!(role(&team), Some(PlannedRole::ShopWindow));

        let next = Fx::date() + Duration::days(1);
        team.players
            .players
            .iter_mut()
            .find(|p| p.id == 20)
            .unwrap()
            .on_listed_by_club("dec_reason_surplus", next);
        team.revise_coach_squad_plan(next, None);
        assert_eq!(role(&team), Some(PlannedRole::ShopWindow));
        let entry = team.staffs.head_coach().squad_plan.entry(20).unwrap();
        assert!(entry.share > 0.0 && entry.shirt.is_some());

        let later = next + Duration::days(1);
        team.players
            .players
            .iter_mut()
            .find(|p| p.id == 20)
            .unwrap()
            .statuses
            .remove(PlayerStatusType::Lst);
        team.revise_coach_squad_plan(later, None);
        assert_ne!(role(&team), Some(PlannedRole::ShopWindow));
    }

    #[test]
    fn a_frozen_out_man_the_club_is_not_selling_is_not_in_the_plans() {
        let squad = PlayerCollection::new(squad_with_a_second(Fx::player(
            20,
            MidfielderCenterLeft,
            146,
            27,
        )));
        let memory = frozen_out(20);
        let mut plan = CoachSquadPlan::new();
        plan.revise(&PlanReading {
            memory: Some(&memory),
            ..PlanReading::of(&squad, &Tactics::new(MatchTacticType::T442), Fx::date())
        });
        assert_eq!(plan.role_of(20), Some(PlannedRole::NotInPlans));
    }

    #[test]
    fn the_gloves_are_shared_as_the_goalkeeping_order_declares_them() {
        use crate::club::staff::goalkeeping::{
            KeeperReviewOutcome, KeeperRoomPlan, KeeperSuccession, KeeperTier,
        };
        let mut players = Fx::eleven(MatchTacticType::T442, 150);
        players.push(Fx::player(20, Goalkeeper, 140, 28));
        players.push(Fx::player(21, Goalkeeper, 110, 19));
        let squad = PlayerCollection::new(players);
        let shape = Tactics::new(MatchTacticType::T442);

        let mut room = KeeperRoomPlan::new();
        room.commit(
            KeeperReviewOutcome {
                tiers: vec![
                    (1, KeeperTier::NumberOne, None),
                    (20, KeeperTier::Deputy, None),
                    (21, KeeperTier::Third, None),
                ],
                number_one: Some(1),
                deputy: Some(20),
                third: Some(21),
                heir: None,
                succession: KeeperSuccession::Settled,
                nominated: None,
                nominee_assurance: None,
                recommendations: Vec::new(),
                authority: 0.8,
                deputy_cup_reach: Some(1.0),
            },
            Fx::date(),
        );
        let mut plan = CoachSquadPlan::new();
        plan.revise(&PlanReading {
            keepers: Some(&room),
            ..PlanReading::of(&squad, &shape, Fx::date())
        });

        for id in [1, 20, 21] {
            let planned = plan.entry(id).unwrap().share;
            let declared = room.glove_share(id).unwrap();
            assert!((planned - declared).abs() < 1e-4, "keeper {id}: {planned} against {declared}");
        }
        assert!(plan.entry(20).unwrap().share > 0.1, "a deputy with the whole cup");
        assert!((plan.total_share() - 11.0).abs() < 1e-3);
    }

    #[test]
    fn an_unchanged_role_keeps_the_day_it_was_set_and_its_ledger() {
        let squad = PlayerCollection::new(Fx::eleven(MatchTacticType::T442, 140));
        let shape = Tactics::new(MatchTacticType::T442);
        let mut plan = CoachSquadPlan::new();
        plan.revise(&PlanReading::of(&squad, &shape, Fx::date()));
        let later = Fx::date() + Duration::days(31);
        assert!(plan.is_stale(&squad, &shape, later), "a month has passed");
        let second = plan.revise(&PlanReading::of(&squad, &shape, later));
        assert!(second.is_empty());
        assert_eq!(plan.entry(4).unwrap().set_on, Fx::date());
    }

    #[test]
    fn a_role_set_by_hand_is_drawn_onto_the_chart_and_stands_until_the_next_revision() {
        let mut players = Fx::eleven(MatchTacticType::T442, 140);
        players.push(Fx::player(20, MidfielderCenterLeft, 138, 26));
        let (mut plan, squad) = Fx::plan(players, MatchTacticType::T442);
        let shape = Tactics::new(MatchTacticType::T442);

        plan.force_role(20, PlannedRole::NotInPlans, Fx::date());
        assert!(plan.is_stale(&squad, &shape, Fx::date()), "the chart has not seen it");
        plan.revise(&PlanReading::of(&squad, &shape, Fx::date()));
        let entry = plan.entry(20).unwrap();
        assert_eq!(entry.role, PlannedRole::NotInPlans);
        assert_eq!(entry.shirt, Some(MidfielderCenterLeft));
        assert_eq!(entry.share, 0.0, "a man with no future holds none of the shirt's football");
        assert!((plan.total_share() - 11.0).abs() < 1e-3);
        assert!(!plan.is_stale(&squad, &shape, Fx::date()));

        plan.revise(&PlanReading::of(&squad, &shape, Fx::date() + Duration::days(31)));
        assert_eq!(plan.role_of(20), Some(PlannedRole::Rotation), "the coach looks again");
    }

    #[test]
    fn nobody_is_planned_for_a_shirt_he_cannot_play() {
        let players: Vec<Player> = (1..=5)
            .map(|id| Fx::player(id, DefenderCenterLeft, 140 - id as u8, 27))
            .collect();
        let (plan, squad) = Fx::plan(players, MatchTacticType::T442);
        let mut shirts = Vec::new();
        for player in squad.iter() {
            let shirt = plan.entry(player.id).unwrap().shirt.unwrap();
            assert!(
                crate::RoleFamiliarity::credit_at(crate::RoleFamiliarity::level_for_slot(
                    &player.positions,
                    shirt
                )) > 0.0,
                "a centre-back is planned at {shirt:?}"
            );
            if !shirts.contains(&shirt) {
                shirts.push(shirt);
            }
        }
        assert!(shirts.len() < 10, "five centre-backs do not fill a 4-4-2");
        assert!(
            (plan.total_share() - shirts.len() as f32).abs() < 1e-3,
            "only the shirts somebody can play carry football: {} over {} shirts",
            plan.total_share(),
            shirts.len()
        );
    }

    #[test]
    fn a_new_manager_inherits_a_squad_not_a_plan() {
        let (mut plan, squad) = Fx::plan(Fx::eleven(MatchTacticType::T442, 140), MatchTacticType::T442);
        assert!(!plan.is_empty());
        plan.clear();
        assert!(plan.is_empty());
        assert!(plan.is_stale(&squad, &Tactics::new(MatchTacticType::T442), Fx::date()));
    }
}
