//! What a coach keeps about a player once they have parted.
//!
//! The hot stores — [`CoachMemory`], the impression, the plan entry — are
//! about a man he is picking this Saturday, and they are discarded when the
//! spell ends, because a plan for a player who has left is not an opinion,
//! it is a leak. What survives is this: how long we were together, what I
//! concluded he was, how far I trusted him, what he did to me and for me,
//! how it ended, and how warm I am about him now.
//!
//! Deliberately small and `Copy` — a coach holds up to
//! [`DossierTuning::CAPACITY`] of these and a `Staff` is cloned constantly.
//! `u8` percentages throughout, and the mind's own [`EpochDay`] clock rather
//! than a `NaiveDate`, for the same reasons [`PlayerJudgement`] gives.
//!
//! [`CoachMemory`]: crate::club::staff::coach::CoachMemory
//! [`PlayerJudgement`]: crate::club::staff::mind::PlayerJudgement

use super::tuning::DossierTuning;
use crate::club::mind::organs::memory::{EpochDay, MindClock};
use crate::club::staff::coach::plan::PlannedRole;

/// How a spell ended. The last thing that happened between two people, and
/// the thing that colours everything before it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum SeparationCause {
    /// Placeholder while the spell is open.
    #[default]
    Ongoing = 0,
    /// The club sacked me.
    IWasSacked = 1,
    /// I took another job.
    IMovedOn = 2,
    /// I stopped.
    IRetired = 3,
    /// Somebody else took the dugout while I stayed at the club.
    SeatLost = 4,
    /// The board took the money. Not his fault, and not mine.
    SoldByBoard = 5,
    /// I wanted him gone and he went.
    SoldOnMyCall = 6,
    /// I let him go for nothing.
    ReleasedOnMyCall = 7,
    /// The club let him go for nothing.
    ReleasedByBoard = 8,
    /// He asked to leave.
    HeRequestedOut = 9,
    /// He played out his contract and walked.
    HeRanDownHisContract = 10,
    /// His loan with me finished.
    LoanEnded = 11,
    /// I sent him out to play somewhere else. A suspension, not an ending.
    LoanedOutByMe = 12,
    /// He stopped playing.
    HeRetired = 13,
}

impl SeparationCause {
    /// True when the coach himself left rather than the player.
    #[inline]
    pub fn is_my_departure(self) -> bool {
        matches!(
            self,
            SeparationCause::IWasSacked
                | SeparationCause::IMovedOn
                | SeparationCause::IRetired
                | SeparationCause::SeatLost
        )
    }

    /// True when the player chose to go.
    #[inline]
    pub fn is_his_choice(self) -> bool {
        matches!(
            self,
            SeparationCause::HeRequestedOut | SeparationCause::HeRanDownHisContract
        )
    }

    /// Signed contribution to how warmly the coach remembers him. `standing`
    /// is the coach's final read of the player, which is what separates a
    /// squad player taking his chance from a man he rated walking out.
    pub fn warmth_term(self, standing: f32) -> f32 {
        match self {
            SeparationCause::SoldByBoard => DossierTuning::PARTING_SOLD_BY_BOARD,
            SeparationCause::SoldOnMyCall => DossierTuning::PARTING_SOLD_ON_MY_CALL,
            SeparationCause::ReleasedOnMyCall => DossierTuning::PARTING_RELEASED_ON_MY_CALL,
            SeparationCause::ReleasedByBoard => DossierTuning::PARTING_RELEASED_BY_BOARD,
            SeparationCause::HeRequestedOut => {
                if standing >= DossierTuning::WALKED_OUT_STANDING {
                    DossierTuning::PARTING_HE_WALKED_OUT_ON_ME
                } else {
                    DossierTuning::PARTING_HE_REQUESTED_OUT
                }
            }
            SeparationCause::HeRanDownHisContract => {
                DossierTuning::PARTING_HE_RAN_DOWN_HIS_CONTRACT
            }
            SeparationCause::HeRetired => DossierTuning::PARTING_HE_RETIRED,
            _ => 0.0,
        }
    }

    pub fn as_i18n_key(self) -> &'static str {
        match self {
            SeparationCause::Ongoing => "parted_ongoing",
            SeparationCause::IWasSacked => "parted_i_was_sacked",
            SeparationCause::IMovedOn => "parted_i_moved_on",
            SeparationCause::IRetired => "parted_i_retired",
            SeparationCause::SeatLost => "parted_seat_lost",
            SeparationCause::SoldByBoard => "parted_sold_by_board",
            SeparationCause::SoldOnMyCall => "parted_sold_on_my_call",
            SeparationCause::ReleasedOnMyCall => "parted_released_on_my_call",
            SeparationCause::ReleasedByBoard => "parted_released_by_board",
            SeparationCause::HeRequestedOut => "parted_he_requested_out",
            SeparationCause::HeRanDownHisContract => "parted_he_ran_down_his_contract",
            SeparationCause::LoanEnded => "parted_loan_ended",
            SeparationCause::LoanedOutByMe => "parted_loaned_out_by_me",
            SeparationCause::HeRetired => "parted_he_retired",
        }
    }
}

/// Things he did to me. Each is a sentence a real manager says about a
/// player years later, which is the test for whether a flag belongs here.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ScarFlags(u16);

impl ScarFlags {
    /// Twice out of the last four big matches, he was not there.
    pub const BIG_MATCH_FLOP: u16 = 1 << 0;
    /// A red card or a decisive error on the one night that counted.
    /// Never forgiven down to nothing.
    pub const COST_US_THE_OCCASION: u16 = 1 << 1;
    /// Three mistakes that led to goals inside a short spell.
    pub const ERROR_PRONE: u16 = 1 << 2;
    /// Cards, fines, a dressing room he was a problem in.
    pub const INDISCIPLINE: u16 = 1 << 3;
    /// He would not play for me. Never forgiven down to nothing.
    pub const REFUSED_TO_PLAY: u16 = 1 << 4;
    /// He asked out while I was picking him.
    pub const WANTED_OUT: u16 = 1 << 5;
    /// He took it to the press.
    pub const WENT_PUBLIC: u16 = 1 << 6;
    /// I could never count on him being fit.
    pub const FRAGILE: u16 = 1 << 7;
    /// He did not work.
    pub const TRAINING_SLACKER: u16 = 1 << 8;
    /// It got so bad I stopped picking him at all — which is a mark on
    /// both of us.
    pub const I_FROZE_HIM_OUT: u16 = 1 << 9;
    /// I backed him and he let me down.
    pub const LET_ME_DOWN: u16 = 1 << 10;

    /// Scars that never fade past [`DossierTuning::SCAR_PROTECTED_FLOOR`].
    pub const PROTECTED: u16 = Self::COST_US_THE_OCCASION | Self::REFUSED_TO_PLAY;

    /// Scars that make a coach refuse to work with the man again rather
    /// than merely rate him lower.
    pub const POISONOUS: u16 = Self::REFUSED_TO_PLAY | Self::WENT_PUBLIC | Self::LET_ME_DOWN;

    #[inline]
    pub fn contains(self, flag: u16) -> bool {
        self.0 & flag != 0
    }

    #[inline]
    pub fn insert(&mut self, flag: u16) {
        self.0 |= flag;
    }

    #[inline]
    pub fn remove(&mut self, flag: u16) {
        self.0 &= !flag;
    }

    #[inline]
    pub fn bits(self) -> u16 {
        self.0
    }

    #[inline]
    pub fn count(self) -> u32 {
        self.0.count_ones()
    }

    #[inline]
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// How heavily each scar weighs before time is applied. The ones that
    /// end relationships weigh most.
    pub fn weight(self) -> f32 {
        let mut total = 0.0;
        for (flag, weight) in [
            (Self::COST_US_THE_OCCASION, 0.60),
            (Self::REFUSED_TO_PLAY, 0.60),
            (Self::WENT_PUBLIC, 0.50),
            (Self::LET_ME_DOWN, 0.50),
            (Self::WANTED_OUT, 0.35),
            (Self::BIG_MATCH_FLOP, 0.35),
            (Self::INDISCIPLINE, 0.30),
            (Self::I_FROZE_HIM_OUT, 0.25),
            (Self::ERROR_PRONE, 0.20),
            (Self::FRAGILE, 0.15),
            (Self::TRAINING_SLACKER, 0.15),
        ] {
            if self.contains(flag) {
                total += weight;
            }
        }
        total
    }

    /// Every scar held, for a renderer.
    pub fn held(self) -> impl Iterator<Item = (u16, &'static str)> {
        Self::CATALOG
            .into_iter()
            .filter(move |(flag, _)| self.contains(*flag))
    }

    const CATALOG: [(u16, &'static str); 11] = [
        (Self::BIG_MATCH_FLOP, "scar_big_match_flop"),
        (Self::COST_US_THE_OCCASION, "scar_cost_us_the_occasion"),
        (Self::ERROR_PRONE, "scar_error_prone"),
        (Self::INDISCIPLINE, "scar_indiscipline"),
        (Self::REFUSED_TO_PLAY, "scar_refused_to_play"),
        (Self::WANTED_OUT, "scar_wanted_out"),
        (Self::WENT_PUBLIC, "scar_went_public"),
        (Self::FRAGILE, "scar_fragile"),
        (Self::TRAINING_SLACKER, "scar_training_slacker"),
        (Self::I_FROZE_HIM_OUT, "scar_i_froze_him_out"),
        (Self::LET_ME_DOWN, "scar_let_me_down"),
    ];
}

/// Things he did for me.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MedalFlags(u16);

impl MedalFlags {
    /// I picked him against the evidence and he proved me right.
    pub const REPAID_FAITH: u16 = 1 << 0;
    /// On the nights that counted, he was there.
    pub const BIG_MATCH_PLAYER: u16 = 1 << 1;
    /// He wore my armband.
    pub const MY_CAPTAIN: u16 = 1 << 2;
    /// I built the side around him.
    pub const CORNERSTONE: u16 = 1 << 3;
    /// I signed him.
    pub const MY_SIGNING: u16 = 1 << 4;
    /// He became a player under me.
    pub const MADE_UNDER_ME: u16 = 1 << 5;
    /// A long spell and not one thing to hold against him.
    pub const NEVER_LET_ME_DOWN: u16 = 1 << 6;
    /// He came with me.
    pub const FOLLOWED_ME: u16 = 1 << 7;
    /// He had somewhere else to go and stayed.
    pub const STAYED_FOR_ME: u16 = 1 << 8;
    /// He was out of the side and played his way back in.
    pub const BOUNCED_BACK: u16 = 1 << 9;

    /// Medals worth keeping a dossier alive for.
    pub const TREASURED: u16 = Self::REPAID_FAITH | Self::MY_CAPTAIN | Self::CORNERSTONE;

    #[inline]
    pub fn contains(self, flag: u16) -> bool {
        self.0 & flag != 0
    }

    #[inline]
    pub fn insert(&mut self, flag: u16) {
        self.0 |= flag;
    }

    #[inline]
    pub fn remove(&mut self, flag: u16) {
        self.0 &= !flag;
    }

    #[inline]
    pub fn bits(self) -> u16 {
        self.0
    }

    #[inline]
    pub fn count(self) -> u32 {
        self.0.count_ones()
    }

    #[inline]
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub fn held(self) -> impl Iterator<Item = (u16, &'static str)> {
        Self::CATALOG
            .into_iter()
            .filter(move |(flag, _)| self.contains(*flag))
    }

    const CATALOG: [(u16, &'static str); 10] = [
        (Self::REPAID_FAITH, "medal_repaid_faith"),
        (Self::BIG_MATCH_PLAYER, "medal_big_match_player"),
        (Self::MY_CAPTAIN, "medal_my_captain"),
        (Self::CORNERSTONE, "medal_cornerstone"),
        (Self::MY_SIGNING, "medal_my_signing"),
        (Self::MADE_UNDER_ME, "medal_made_under_me"),
        (Self::NEVER_LET_ME_DOWN, "medal_never_let_me_down"),
        (Self::FOLLOWED_ME, "medal_followed_me"),
        (Self::STAYED_FOR_ME, "medal_stayed_for_me"),
        (Self::BOUNCED_BACK, "medal_bounced_back"),
    ];
}

/// One coach's lasting record of one player.
#[derive(Debug, Clone, Copy, Default)]
pub struct PlayerDossier {
    pub player_id: u32,
    /// Where we last worked together.
    pub last_club: u32,
    /// How many separate times we have been at the same club.
    pub spells: u8,
    /// Matches of his I have watched, across every spell.
    pub matches_together: u16,
    pub first_met: EpochDay,
    /// When the last spell closed. Meaningless while [`Self::open`].
    pub last_parted: EpochDay,
    pub age_at_parting: u8,

    level_pct: u8,
    ceiling_pct: u8,
    long_form_tenths: u8,
    tactical_trust_pct: u8,
    big_match_trust_pct: u8,
    training_trust_pct: u8,
    professionalism_pct: u8,
    /// Where he stood with me when we parted, −100..=100.
    standing_pct: i8,
    /// How I feel about him, −100..=100. The number a reunion turns on.
    warmth_pct: i8,
    /// How I read that *he* felt about *me*, −100..=100. A guess, and it
    /// can be wrong.
    his_stance_pct: i8,

    /// Highest role he ever held under me, as `rank + 1`. Zero means he
    /// never had a declared role — which is a real state, not a missing
    /// one: plenty of players pass through a squad without the manager
    /// ever committing to what they are.
    peak_role: u8,
    /// The role he held when we parted, same encoding.
    last_role: u8,
    pub promises_kept: u8,
    pub promises_broken: u8,

    pub parted: SeparationCause,
    pub scars: ScarFlags,
    pub medals: MedalFlags,
    scar_strength_pct: u8,
    /// A spell with him is running right now.
    pub open: bool,
}

impl PlayerDossier {
    /// A dossier opened the day a coach and a player start working together.
    pub fn opened(player_id: u32, club_id: u32, today: EpochDay) -> Self {
        PlayerDossier {
            player_id,
            last_club: club_id,
            spells: 1,
            first_met: today,
            last_parted: today,
            open: true,
            ..PlayerDossier::default()
        }
    }

    #[inline]
    fn to_pct(value: f32) -> u8 {
        (value.clamp(0.0, 1.0) * 100.0).round() as u8
    }

    #[inline]
    fn to_signed_pct(value: f32) -> i8 {
        (value.clamp(-1.0, 1.0) * 100.0).round() as i8
    }

    #[inline]
    pub fn level(&self) -> f32 {
        self.level_pct as f32 / 100.0
    }

    #[inline]
    pub fn ceiling(&self) -> f32 {
        self.ceiling_pct as f32 / 100.0
    }

    #[inline]
    pub fn long_form(&self) -> f32 {
        self.long_form_tenths as f32 / 10.0
    }

    #[inline]
    pub fn tactical_trust(&self) -> f32 {
        self.tactical_trust_pct as f32 / 100.0
    }

    #[inline]
    pub fn big_match_trust(&self) -> f32 {
        self.big_match_trust_pct as f32 / 100.0
    }

    #[inline]
    pub fn training_trust(&self) -> f32 {
        self.training_trust_pct as f32 / 100.0
    }

    #[inline]
    pub fn professionalism(&self) -> f32 {
        self.professionalism_pct as f32 / 100.0
    }

    #[inline]
    pub fn standing(&self) -> f32 {
        self.standing_pct as f32 / 100.0
    }

    /// How warmly he remembers him, as it stood at parting.
    #[inline]
    pub fn warmth(&self) -> f32 {
        self.warmth_pct as f32 / 100.0
    }

    #[inline]
    pub fn his_stance(&self) -> f32 {
        self.his_stance_pct as f32 / 100.0
    }

    /// The most central role he ever held under this coach. `None` when the
    /// coach never declared one.
    #[inline]
    pub fn peak_role(&self) -> Option<PlannedRole> {
        (self.peak_role > 0).then(|| PlannedRole::from_u8(self.peak_role - 1))
    }

    /// The role he held when they parted.
    #[inline]
    pub fn last_role(&self) -> Option<PlannedRole> {
        (self.last_role > 0).then(|| PlannedRole::from_u8(self.last_role - 1))
    }

    /// Raw scar weight as it stood at parting, before time.
    #[inline]
    pub fn scar_strength(&self) -> f32 {
        self.scar_strength_pct as f32 / 100.0
    }

    /// Years since the last spell closed.
    ///
    /// Deliberately still counted while a spell is open, because the one
    /// caller that must not see it — [`Self::significance`], which never
    /// evicts a live record — short-circuits before it gets here, and the
    /// ones that must are the reunion reads, which run at the moment a
    /// spell reopens and need the gap that was just bridged.
    pub fn years_apart(&self, today: EpochDay) -> f32 {
        MindClock::elapsed_f32(self.last_parted, today) / 365.0
    }

    /// What the grievance still weighs. Protected scars keep a floor; the
    /// rest go a quarter a year.
    pub fn scar_now(&self, today: EpochDay) -> f32 {
        let raw = self.scar_strength();
        if raw <= 0.0 {
            return 0.0;
        }
        let years = self.years_apart(today);
        let faded = raw * DossierTuning::SCAR_DECAY_PER_YEAR.powf(years);
        if self.scars.contains(ScarFlags::PROTECTED) {
            faded.max(raw * DossierTuning::SCAR_PROTECTED_FLOOR)
        } else {
            faded
        }
    }

    /// How warmly he feels about him *today*. Warmth outlives detail.
    pub fn warmth_now(&self, today: EpochDay) -> f32 {
        let years = self.years_apart(today);
        let time = (-years / DossierTuning::TAU_REUNION_YEARS).exp();
        self.warmth()
            * (DossierTuning::WARMTH_FADE_FLOOR
                + (1.0 - DossierTuning::WARMTH_FADE_FLOOR) * time)
    }

    /// How much of a full record would be lost by dropping him. Years
    /// together dominate; a protected mark is never traded away.
    pub fn significance(&self, today: EpochDay) -> f32 {
        if self.open {
            // A spell in progress is never evicted — the hot stores point
            // at it.
            return f32::INFINITY;
        }
        let depth = (self.matches_together as f32 / DossierTuning::SIGNIFICANCE_MATCHES_FULL)
            .clamp(0.0, 1.0);
        let warmth = self.warmth().abs();
        let marks = ((self.scars.count() + self.medals.count()) as f32
            / DossierTuning::SIGNIFICANCE_MARKS_FULL)
            .clamp(0.0, 1.0);
        let recency =
            (-self.years_apart(today) / DossierTuning::SIGNIFICANCE_RECENCY_TAU_YEARS).exp();

        let mut score = depth * DossierTuning::SIGNIFICANCE_W_MATCHES
            + warmth * DossierTuning::SIGNIFICANCE_W_WARMTH
            + marks * DossierTuning::SIGNIFICANCE_W_MARKS
            + recency * DossierTuning::SIGNIFICANCE_W_RECENCY;

        if self.scars.contains(ScarFlags::PROTECTED) || self.medals.contains(MedalFlags::TREASURED)
        {
            score += DossierTuning::SIGNIFICANCE_PROTECTED_BONUS;
        }
        score
    }

    // ── Writes, all through the closer ──────────────────────────

    pub fn set_reads(
        &mut self,
        level: f32,
        ceiling: f32,
        long_form: f32,
        tactical: f32,
        big_match: f32,
        training: f32,
        professionalism: f32,
    ) {
        self.level_pct = Self::to_pct(level);
        self.ceiling_pct = Self::to_pct(ceiling).max(self.level_pct);
        self.long_form_tenths = (long_form.clamp(0.0, 10.0) * 10.0).round() as u8;
        self.tactical_trust_pct = Self::to_pct(tactical);
        self.big_match_trust_pct = Self::to_pct(big_match);
        self.training_trust_pct = Self::to_pct(training);
        self.professionalism_pct = Self::to_pct(professionalism);
    }

    pub fn set_standing(&mut self, standing: f32) {
        self.standing_pct = Self::to_signed_pct(standing);
    }

    pub fn set_warmth(&mut self, warmth: f32) {
        self.warmth_pct = Self::to_signed_pct(warmth);
    }

    pub fn set_his_stance(&mut self, stance: f32) {
        self.his_stance_pct = Self::to_signed_pct(stance);
    }

    /// Record a role he held. The peak only ever climbs — a man who was a
    /// cornerstone and finished as cover was still a cornerstone.
    pub fn note_role(&mut self, role: PlannedRole) {
        // `PlannedRole` orders most-central first, so a *lower* rank is a
        // higher role; the stored value is `rank + 1` so that zero can mean
        // "no role declared".
        let rank = role.as_u8() + 1;
        self.last_role = rank;
        self.peak_role = if self.peak_role == 0 {
            rank
        } else {
            self.peak_role.min(rank)
        };
    }

    pub fn add_matches(&mut self, matches: u16) {
        self.matches_together = self.matches_together.saturating_add(matches);
    }

    /// Recompute the stored scar weight from the flags currently held.
    pub fn refresh_scar_strength(&mut self, temperament: f32) {
        let weighted = self.scars.weight() * DossierTuning::temperament_scale(temperament);
        self.scar_strength_pct = Self::to_pct(weighted);
    }

    /// Halve what the grievance weighs — a cleared-up misunderstanding, or a
    /// public record that says the coach was wrong about him.
    pub fn soften_scars(&mut self, factor: f32) {
        let softened = self.scar_strength() * factor.clamp(0.0, 1.0);
        self.scar_strength_pct = Self::to_pct(softened);
    }

    /// Close the spell.
    pub fn close(&mut self, cause: SeparationCause, club_id: u32, age: u8, today: EpochDay) {
        self.parted = cause;
        self.last_club = club_id;
        self.age_at_parting = age;
        self.last_parted = today;
        self.open = false;
    }

    /// Re-open it — the two of them are working together again.
    ///
    /// `last_parted` is deliberately left alone. It is what the reunion
    /// reads measure the gap from, and they run *after* this: overwriting
    /// it here would tell every one of them that no time had passed.
    pub fn reopen(&mut self, club_id: u32, _today: EpochDay) {
        self.spells = self.spells.saturating_add(1);
        self.last_club = club_id;
        self.open = true;
        self.parted = SeparationCause::Ongoing;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TODAY: EpochDay = 10_000;
    const YEAR: EpochDay = 365;

    /// Fixture builders, grouped so the tests read as sentences.
    struct Fx;

    impl Fx {
        fn parted(scars: u16, medals: u16, warmth: f32, years_ago: u16) -> PlayerDossier {
            let mut dossier = PlayerDossier::opened(9, 3, TODAY - years_ago * YEAR);
            dossier.scars = ScarFlags(scars);
            dossier.medals = MedalFlags(medals);
            dossier.set_warmth(warmth);
            dossier.refresh_scar_strength(10.0);
            dossier.close(
                SeparationCause::IMovedOn,
                3,
                28,
                TODAY - years_ago * YEAR,
            );
            dossier
        }
    }

    #[test]
    fn a_dossier_stays_inside_its_budget() {
        assert!(
            size_of::<PlayerDossier>() <= 48,
            "PlayerDossier grew to {} bytes",
            size_of::<PlayerDossier>()
        );
    }

    #[test]
    fn an_ordinary_grievance_fades_and_a_red_card_in_a_final_does_not() {
        let ordinary = Fx::parted(ScarFlags::ERROR_PRONE, 0, 0.0, 10);
        let final_night = Fx::parted(ScarFlags::COST_US_THE_OCCASION, 0, 0.0, 10);

        assert!(
            ordinary.scar_now(TODAY) < 0.02,
            "ten years should bury a run of mistakes: {}",
            ordinary.scar_now(TODAY)
        );
        assert!(
            final_night.scar_now(TODAY) > 0.15,
            "a red card in a final is still a red card in a final: {}",
            final_night.scar_now(TODAY)
        );
    }

    #[test]
    fn warmth_outlives_the_detail() {
        let warm = Fx::parted(0, MedalFlags::CORNERSTONE, 0.8, 6);
        // Six years is 0.22 of the time term; warmth keeps its floor.
        assert!(
            warm.warmth_now(TODAY) > 0.5,
            "he still likes him: {}",
            warm.warmth_now(TODAY)
        );
        assert!(warm.warmth_now(TODAY) < warm.warmth());
    }

    #[test]
    fn a_protected_mark_outranks_a_passing_acquaintance() {
        let mut passing = Fx::parted(0, 0, 0.1, 1);
        passing.add_matches(4);
        let mut captain = Fx::parted(0, MedalFlags::MY_CAPTAIN, 0.4, 9);
        captain.add_matches(4);

        assert!(
            captain.significance(TODAY) > passing.significance(TODAY),
            "captain={} passing={}",
            captain.significance(TODAY),
            passing.significance(TODAY)
        );
    }

    #[test]
    fn significance_prefers_years_together_over_a_single_bad_night() {
        let mut servant = Fx::parted(0, 0, 0.2, 2);
        servant.add_matches(120);
        let mut one_night = Fx::parted(ScarFlags::BIG_MATCH_FLOP, 0, -0.2, 2);
        one_night.add_matches(3);

        assert!(
            servant.significance(TODAY) > one_night.significance(TODAY),
            "servant={} one_night={}",
            servant.significance(TODAY),
            one_night.significance(TODAY)
        );
    }

    #[test]
    fn an_open_spell_is_never_evicted() {
        let open = PlayerDossier::opened(1, 2, TODAY);
        assert!(open.significance(TODAY).is_infinite());
    }

    #[test]
    fn a_man_he_rated_walking_out_stings_more_than_a_squad_player_leaving() {
        assert!(
            SeparationCause::HeRequestedOut.warmth_term(0.5)
                < SeparationCause::HeRequestedOut.warmth_term(0.0)
        );
    }

    #[test]
    fn the_peak_role_only_ever_climbs() {
        let mut dossier = PlayerDossier::opened(1, 2, TODAY);
        assert_eq!(dossier.peak_role(), None, "no role declared yet");
        dossier.note_role(PlannedRole::Cover);
        dossier.note_role(PlannedRole::Starter);
        dossier.note_role(PlannedRole::NotInPlans);
        assert_eq!(dossier.peak_role(), Some(PlannedRole::Starter));
        assert_eq!(dossier.last_role(), Some(PlannedRole::NotInPlans));
    }
}
