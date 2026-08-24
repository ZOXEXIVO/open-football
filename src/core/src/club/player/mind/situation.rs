//! Where the player actually is — the read-only picture the sub-minds
//! reason over.
//!
//! Gathered once per tick by the caller, in the same spirit as
//! `TransferDesireContext`: the mind never walks the simulator world,
//! and a sub-mind never reaches back into `Player` for a field. That
//! keeps every faculty unit-testable against a plain struct and stops
//! the reflect rules quietly growing dependencies on the whole
//! simulation.
//!
//! Deliberately thin. Anything a sub-mind can work out from what he
//! remembers or what he wants belongs in the organs, not here — this is
//! only the ground truth he cannot know from the inside.
//!
//! **Every new field must read as "no view" at its default**, not as bad
//! news. A neutral situation reaching a faculty is the trap the phase-4
//! log records: `CompetitiveMind` reads a neutral one as a man getting
//! the minutes his role implies, and quietly satisfies wants the caller
//! knew nothing about.

use super::organs::memory::ActorRef;

/// How close a player is to his country's side.
///
/// Held as a band rather than a cap count because the difference that
/// matters is not how many he has — it is whether the next squad is
/// something he is defending, chasing, or has no business thinking about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NationalStanding {
    /// Not on anybody's list.
    #[default]
    Unknown,
    /// Has been called up, but is not in the current thinking.
    Fringe,
    /// In recent squads without being certain of them.
    InContention,
    /// A regular. His place is his to lose.
    Established,
}

impl NationalStanding {
    /// How much a tournament on the horizon actually presses on him,
    /// 0..1. A man with no international standing at all feels nothing;
    /// the one on the edge of the squad feels it most, because he is the
    /// one whose club football decides it. An established international
    /// feels it less than the man chasing him — he is going anyway,
    /// unless he stops playing altogether.
    pub fn tournament_stake(self) -> f32 {
        match self {
            NationalStanding::Unknown => 0.0,
            NationalStanding::Fringe => 0.65,
            NationalStanding::InContention => 1.0,
            NationalStanding::Established => 0.55,
        }
    }

    #[inline]
    pub fn is_international(self) -> bool {
        !matches!(self, NationalStanding::Unknown)
    }
}

/// The player's objective circumstances this tick.
#[derive(Debug, Clone, Copy)]
pub struct MindSituation {
    pub age: u8,
    /// Personality, 0–20.
    pub ambition: f32,
    pub pressure: f32,
    pub adaptability: f32,
    /// Whether he is the sort of man who stays. Reaches the faculties
    /// rather than only the memory organ, because it is what decides
    /// whether long service becomes an anchor or an itch.
    pub loyalty: f32,
    /// Whether he does the work. Decides whether a man who has lost his
    /// place fights for it or sulks about it.
    pub professionalism: f32,
    /// How hot his head is. Decides how quickly a grievance becomes
    /// something he says out loud.
    pub temperament: f32,
    /// Rolling share of recent competitive matches started, 0..1.
    /// Neutral 0.5 before he has played enough for it to mean anything.
    pub starter_ratio: f32,
    /// Competitive appearances since his last goal. Saturates.
    pub apps_since_goal: u8,
    /// Days left on his deal. 0 when he has none.
    pub contract_days_left: u16,
    /// Days since he joined. 0 when unknown.
    pub days_at_club: u16,
    /// Share of matches his squad role implies he should start, 0..1.
    ///
    /// Carried as the derived number rather than the status itself, for
    /// two reasons: `PlayerSquadStatus` is not `Copy` (and the whole
    /// situation is), and more importantly the table that turns a role
    /// into an expectation already exists at
    /// [`PlayingTimeFrustrationConfig::expected_start_share`]. Storing
    /// the answer keeps one source of truth instead of a second copy
    /// that can drift.
    ///
    /// [`PlayingTimeFrustrationConfig::expected_start_share`]: crate::club::player::happiness::PlayingTimeFrustrationConfig::expected_start_share
    pub expected_start_share: f32,
    /// Who the manager is, so the professional mind can hold a read of a
    /// specific person rather than of "the manager" in the abstract —
    /// and notice when it becomes somebody else.
    pub manager: ActorRef,
    /// Club reputation, 0..1.
    pub club_reputation: f32,
    /// Is he playing abroad?
    pub is_abroad: bool,
    /// Does he speak the local language?
    pub speaks_local_language: bool,
    /// Compatriots and shared-language teammates in the squad.
    pub familiar_teammates: u8,
    pub is_on_loan: bool,

    // ── Where he stands in this dressing room ───────────────────
    //
    // Written by the team's weekly standing pass. All of it reads as
    // "no view" at zero, because a squad that has not been ranked yet
    // must not look to him like a squad he is bottom of.
    /// His place in the queue at his position, 1-based. 0 = unknown.
    pub pecking_rank: u8,
    /// How many others are in that queue.
    pub rivals_at_position: u8,
    /// The man in his shirt, or the man chasing it.
    pub top_rival: ActorRef,
    /// That man's age.
    pub top_rival_age: u8,
    /// His observable level minus that man's, −100..+100.
    pub rival_gap: i8,
    pub is_captain: bool,
    pub is_vice_captain: bool,
    /// Where he sits in the squad's pay order, 1.0 top and 0.0 bottom.
    /// Neutral 0.5 when the squad has not been ranked.
    pub wage_standing: f32,

    // ── What this club can still teach him ──────────────────────
    /// The best coaching the club has for a man in his position, 0–20.
    /// 0 = unknown, which reads as no view rather than as a bad staff.
    pub coaching_ceiling: f32,
    /// His own observable level, 1..200. 0 = unknown. Paired with the
    /// ceiling above: what counts is not whether the coaching is good,
    /// it is whether it is good *for him*.
    pub own_level: u8,
    /// How he is training, 0–20, neutral 10.
    pub training_performance: f32,
    /// His regressed season rating, 0..10. 0 when he has not played
    /// enough for it to mean anything.
    pub recent_rating: f32,

    // ── His country ─────────────────────────────────────────────
    /// Months until his nation's next major tournament. `u8::MAX` when
    /// there is none in view, so "no tournament" is further away than
    /// any real one rather than imminent.
    pub months_to_tournament: u8,
    pub national_standing: NationalStanding,
}

impl Default for MindSituation {
    fn default() -> Self {
        MindSituation::neutral()
    }
}

impl MindSituation {
    /// A neutral situation, for tests and for sites with nothing to
    /// offer yet. Every faculty reads this as "no view" rather than as
    /// bad news.
    pub fn neutral() -> Self {
        MindSituation {
            age: 26,
            ambition: 10.0,
            pressure: 10.0,
            adaptability: 10.0,
            loyalty: 10.0,
            professionalism: 10.0,
            temperament: 10.0,
            starter_ratio: 0.5,
            apps_since_goal: 0,
            contract_days_left: 0,
            days_at_club: 0,
            expected_start_share: 0.50,
            manager: ActorRef::NONE,
            club_reputation: 0.5,
            is_abroad: false,
            speaks_local_language: true,
            familiar_teammates: 0,
            is_on_loan: false,
            pecking_rank: 0,
            rivals_at_position: 0,
            top_rival: ActorRef::NONE,
            top_rival_age: 0,
            rival_gap: 0,
            is_captain: false,
            is_vice_captain: false,
            wage_standing: 0.5,
            coaching_ceiling: 0.0,
            own_level: 0,
            training_performance: 10.0,
            recent_rating: 0.0,
            months_to_tournament: u8::MAX,
            national_standing: NationalStanding::Unknown,
        }
    }

    /// Has he been here long enough to have formed a view? Below this
    /// the social and professional faculties hold their tongue rather
    /// than reading a settling-in period as a problem — the same
    /// honeymoon the happiness path already respects.
    pub const SETTLING_DAYS: u16 = 90;

    /// Years of service at which a man stops being a signing and starts
    /// being part of the furniture. Five seasons.
    pub const CLUB_SERVANT_DAYS: u16 = 1825;

    #[inline]
    pub fn is_settled(&self) -> bool {
        self.days_at_club >= Self::SETTLING_DAYS
    }

    /// Years of prime left, 0..1. Nothing at the very end of a career,
    /// full through the mid twenties. Continuous, so there is no
    /// birthday at which a player's outlook flips.
    pub fn career_runway(&self) -> f32 {
        ((34.0 - self.age as f32) / 12.0).clamp(0.0, 1.0)
    }

    /// How far into his career he is, 0..1 — the inverse read, for the
    /// faculties that care about time running out rather than time left.
    #[inline]
    pub fn career_spent(&self) -> f32 {
        1.0 - self.career_runway()
    }

    /// Is he playing regularly?
    #[inline]
    pub fn is_playing(&self) -> bool {
        self.starter_ratio >= 0.5
    }

    /// Contract pressure, 0..1 — nothing with two years left, total at
    /// expiry.
    pub fn contract_pressure(&self) -> f32 {
        if self.contract_days_left == 0 {
            return 0.0;
        }
        (1.0 - self.contract_days_left as f32 / 730.0).clamp(0.0, 1.0)
    }

    /// Is he socially adrift — abroad, without the language, with nobody
    /// who shares his background?
    pub fn is_culturally_isolated(&self) -> bool {
        self.is_abroad && !self.speaks_local_language && self.familiar_teammates == 0
    }

    /// What he is expected to be here, 0..1.
    ///
    /// The paperwork says one thing and the wage packet says another,
    /// and a dressing room believes the wage packet. A club-record
    /// earner expects to play whatever his contract calls him, the crowd
    /// expects it of him, and he knows both — which is why the same
    /// eight quiet games are a promising start for a squad addition and
    /// a crisis for the marquee signing.
    ///
    /// Takes the greater of the two, never the lesser: being paid
    /// modestly does not excuse a man who was promised a starting role.
    pub fn standing_expectation(&self) -> f32 {
        let by_wage = ((self.wage_standing - 0.5) / 0.5).clamp(0.0, 1.0) * 0.75;
        self.expected_start_share.max(by_wage)
    }

    /// Is he carrying a price tag — paid like a man the club is counting
    /// on, and not delivering?
    pub fn carrying_a_price_tag(&self) -> bool {
        self.wage_standing > 0.8 && self.playing_time_gap() < -0.2
    }

    /// How far short of what he was promised he is falling, −1..+1.
    /// Positive means he is playing more than his role implies.
    pub fn playing_time_gap(&self) -> f32 {
        (self.starter_ratio - self.standing_expectation()).clamp(-1.0, 1.0)
    }

    // ── Personality, as continuous drives ───────────────────────
    //
    // Each returns 0..1 so a rule can scale by it rather than branch on
    // a threshold, per `feedback_realistic_not_hacks`.

    /// How much of a stayer he is, 0..1.
    #[inline]
    pub fn loyalty_drive(&self) -> f32 {
        (self.loyalty / 20.0).clamp(0.0, 1.0)
    }

    /// How much of a climber he is, 0..1.
    #[inline]
    pub fn ambition_drive(&self) -> f32 {
        (self.ambition / 20.0).clamp(0.0, 1.0)
    }

    /// How readily he keeps working at something that is not going his
    /// way, 0..1.
    #[inline]
    pub fn diligence(&self) -> f32 {
        (self.professionalism / 20.0).clamp(0.0, 1.0)
    }

    /// How quickly a grievance becomes something he says out loud, 0..1.
    /// A hot head voices at half the provocation a level one needs.
    #[inline]
    pub fn volatility(&self) -> f32 {
        ((20.0 - self.temperament) / 20.0).clamp(0.0, 1.0)
    }

    /// How thin-skinned he is about noise from outside, 0..1.
    #[inline]
    pub fn thin_skinned(&self) -> f32 {
        ((14.0 - self.pressure) / 14.0).clamp(0.0, 1.0)
    }

    // ── Standing in the squad ───────────────────────────────────

    /// Has the squad been ranked at all? Everything positional reads as
    /// no view until the team's weekly pass has run once.
    #[inline]
    pub fn has_squad_view(&self) -> bool {
        self.pecking_rank > 0
    }

    /// Is he first choice at his position on merit?
    #[inline]
    pub fn is_first_choice(&self) -> bool {
        self.pecking_rank == 1
    }

    /// How badly he is being kept out by a man he does not rate above
    /// himself, 0..1.
    ///
    /// The distinction the flat minutes number cannot draw: a boy of
    /// twenty behind a far better player is waiting his turn, and a
    /// twenty-eight-year-old behind an equal is being wronged. It scales
    /// with the gap in his favour and with how little runway he has left
    /// to wait.
    pub fn blocked_unfairly(&self) -> f32 {
        if !self.has_squad_view() || self.is_first_choice() {
            return 0.0;
        }
        // He rates himself at least as highly as the man in front: the
        // grievance is the part of the gap that runs his way. Twelve
        // observable levels of daylight is a man who is plainly better
        // and plainly not being picked.
        let merit = (self.rival_gap as f32 / 12.0).clamp(0.0, 1.0);
        // Whether waiting is worth anything. A boy behind a veteran can
        // see the shirt coming to him and is in no hurry; a man behind
        // someone his own age is waiting for nothing, and one behind a
        // younger player is waiting for it to get worse. Centred so the
        // ordinary case — a peer — is most of a grievance rather than
        // half of one.
        let waiting_is_pointless = if self.top_rival_age == 0 {
            0.8
        } else {
            ((self.age as f32 - self.top_rival_age as f32 + 12.0) / 15.0).clamp(0.0, 1.0)
        };
        (merit * 0.65 + self.career_spent() * 0.35) * waiting_is_pointless
    }

    /// Is the man in front of him old enough that the shirt is coming
    /// anyway? The reason a good young player stays put.
    pub fn can_wait_for_the_shirt(&self) -> bool {
        self.has_squad_view()
            && !self.is_first_choice()
            && self.top_rival_age >= 31
            && self.age <= 24
    }

    // ── Development ─────────────────────────────────────────────

    /// How far the coaching here falls short of the player he already
    /// is, 0..1.
    ///
    /// Not "is this a good staff" but "is this staff good *for him*".
    /// A bench of 12s is a fine education for a third-division full-back
    /// and a dead end for an international. Reads as no view — zero —
    /// until both numbers are known.
    pub fn coaching_shortfall(&self) -> f32 {
        if self.coaching_ceiling <= 0.0 || self.own_level == 0 {
            return 0.0;
        }
        // What a man at his level needs from a coach, on the 0–20 scale.
        // A level-150 player needs a 15; a level-80 player needs an 8.
        let needs = (self.own_level as f32 / 10.0).clamp(4.0, 18.0);
        ((needs - self.coaching_ceiling) / 10.0).clamp(0.0, 1.0)
    }

    /// Is he training like a man who is still going somewhere? −1..+1,
    /// centred on the neutral 10.
    #[inline]
    pub fn training_signal(&self) -> f32 {
        ((self.training_performance - 10.0) / 6.0).clamp(-1.0, 1.0)
    }

    /// How far his own level sits above what a club of this standing
    /// normally holds, 0..1.
    ///
    /// The observable answer to "have I outgrown this place", replacing
    /// a bare reputation threshold with a comparison between two things
    /// that are actually comparable: the player he looks like from
    /// outside, and the players a club of this size usually has.
    pub fn outgrown_the_club(&self) -> f32 {
        if self.own_level == 0 {
            // Nothing to compare him against yet. Fall back to the
            // club's own standing, which is the cruder read the model
            // used before there was a self-read at all: a small club is
            // *some* evidence, just not evidence about this player.
            return ((0.6 - self.club_reputation.clamp(0.0, 1.0)) / 0.6).clamp(0.0, 1.0);
        }
        // A club at the bottom of the reputation scale carries level-40
        // players; one at the top carries level-180 players.
        let club_band = 40.0 + self.club_reputation.clamp(0.0, 1.0) * 140.0;
        ((self.own_level as f32 - club_band) / 40.0).clamp(0.0, 1.0)
    }

    // ── His country ─────────────────────────────────────────────

    /// How hard the next tournament is pressing on him, 0..1.
    ///
    /// Nothing at all outside about eighteen months, rising steeply
    /// through the season before it, and it only presses on a man who
    /// has something to lose or gain by it. The January of a tournament
    /// year is the sharpest month in the calendar and this is why.
    pub fn tournament_pressure(&self) -> f32 {
        if self.months_to_tournament == u8::MAX || !self.national_standing.is_international() {
            return 0.0;
        }
        // 18 months out: nothing. On the eve of it: total.
        let nearness = ((18.0 - self.months_to_tournament as f32) / 18.0).clamp(0.0, 1.0);
        // Squared, so the pressure genuinely belongs to the last year
        // rather than being spread evenly over the cycle.
        nearness * nearness * self.national_standing.tournament_stake()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_neutral_situation_reads_as_no_view_rather_than_bad_news() {
        let s = MindSituation::neutral();
        assert_eq!(s.playing_time_gap(), 0.0);
        assert_eq!(s.contract_pressure(), 0.0);
        assert!(!s.is_culturally_isolated());
        assert!(!s.is_settled(), "day one is not settled");
        assert_eq!(s.blocked_unfairly(), 0.0, "an unranked squad is no view");
        assert_eq!(s.coaching_shortfall(), 0.0, "an unknown staff is no view");
        assert_eq!(s.tournament_pressure(), 0.0);
        assert_eq!(s.training_signal(), 0.0);
    }

    #[test]
    fn the_runway_shortens_continuously() {
        let at = |age: u8| {
            MindSituation {
                age,
                ..MindSituation::neutral()
            }
            .career_runway()
        };

        assert_eq!(at(22), 1.0);
        assert!(at(28) > at(31));
        assert!(at(31) > at(33));
        assert_eq!(at(35), 0.0);
        // No cliff anywhere in between.
        for age in 22..35u8 {
            assert!(at(age) >= at(age + 1));
        }
    }

    #[test]
    fn contract_pressure_builds_over_the_last_two_years() {
        let with = |days: u16| {
            MindSituation {
                contract_days_left: days,
                ..MindSituation::neutral()
            }
            .contract_pressure()
        };

        assert_eq!(with(730), 0.0);
        assert!((with(365) - 0.5).abs() < 0.01);
        assert!(with(30) > 0.9);
        assert_eq!(
            with(0),
            0.0,
            "no contract is not the same as an expiring one"
        );
    }

    #[test]
    fn the_playing_time_gap_is_measured_against_the_role_he_was_given() {
        let benched_key_player = MindSituation {
            expected_start_share: 0.70,
            starter_ratio: 0.2,
            ..MindSituation::neutral()
        };
        let same_minutes_as_a_backup = MindSituation {
            expected_start_share: 0.15,
            starter_ratio: 0.2,
            ..MindSituation::neutral()
        };

        assert!(benched_key_player.playing_time_gap() < -0.4);
        assert!(
            same_minutes_as_a_backup.playing_time_gap() > -0.1,
            "identical minutes, and only one of them has a grievance"
        );
    }

    #[test]
    fn cultural_isolation_needs_all_three() {
        let adrift = MindSituation {
            is_abroad: true,
            speaks_local_language: false,
            familiar_teammates: 0,
            ..MindSituation::neutral()
        };
        assert!(adrift.is_culturally_isolated());

        let has_a_compatriot = MindSituation {
            familiar_teammates: 2,
            ..adrift
        };
        assert!(
            !has_a_compatriot.is_culturally_isolated(),
            "one man who speaks your language changes everything"
        );
    }

    #[test]
    fn a_boy_behind_a_veteran_is_not_in_the_same_position_as_a_man_behind_a_peer() {
        // Twenty, second choice, and the man in front is thirty-four.
        let waiting_his_turn = MindSituation {
            age: 20,
            pecking_rank: 2,
            rivals_at_position: 1,
            top_rival_age: 34,
            rival_gap: -15,
            ..MindSituation::neutral()
        };
        // Twenty-eight, second choice, and the man in front is his equal
        // and his age.
        let being_wronged = MindSituation {
            age: 28,
            pecking_rank: 2,
            rivals_at_position: 1,
            top_rival_age: 28,
            rival_gap: 5,
            ..MindSituation::neutral()
        };

        assert!(waiting_his_turn.can_wait_for_the_shirt());
        assert!(!being_wronged.can_wait_for_the_shirt());
        assert!(
            being_wronged.blocked_unfairly() > waiting_his_turn.blocked_unfairly(),
            "identical minutes; only one of them has a grievance"
        );
    }

    #[test]
    fn first_choice_has_nothing_to_be_aggrieved_about() {
        let first = MindSituation {
            pecking_rank: 1,
            rivals_at_position: 3,
            rival_gap: 20,
            ..MindSituation::neutral()
        };
        assert_eq!(first.blocked_unfairly(), 0.0);
    }

    #[test]
    fn the_coaching_shortfall_is_measured_against_the_player_he_already_is() {
        // The same modest bench, read by two very different players.
        let international_at_a_small_club = MindSituation {
            own_level: 160,
            coaching_ceiling: 8.0,
            ..MindSituation::neutral()
        };
        let lower_league_pro_at_the_same_club = MindSituation {
            own_level: 70,
            coaching_ceiling: 8.0,
            ..MindSituation::neutral()
        };

        assert!(international_at_a_small_club.coaching_shortfall() > 0.5);
        assert_eq!(
            lower_league_pro_at_the_same_club.coaching_shortfall(),
            0.0,
            "a bench of eights is a perfectly good education at that level"
        );
    }

    #[test]
    fn an_elite_staff_falls_short_of_nobody() {
        let elite = MindSituation {
            own_level: 180,
            coaching_ceiling: 19.0,
            ..MindSituation::neutral()
        };
        assert_eq!(elite.coaching_shortfall(), 0.0);
    }

    #[test]
    fn the_tournament_only_presses_in_the_last_year_and_only_on_internationals() {
        let with = |months: u8, standing: NationalStanding| {
            MindSituation {
                months_to_tournament: months,
                national_standing: standing,
                ..MindSituation::neutral()
            }
            .tournament_pressure()
        };

        assert_eq!(with(18, NationalStanding::InContention), 0.0);
        assert!(with(12, NationalStanding::InContention) < 0.2);
        assert!(with(6, NationalStanding::InContention) > 0.4);
        assert!(with(1, NationalStanding::InContention) > 0.85);

        assert_eq!(
            with(4, NationalStanding::Unknown),
            0.0,
            "a man with no international standing has no stake in it"
        );
        assert!(
            with(4, NationalStanding::InContention) > with(4, NationalStanding::Established),
            "the man on the edge of the squad is the one it decides"
        );
    }

    #[test]
    fn personality_reads_as_a_continuous_drive_not_a_band() {
        let loyal = MindSituation {
            loyalty: 18.0,
            ..MindSituation::neutral()
        };
        let mercenary = MindSituation {
            loyalty: 3.0,
            ..MindSituation::neutral()
        };
        assert!(loyal.loyalty_drive() > mercenary.loyalty_drive());

        let hot_head = MindSituation {
            temperament: 4.0,
            ..MindSituation::neutral()
        };
        let level = MindSituation {
            temperament: 17.0,
            ..MindSituation::neutral()
        };
        assert!(hot_head.volatility() > level.volatility());
    }
}
