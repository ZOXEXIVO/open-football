//! The keeper room — every goalkeeper at the club, seen as one group.
//!
//! Nothing in the sim held the club's keepers together. Squad depth was
//! computed per team, the transfer desk read the first team only, and the
//! academy sides were a different world entirely. So the one position where
//! the whole club shares a single shirt was the one position nobody managed
//! as a unit: a forty-year-old could hold the gloves for a decade while two
//! deputies in their prime and a promising boy in the under-eighteens all
//! waited for a season that never came.
//!
//! A real goalkeeping department starts from the opposite end. It counts the
//! keepers it has — first team, reserves, under-twenty-ones, under-eighteens
//! — and asks one question of the group: is there a number one, is there
//! someone who could go in tomorrow without the team dropping, and is there
//! anybody coming. [`KeeperRoom`] is that census, and it reads only what a
//! coach can actually see.

use chrono::NaiveDate;

use crate::club::staff::perception::{
    AbilityEstimator, DevelopmentFormEvidence, PotentialEstimator,
};
use crate::club::team::TeamType;
use crate::utils::DateUtils;
use crate::{Player, PlayerSquadStatus};

/// One goalkeeper as the department sees him.
///
/// Every field is observable: assessed level and ceiling from the
/// perception layer, appearances from the stat ledger, age from the birth
/// date. The hidden ability digits are never read here — a goalkeeping
/// coach watches training and matches like everybody else.
#[derive(Debug, Clone)]
pub struct RoomKeeper {
    pub player_id: u32,
    /// The squad he is rostered with.
    pub team_id: u32,
    pub team_type: TeamType,
    pub age: u8,
    /// Assessed level (1..200) — [`AbilityEstimator::observable_level`].
    pub level: u8,
    /// Assessed ceiling (1..200) — [`PotentialEstimator::observable_ceiling`].
    pub ceiling: u8,
    /// Senior competitive appearances this season (league + domestic cup).
    pub senior_apps: u16,
    /// Appearances in whichever competition actually carries his football —
    /// the academy leagues are friendly-flagged, so a youth keeper's season
    /// lives in the friendly bucket.
    pub development_apps: u16,
    /// Sample-regressed rating over that same bucket.
    pub form: f32,
    pub days_idle: u16,
    pub squad_status: Option<PlayerSquadStatus>,
    pub is_injured: bool,
    /// Days left on his contract, when he has one.
    pub contract_days_left: Option<i64>,
    /// Manager-pinned for the senior XI.
    pub is_pinned: bool,
}

impl RoomKeeper {
    /// A keeper old enough that the club is buying his present, not his
    /// future. Keepers mature late — the band runs past where an outfield
    /// prospect's would end.
    pub const DEVELOPMENT_AGE: u8 = 23;

    /// Read one keeper.
    pub fn read(player: &Player, team_id: u32, team_type: TeamType, today: NaiveDate) -> Self {
        let stats = &player.statistics;
        let cup = &player.cup_statistics;
        RoomKeeper {
            player_id: player.id,
            team_id,
            team_type,
            age: DateUtils::age(player.birth_date, today),
            level: AbilityEstimator::observable_level(player),
            ceiling: PotentialEstimator::observable_ceiling(player, today),
            senior_apps: stats.played + stats.played_subs + cup.played + cup.played_subs,
            development_apps: DevelopmentFormEvidence::games(player),
            form: DevelopmentFormEvidence::regressed_rating(player),
            days_idle: player.player_attributes.days_since_last_match,
            squad_status: player.contract.as_ref().map(|c| c.squad_status.clone()),
            is_injured: player.player_attributes.is_injured,
            contract_days_left: player
                .contract
                .as_ref()
                .map(|c| (c.expiration - today).num_days()),
            is_pinned: player.is_force_match_selection,
        }
    }

    /// He is rostered with a squad that plays senior football.
    pub fn is_senior(&self) -> bool {
        !self.team_type.is_youth()
    }

    /// He is still young enough to be a project rather than a purchase.
    pub fn is_pathway_age(&self) -> bool {
        self.age <= Self::DEVELOPMENT_AGE
    }

    /// Where a keeper sits on his own arc. Keepers peak late and decline
    /// late — a twenty-four-year-old is a project, a thirty-four-year-old is
    /// still a number one, and the two facts have to be held at once.
    /// Answers "what will he be worth in `years` time", never "how good is
    /// he today".
    pub fn projected_level(&self, years: u8) -> f32 {
        let then = self.age.saturating_add(years);
        let now = KeeperAgeCurve::of(self.age);
        let later = KeeperAgeCurve::of(then);
        // Growth toward the ceiling is what the curve buys before the peak;
        // after it, the same curve takes level away.
        let headroom = (self.ceiling as f32 - self.level as f32).max(0.0);
        let gain = headroom * ((later - now).max(0.0) / (1.0 - now).max(0.05));
        let decay = self.level as f32 * ((now - later).max(0.0) / now.max(0.05));
        (self.level as f32 + gain - decay).max(1.0)
    }
}

/// The goalkeeping age curve, normalised to 1.0 across the peak.
///
/// Real keepers arrive late and leave late: a twenty-year-old is raw
/// however good his hands are, the position is learned through the
/// twenties, the peak sits around thirty and holds into the mid-thirties,
/// and the fall after that is slower than at any outfield position. Every
/// age judgement the department makes routes through here rather than
/// through an outfield age band.
pub struct KeeperAgeCurve;

impl KeeperAgeCurve {
    /// First age at which a keeper is plausibly a senior number one.
    pub const SENIOR_FROM: u8 = 21;
    /// The plateau — a keeper is at his best across this whole band.
    pub const PEAK_FROM: u8 = 27;
    pub const PEAK_UNTIL: u8 = 33;
    /// Past here the club should have a successor under contract.
    pub const SUCCESSION_FROM: u8 = 31;
    /// The late-career tail: still selectable, but no longer a plan.
    pub const LATE_CAREER: u8 = 36;

    /// Fraction of his peak a keeper is at `age`.
    pub fn of(age: u8) -> f32 {
        match age {
            0..=16 => 0.42,
            17..=20 => 0.42 + (age - 16) as f32 * 0.055,
            21..=26 => 0.64 + (age - 20) as f32 * 0.060,
            27..=33 => 1.0,
            34..=38 => 1.0 - (age - 33) as f32 * 0.045,
            _ => 0.74,
        }
    }
}

/// Every keeper at the club, ordered as the department orders them.
///
/// Built once per review from whatever squads the caller offers, which is
/// deliberately every squad the club owns — the first team's problem may
/// well be sitting in the under-eighteens.
#[derive(Debug, Clone, Default)]
pub struct KeeperRoom {
    keepers: Vec<RoomKeeper>,
}

impl KeeperRoom {
    /// Assemble the room from `(team_id, team_type, player)` triples.
    /// Non-keepers and keepers away on loan are skipped — a loaned keeper is
    /// somebody else's team sheet this season.
    pub fn assemble<'a, I>(squads: I, today: NaiveDate) -> Self
    where
        I: IntoIterator<Item = (u32, TeamType, &'a Player)>,
    {
        let mut keepers: Vec<RoomKeeper> = squads
            .into_iter()
            .filter(|(_, _, p)| p.positions.is_goalkeeper() && !p.is_on_loan())
            .map(|(team_id, team_type, p)| RoomKeeper::read(p, team_id, team_type, today))
            .collect();
        // Strongest first, with age as the tiebreak so the settled senior
        // sits above the boy on an equal read.
        keepers.sort_by(|a, b| {
            b.level
                .cmp(&a.level)
                .then_with(|| b.age.cmp(&a.age))
                .then_with(|| a.player_id.cmp(&b.player_id))
        });
        KeeperRoom { keepers }
    }

    pub fn is_empty(&self) -> bool {
        self.keepers.is_empty()
    }

    pub fn len(&self) -> usize {
        self.keepers.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = &RoomKeeper> {
        self.keepers.iter()
    }

    pub fn get(&self, player_id: u32) -> Option<&RoomKeeper> {
        self.keepers.iter().find(|k| k.player_id == player_id)
    }

    /// Keepers rostered with a squad that plays senior football, strongest
    /// first.
    pub fn seniors(&self) -> impl Iterator<Item = &RoomKeeper> {
        self.keepers.iter().filter(|k| k.is_senior())
    }

    /// Keepers still in the academy pathway, strongest first.
    pub fn pathway(&self) -> impl Iterator<Item = &RoomKeeper> {
        self.keepers.iter().filter(|k| k.team_type.is_youth())
    }

    /// The strongest keeper in the building, whatever squad he trains with.
    pub fn best(&self) -> Option<&RoomKeeper> {
        self.keepers.first()
    }

    /// Assessed level of the `index`-th best senior keeper, when the room is
    /// that deep. The bar an academy keeper is measured against is this one
    /// — the third choice, not the number one.
    pub fn senior_level_at(&self, index: usize) -> Option<u8> {
        self.seniors().nth(index).map(|k| k.level)
    }

    /// Whether the room carries a senior keeper inside an age band.
    pub fn has_senior_aged(&self, from: u8, to: u8) -> bool {
        self.seniors().any(|k| k.age >= from && k.age <= to)
    }
}
