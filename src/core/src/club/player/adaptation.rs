use crate::club::player::language::Language;
use crate::club::player::player::{ManagerPromiseKind, Player};
use crate::club::PlayerPositionType;
use crate::HappinessEventType;
use chrono::NaiveDate;

/// Post-transfer settling window. For the first ~12 weeks at a new club the
/// player's match rating is dampened, and weekly integration events fire.
pub const SETTLEMENT_WINDOW_DAYS: i64 = 84;

/// Context left on the player by transfer execution. Consumed the next
/// time the player simulates — that's where shock events, role-fit checks
/// and the implicit playing-time promise are emitted. Keeping this as
/// transient state (rather than having execution push events directly)
/// means the player reacts to a new environment as part of his own
/// processing, alongside happiness, language, integration, etc.
#[derive(Debug, Clone)]
pub struct PendingSigning {
    pub previous_salary: Option<u32>,
    pub fee: f64,
    pub is_loan: bool,
    /// Destination club id — needed to check whether the signing is to one
    /// of the player's favorite clubs so the right shock event can fire.
    pub destination_club_id: u32,
}

/// Ambition-vs-club gap (raw units, see `calculate_ambition_fit`) past which
/// the player notices he joined below his level.
const AMBITION_SHOCK_THRESHOLD: f32 = 4000.0;

/// Ambition-vs-club surplus past which the move is felt as a clear step up
/// (club rep exceeds what the player's ambition was expecting).
const DREAM_MOVE_THRESHOLD: f32 = 1500.0;

/// Club reputation (0..10000 scale) above which the signing carries extra
/// prestige — Champions League contenders and the giants.
const ELITE_CLUB_REPUTATION: f32 = 7500.0;

/// New/old salary ratio below which the contract is felt as a demotion.
const SALARY_SHOCK_RATIO: f32 = 0.4;

/// New/old salary ratio above which the contract is felt as a breakthrough.
const SALARY_BOOST_RATIO: f32 = 1.8;

impl Player {
    /// Days elapsed since the player's most recent transfer/loan, if any.
    pub fn days_since_transfer(&self, now: NaiveDate) -> Option<i64> {
        self.last_transfer_date.map(|d| (now - d).num_days())
    }

    /// Multiplier (0.80..1.00) applied to match rating while settling at a
    /// new club. Linear recovery across [`SETTLEMENT_WINDOW_DAYS`], trimmed
    /// by local-language fluency, adaptability, and motivation when the
    /// move is a step up (an excited youngster at Barcelona adapts faster
    /// than a demoralised veteran at a minnow).
    pub fn settlement_form_multiplier(
        &self,
        now: NaiveDate,
        country_code: &str,
        club_rep_0_to_1: f32,
    ) -> f32 {
        let days = match self.days_since_transfer(now) {
            Some(d) if d >= 0 && d < SETTLEMENT_WINDOW_DAYS => d as f32,
            _ => return 1.0,
        };

        let recovery = days / SETTLEMENT_WINDOW_DAYS as f32;
        let mut penalty = (1.0 - recovery) * 0.15;

        if self.speaks_local_language(country_code) {
            penalty *= 0.4;
        }

        let adapt = self.attributes.adaptability.clamp(0.0, 20.0);
        let adapt_factor = 1.0 - (adapt / 20.0) * 0.6;
        penalty *= adapt_factor;

        if self.is_step_up_move(club_rep_0_to_1) {
            penalty *= 0.6;
        }

        (1.0 - penalty).clamp(0.80, 1.0)
    }

    /// A step-up move is one where the club's reputation visibly exceeds
    /// what the player's ambition was already expecting.
    pub fn is_step_up_move(&self, club_rep_0_to_1: f32) -> bool {
        let ambition = self.attributes.ambition;
        let expected_rep = (ambition - 5.0).max(0.0) * 800.0;
        let club_rep = club_rep_0_to_1 * 10000.0;
        club_rep - expected_rep >= DREAM_MOVE_THRESHOLD
    }

    /// True if the player speaks the country's primary language well enough
    /// (native or ≥70 proficiency) that culture shock is muted.
    pub fn speaks_local_language(&self, country_code: &str) -> bool {
        if country_code.is_empty() {
            return true;
        }
        let langs = Language::from_country_code(country_code);
        if langs.is_empty() {
            return true;
        }
        langs.iter().any(|l| {
            self.languages.iter().any(|pl| {
                pl.language == *l && (pl.is_native || pl.proficiency >= 70)
            })
        })
    }

    /// Consume a pending signing: emit the one-shot shock events, check role
    /// fit against the current formation, and record the implicit playing-
    /// time promise. Safe to call every tick — it's a no-op if nothing is
    /// pending.
    pub fn process_transfer_shock(
        &mut self,
        now: NaiveDate,
        club_rep_0_to_1: f32,
        country_code: &str,
        formation: Option<&[PlayerPositionType; 11]>,
    ) {
        let Some(pending) = self.pending_signing.take() else { return };

        // Ambition / dream / elite-club reactions fire for loans too —
        // being loaned to Real Madrid is still the move of your life, even
        // if you're going back in a year. Loans pay at the borrowing club's
        // loan wage (distinct from a full contract) so salary shock/boost
        // is skipped for them; that lever is tuned for permanent moves.
        let loan_damp = if pending.is_loan { 0.7 } else { 1.0 };
        let is_favorite_destination = self.favorite_clubs.contains(&pending.destination_club_id);
        // Ambition shock is muted when joining a favorite club — the player
        // knew what they were signing for and the sentimental pull covers the
        // ambition gap. Reputation-based "I should be at a bigger club"
        // doesn't apply to your boyhood side.
        if !is_favorite_destination {
            self.emit_ambition_shock(club_rep_0_to_1, loan_damp);
        }
        if is_favorite_destination {
            // Signing for a childhood/legend club trumps the reputation-gap
            // logic — fire DreamMove at full weight regardless of where the
            // club sits on the prestige ladder. A player returning to boyhood
            // club feels this even if it's a rep-drop move.
            self.happiness
                .add_event(HappinessEventType::DreamMove, 15.0 * loan_damp);
        } else {
            self.emit_dream_move(club_rep_0_to_1, loan_damp);
        }
        self.emit_joining_elite(club_rep_0_to_1, loan_damp);

        if !pending.is_loan {
            self.emit_salary_shock(pending.previous_salary);
            self.emit_salary_boost(pending.previous_salary);
        }

        // Shirt number prestige — getting a single-digit or iconic number
        // at the new club is a real pride moment, especially for younger
        // players. Fires once per signing.
        if let Some(shirt) = self.contract.as_ref().and_then(|c| c.shirt_number) {
            let magnitude = match shirt {
                7 | 9 | 10 => 4.0,
                1..=11 => 2.0,
                _ => 0.0,
            };
            if magnitude > 0.0 {
                self.happiness
                    .add_event(HappinessEventType::ShirtNumberPromotion, magnitude);
            }
        }

        if !self.speaks_local_language(country_code) {
            let mag = if pending.is_loan { -3.0 } else { -5.0 };
            self.happiness.add_event(HappinessEventType::FeelingIsolated, mag);
        }

        if let Some(f) = formation {
            self.emit_role_mismatch_if_unfit(f);
        }

        // Big-money signings (or loans — the borrowing club took him to play)
        // arrive with an implicit playing-time promise.
        let promise_horizon = if pending.is_loan {
            60
        } else if pending.fee >= 5_000_000.0 {
            90
        } else {
            0
        };
        if promise_horizon > 0 {
            self.record_promise(ManagerPromiseKind::PlayingTime, now, promise_horizon);
        }
    }

    fn emit_ambition_shock(&mut self, club_rep_0_to_1: f32, damp: f32) {
        let ambition = self.attributes.ambition;
        if ambition <= 10.0 {
            return;
        }
        let expected_rep = (ambition - 10.0) * 800.0;
        let club_rep = club_rep_0_to_1 * 10000.0;
        let gap = expected_rep - club_rep;
        if gap <= AMBITION_SHOCK_THRESHOLD {
            return;
        }
        let severity = (gap / 8000.0).clamp(0.5, 2.0);
        self.happiness
            .add_event(HappinessEventType::AmbitionShock, -8.0 * severity * damp);
    }

    fn emit_salary_shock(&mut self, previous_salary: Option<u32>) {
        let Some(prev) = previous_salary else { return };
        let Some(new) = self.contract.as_ref().map(|c| c.salary) else { return };
        if prev == 0 {
            return;
        }
        let ratio = new as f32 / prev as f32;
        if ratio >= SALARY_SHOCK_RATIO {
            return;
        }
        let severity = ((SALARY_SHOCK_RATIO - ratio) / SALARY_SHOCK_RATIO).clamp(0.0, 1.0);
        self.happiness
            .add_event(HappinessEventType::SalaryShock, -6.0 - 6.0 * severity);
    }

    fn emit_dream_move(&mut self, club_rep_0_to_1: f32, damp: f32) {
        let ambition = self.attributes.ambition;
        let expected_rep = (ambition - 5.0).max(0.0) * 800.0;
        let club_rep = club_rep_0_to_1 * 10000.0;
        let surplus = club_rep - expected_rep;
        if surplus < DREAM_MOVE_THRESHOLD {
            return;
        }
        // Magnitude scales with how far above expectations the move is;
        // ambitious players (high `ambition`) also feel it more strongly.
        let severity = (surplus / 6000.0).clamp(0.5, 2.0);
        let ambition_weight = (ambition / 20.0).clamp(0.4, 1.2);
        self.happiness.add_event(
            HappinessEventType::DreamMove,
            10.0 * severity * ambition_weight * damp,
        );
    }

    fn emit_joining_elite(&mut self, club_rep_0_to_1: f32, damp: f32) {
        let club_rep = club_rep_0_to_1 * 10000.0;
        if club_rep < ELITE_CLUB_REPUTATION {
            return;
        }
        let player_rep = self.player_attributes.world_reputation as f32;
        // Only fire if the club is meaningfully above the player's own
        // standing — a Ballon d'Or winner moving clubs doesn't feel this.
        if club_rep - player_rep < 1500.0 {
            return;
        }
        self.happiness.add_event(HappinessEventType::JoiningElite, 6.0 * damp);
    }

    fn emit_salary_boost(&mut self, previous_salary: Option<u32>) {
        let Some(prev) = previous_salary else { return };
        let Some(new) = self.contract.as_ref().map(|c| c.salary) else { return };
        if prev == 0 {
            return;
        }
        let ratio = new as f32 / prev as f32;
        if ratio < SALARY_BOOST_RATIO {
            return;
        }
        let severity = ((ratio - SALARY_BOOST_RATIO) / 2.0).clamp(0.0, 1.5);
        self.happiness
            .add_event(HappinessEventType::SalaryBoost, 4.0 + 4.0 * severity);
    }

    fn emit_role_mismatch_if_unfit(&mut self, formation: &[PlayerPositionType; 11]) {
        let primary = self.position();
        if formation.iter().any(|p| *p == primary) {
            return;
        }
        let group_match = formation
            .iter()
            .any(|p| p.position_group() == primary.position_group());
        let mag = if group_match { -4.0 } else { -8.0 };
        self.happiness.add_event(HappinessEventType::RoleMismatch, mag);
    }

    /// Development multiplier applied when a player has just stepped up to
    /// a better club. Training alongside higher-calibre teammates and
    /// absorbing a new tactical culture accelerates growth — but only
    /// while there's still catching up to do. The effect fades over the
    /// settlement window and is proportional to the rep gap.
    pub fn step_up_development_multiplier(
        &self,
        now: NaiveDate,
        club_rep_0_to_1: f32,
    ) -> f32 {
        let days = match self.days_since_transfer(now) {
            Some(d) if d >= 0 && d < SETTLEMENT_WINDOW_DAYS => d as f32,
            _ => return 1.0,
        };
        let club_rep = club_rep_0_to_1 * 10000.0;
        let player_rep = self.player_attributes.world_reputation as f32;
        let gap = club_rep - player_rep;
        if gap <= 1000.0 {
            return 1.0;
        }
        let gap_factor = (gap / 8000.0).clamp(0.0, 0.25);
        let recency = 1.0 - (days / SETTLEMENT_WINDOW_DAYS as f32);
        (1.0 + gap_factor * recency).clamp(1.0, 1.25)
    }

    /// Weekly integration tick. During the settlement window the player
    /// either bonds with the squad or feels isolated, depending on language
    /// fluency, personality, and age. Runs for ~24 weeks after a transfer so
    /// there's a tail of recovery even once the form penalty has faded.
    pub fn process_integration(&mut self, now: NaiveDate, country_code: &str) {
        let Some(days) = self.days_since_transfer(now) else {
            self.process_chronic_language_isolation(now, country_code);
            return;
        };
        if !(0..=168).contains(&days) {
            self.process_chronic_language_isolation(now, country_code);
            return;
        }

        let weeks = days / 7;
        let speaks_local = self.speaks_local_language(country_code);
        let adapt = self.attributes.adaptability.clamp(0.0, 20.0);
        let prof = self.attributes.professionalism.clamp(0.0, 20.0);
        let pull_toward_bonding = (adapt + prof) / 40.0;

        if weeks < 4 && !speaks_local && adapt < 12.0 {
            self.happiness
                .add_event(HappinessEventType::FeelingIsolated, -2.0);
            return;
        }

        if pull_toward_bonding > 0.55 || speaks_local {
            self.happiness
                .add_event(HappinessEventType::TeammateBonding, 1.5);
        }

        if weeks >= 8 && (pull_toward_bonding > 0.5 || speaks_local) {
            self.happiness
                .add_event(HappinessEventType::SettledIntoSquad, 1.0);
        }
    }

    /// Post-settlement ongoing language check. A player who's been at a
    /// foreign club for years but never picked up the language keeps
    /// accruing small isolation hits — the dressing-room outsider model.
    /// Runs monthly (day-of-month 1) instead of weekly to avoid stacking.
    fn process_chronic_language_isolation(&mut self, now: NaiveDate, country_code: &str) {
        use chrono::Datelike;
        if now.day() != 1 {
            return;
        }
        if self.speaks_local_language(country_code) {
            return;
        }
        // Passive acceptance: high adaptability/professionalism masks it.
        let adapt = self.attributes.adaptability.clamp(0.0, 20.0);
        let prof = self.attributes.professionalism.clamp(0.0, 20.0);
        if (adapt + prof) / 40.0 > 0.7 {
            return;
        }
        self.happiness
            .add_event(HappinessEventType::FeelingIsolated, -1.5);
    }
}
