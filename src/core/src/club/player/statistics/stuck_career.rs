//! Career stagnation — how many recent seasons a player has spent at his
//! club without first-team football, and whether the club keeps shipping
//! him out on loan instead of picking him.
//!
//! Read from the canonical `season_ledger`, so it is observable history:
//! what actually happened on the pitch, never hidden ability. A season
//! counts as "stuck" when the player's PARENT-club league starts and
//! appearances both stay under the bars — which means a year spent
//! entirely out on loan counts too (zero parent starts), while a genuine
//! run of starts breaks the chain.
//!
//! Consumers:
//!   * the perennial-backup mood audit (`BackupCareerAnxiety`) — whether a
//!     squad player still minds being a spectator, and how loudly.
//!   * `Player::process_transfer_desire` — whether the grievance has
//!     hardened into a standing request to go and play somewhere else
//!     ([`TransferRequestReason::WantsFirstTeamFootball`]).
//!   * the playing-time frustration model — bench credit stops absolving
//!     the club once the seasons pile up
//!     ([`StuckCareerScan::bench_credit_scale`]).
//!
//! Both consumers share the scan so the club's story about a player and
//! the player's own story about himself can never drift apart.

use super::ledger::PlayerStatCompetitionKind;
use crate::Player;
use chrono::{Datelike, Duration, NaiveDate};

/// Consecutive recent seasons without first-team football at the parent
/// club, plus whether the club has been circulating the player on loan.
/// Built by [`StuckCareerScan::of`]; `None` means there is no usable
/// league history to judge from yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StuckCareerScan {
    /// Consecutive season-years, counting back from the player's most
    /// recent league season at this club, in which he got no first-team
    /// football. Saturates at [`Self::MAX_LOOKBACK_YEARS`].
    pub stuck_years: u16,
    /// Two or more distinct season-years of the last four were spent out
    /// on loan from this club — the player keeps being circulated
    /// instead of played.
    pub serial_loanee: bool,
}

/// Personality and setting behind [`StuckCareerScan::restlessness`].
#[derive(Debug, Clone, Copy)]
pub struct RestlessnessInputs {
    /// Hidden personality ambition, 0..20.
    pub ambition: f32,
    /// Mental determination skill, 0..20.
    pub determination: f32,
    /// Hidden personality loyalty, 0..20.
    pub loyalty: f32,
    pub age: u8,
    pub is_goalkeeper: bool,
    /// Club standing on the 4000..8000 reputation band, normalised 0..1
    /// — how much prestige there is in simply belonging here.
    pub club_rep01: f32,
}

/// How badly a stuck player wants out, and the shared discount both
/// consumers apply to the comforts that keep him.
#[derive(Debug, Clone, Copy)]
pub struct Restlessness {
    /// 0..1-ish push, before any wage-comfort subtraction.
    pub score: f32,
    /// Inside the position-adjusted prime window — the years in which
    /// being a spectator costs the player a career rather than a season.
    pub in_prime_window: bool,
}

impl StuckCareerScan {
    /// A season with at most this many parent-club league starts…
    pub const STUCK_SEASON_MAX_STARTS: u16 = 8;
    /// …and at most this many total parent-club league appearances counts
    /// as a season without first-team football.
    pub const STUCK_SEASON_MAX_APPS: u16 = 15;
    /// How many season-years back the stuck-season scan may walk.
    pub const MAX_LOOKBACK_YEARS: u16 = 6;

    /// Position-adjusted career phases: `(prime_start, prime_end,
    /// fade_end)`. Goalkeeper careers run ~3 years later, which is why a
    /// keeper can still be waiting for a first-team shirt at 30 while an
    /// outfield player the same age has run out of road. Shared by the
    /// backup-anxiety score, the reserve-ambition fade and the
    /// first-team-football request so every breakthrough dream lights up
    /// and extinguishes on the same line.
    pub fn career_phases(is_goalkeeper: bool) -> (f32, f32, f32) {
        if is_goalkeeper {
            (27.0, 33.0, 37.0)
        } else {
            (26.0, 31.0, 34.0)
        }
    }

    /// Walk the canonical season ledger backwards from the most recent
    /// league season the player was at this club for. Returns `None` when
    /// there is no league history at this club to judge from.
    pub fn of(player: &Player, today: NaiveDate) -> Option<Self> {
        let ledger = &player.statistics_history.season_ledger;
        // Seasons before the player joined this club say nothing about
        // his standing here. Homegrown players have no floor.
        let join_year_floor = Self::club_tenure_days(player, today)
            .map(|d| (today - Duration::days(d)).year() as u16);
        let at_club = |year: u16| join_year_floor.map_or(true, |floor| year >= floor);

        let anchor = ledger
            .iter()
            .filter(|e| matches!(e.competition_kind, PlayerStatCompetitionKind::League))
            .filter(|e| at_club(e.season_start_year))
            .map(|e| e.season_start_year)
            .max()?;

        let mut stuck_years: u16 = 0;
        for back in 0..Self::MAX_LOOKBACK_YEARS {
            let Some(year) = anchor.checked_sub(back) else {
                break;
            };
            if !at_club(year) {
                break;
            }
            // Sum parent-club (non-loan) league starts / apps for the
            // year; a year with no parent rows at all was a year the
            // club gave him nothing.
            let (starts, apps) = ledger
                .iter()
                .filter(|e| {
                    e.season_start_year == year
                        && !e.is_loan
                        && matches!(e.competition_kind, PlayerStatCompetitionKind::League)
                })
                .fold((0u16, 0u16), |(s, a), e| {
                    (
                        s.saturating_add(e.statistics.played),
                        a.saturating_add(e.statistics.played)
                            .saturating_add(e.statistics.played_subs),
                    )
                });
            if starts <= Self::STUCK_SEASON_MAX_STARTS && apps <= Self::STUCK_SEASON_MAX_APPS {
                stuck_years += 1;
            } else {
                break;
            }
        }

        // Serial loanee: two or more distinct season-years of the last
        // four spent out on loan from this club.
        let loan_floor = anchor.saturating_sub(3);
        let mut loan_years: Vec<u16> = ledger
            .iter()
            .filter(|e| {
                e.is_loan
                    && matches!(e.competition_kind, PlayerStatCompetitionKind::League)
                    && e.season_start_year >= loan_floor
                    && at_club(e.season_start_year)
            })
            .map(|e| e.season_start_year)
            .collect();
        loan_years.sort_unstable();
        loan_years.dedup();

        Some(Self {
            stuck_years,
            serial_loanee: loan_years.len() >= 2,
        })
    }

    /// How long the player has actually been this club's player.
    ///
    /// `days_since_transfer` is stamped by the loan machinery as well as
    /// by the market, so a player who is lent out and brought home reads
    /// as a brand-new signing twice a year — and a serial loanee, the
    /// exact career this scan exists to see, would never clear a settling
    /// window at all. The permanent contract's own start date is the
    /// honest anchor for a stay; the later of the two is taken so a
    /// renewal that re-stamps the deal cannot shorten a long one either.
    /// `None` means no anchor at all — a homegrown player who has never
    /// moved, and who is settled by definition.
    pub fn club_tenure_days(player: &Player, today: NaiveDate) -> Option<i64> {
        let since_move = player.days_since_transfer(today)?;
        let since_contract = player
            .contract
            .as_ref()
            .and_then(|c| c.started)
            .map(|start| (today - start).num_days())
            .unwrap_or(0);
        Some(since_move.max(since_contract))
    }

    /// 0..1 stuck pressure, saturating at four seasons — the shared ramp
    /// both the mood audit and the transfer-desire escalation read so a
    /// player's patience runs out on one curve, not two.
    pub fn pressure(&self) -> f32 {
        self.stuck_years.min(4) as f32 / 4.0
    }

    /// How hard this player is pushing to go and play, 0..1, before the
    /// club's own wage-comfort argument is subtracted.
    ///
    /// One curve, two consumers: the monthly mood audit asks "does he
    /// mind?" and the daily transfer-desire pass asks "does he act?".
    /// They apply different bars to the same number, so a player can
    /// never be quietly seething by one model and content by the other.
    pub fn restlessness(&self, inputs: RestlessnessInputs) -> Restlessness {
        let ambition01 = (inputs.ambition / 20.0).clamp(0.0, 1.0);
        let determination01 = (inputs.determination / 20.0).clamp(0.0, 1.0);
        let loyalty01 = (inputs.loyalty / 20.0).clamp(0.0, 1.0);

        // Age urgency: ramps in from 24, saturates through the prime
        // ("the years are slipping away"), then eases toward the
        // late-career line as the veteran makes peace with the role.
        let (prime_start, prime_end, fade_end) = Self::career_phases(inputs.is_goalkeeper);
        let a = inputs.age as f32;
        let age_urgency = if a < prime_start {
            0.35 + 0.65 * ((a - 23.0) / (prime_start - 23.0)).clamp(0.0, 1.0)
        } else if a <= prime_end {
            1.0
        } else {
            1.0 - 0.5 * ((a - prime_end) / (fade_end - prime_end)).clamp(0.0, 1.0)
        };

        let stuck_pressure = self.pressure();
        let serial_bonus = if self.serial_loanee { 0.15 } else { 0.0 };

        // Prestige wears out; money does not. Belonging to a famous club
        // is a thrill in your first seasons there and simply your address
        // by the fifth — whereas a contract above your own worth pays the
        // same every month however long you have been watching, which is
        // exactly why real squad players sign the extension. Without the
        // prestige taper a giant's reputation saturated the comfort term
        // permanently, so only a near-maximum-ambition player could ever
        // grow restless and every ordinary deputy served out his career
        // in silence.
        let prestige_wear = 1.0 - 0.55 * stuck_pressure;
        let big_club_comfort =
            inputs.club_rep01.clamp(0.0, 1.0) * 0.22 * (1.0 - 0.6 * ambition01) * prestige_wear;
        let loyalty_brake = loyalty01 * 0.10;

        let score = 0.40 * ambition01
            + 0.28 * age_urgency
            + 0.17 * stuck_pressure
            + 0.08 * determination01
            + serial_bonus
            - big_club_comfort
            - loyalty_brake;

        Restlessness {
            score,
            in_prime_window: a >= prime_start - 2.0 && a <= prime_end,
        }
    }

    /// How much of the usual unused-bench involvement credit still counts
    /// after this many stuck seasons, 0..1.
    ///
    /// Being named on the bench every week is genuine involvement the
    /// first season — the player is around the first team, travelling,
    /// one injury from playing. It stops feeling like involvement when it
    /// is all he has had for years. Without this taper the credit
    /// (0.10/match) sits so close to a backup's own expectation bar
    /// (0.15/match) that his playing-time deficit mathematically cannot
    /// reach the complaint threshold no matter how many seasons pass — a
    /// career spectator with no voice.
    ///
    /// Continuous by design: season one is untouched, so the settled
    /// squad-role calibration holds, and the credit fades from there.
    pub fn bench_credit_scale(&self) -> f32 {
        const FULL_CREDIT_SEASONS: f32 = 1.0;
        const FADE_SEASONS: f32 = 3.0;
        /// Floor — the bench is never worth literally nothing.
        const RESIDUAL: f32 = 0.35;
        let over = (self.stuck_years as f32 - FULL_CREDIT_SEASONS).max(0.0);
        let faded = (over / FADE_SEASONS).clamp(0.0, 1.0);
        1.0 - (1.0 - RESIDUAL) * faded
    }
}
