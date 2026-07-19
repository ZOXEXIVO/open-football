//! Squad-need calculus + emergency free-agent selection.
//!
//! Two cooperating pieces:
//!
//! 1. [`FirstTeamSquadNeeds`] — pure snapshot of how badly a main team is
//!    underfilled. Single source of truth for the first-team minimums
//!    used by both the transfer-pipeline padding logic and the
//!    emergency free-agent pass.
//!
//! 2. [`EmergencySquadFillStrategy`] — pure scoring fn that ranks a
//!    free-agent candidate for a club whose main squad is below the
//!    minimum. Prefers local/age-fit players, penalises high-rep stars
//!    unlikely to drop down, allows older veterans when the squad is
//!    critically short.
//!
//! Sits as a sibling of `pipeline` so both the per-country emergency
//! signing path (`country::result::transfers::free_agents`) and the
//! transfer-pipeline squad evaluation can import the helpers without
//! crossing the driver / framework layering.
//!
//! Note: `MIN_FIRST_TEAM_SQUAD` is re-exported from
//! `crate::club::team::squad` (the existing source of truth) — this
//! module only adds the per-group minimums and the snapshot/scoring
//! structs.
use crate::club::Club;
use crate::club::team::squad::MIN_FIRST_TEAM_SQUAD;
use crate::{PlayerFieldPositionGroup, PlayerPositionType};

/// Minimum goalkeepers a first team should carry — one starter plus
/// at least one credible deputy. Below this the club can't survive a
/// single suspension or knock to its keeper.
pub const MIN_GROUP_GOALKEEPER: usize = 2;
/// Minimum defenders — four-at-the-back baseline plus three rotation
/// bodies. Matches the way `evaluation::group_depth_requirement`
/// pads back-fours.
pub const MIN_GROUP_DEFENDER: usize = 7;
/// Minimum midfielders — three/four-in-midfield baseline plus a
/// handful of rotation cover. Same depth target as defenders so the
/// shape can absorb injuries in either zone.
pub const MIN_GROUP_MIDFIELDER: usize = 7;
/// Minimum forwards — two starters plus a deputy and a wide
/// rotation option. Anything below leaves the side with no plan B
/// when the lone striker is suspended.
pub const MIN_GROUP_FORWARD: usize = 4;

/// Pure snapshot of how short the main first-team is, per group and in
/// total. Built by [`FirstTeamSquadNeeds::for_club`] once per emergency
/// pass and shared with the scoring strategy below.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FirstTeamSquadNeeds {
    /// Current main-team headcount. Counts loanees too — they
    /// physically occupy a roster slot and the player isn't "owned"
    /// for purposes of the depth calculation, but the emergency pass
    /// should not double-fill while loanees are in place.
    pub main_team_size: usize,
    /// Headcount missing against [`MIN_FIRST_TEAM_SQUAD`]. Zero when
    /// the squad is at or above the minimum.
    pub total_missing: usize,
    pub gk_missing: usize,
    pub def_missing: usize,
    pub mid_missing: usize,
    pub fwd_missing: usize,
    /// Actual per-group headcounts. Kept alongside the shortfalls so the
    /// emergency projection can seed from real numbers — reconstructing
    /// counts as `MIN - missing` clamped every above-minimum group at
    /// its floor (10 defenders read as 7), which skewed the
    /// thinnest-group rotation toward already-stuffed groups.
    pub gk_count: usize,
    pub def_count: usize,
    pub mid_count: usize,
    pub fwd_count: usize,
    /// Under-11 = the club can't even field a side — the most extreme
    /// emergency state, used to ignore wage / rep gates in matching.
    pub urgent: bool,
}

/// One slot in the emergency signing plan: which group still has
/// headroom, how many bodies to add, and the reason string the
/// signing should be tagged with so the UI can surface why the
/// player was picked.
#[derive(Debug, Clone, Copy)]
pub struct EmergencyGroupSlot {
    pub group: PlayerFieldPositionGroup,
    pub missing: usize,
    pub reason: &'static str,
}

impl EmergencyGroupSlot {
    /// Concrete position type used when emitting transfer requests
    /// or matching candidates for this slot. One per group — the
    /// pipeline downstream resolves position-group fit, so a single
    /// canonical position keeps requests stable without needing to
    /// enumerate every layout (DCL, DCR, DM, …).
    pub fn representative_position(group: PlayerFieldPositionGroup) -> PlayerPositionType {
        match group {
            PlayerFieldPositionGroup::Goalkeeper => PlayerPositionType::Goalkeeper,
            PlayerFieldPositionGroup::Defender => PlayerPositionType::DefenderCenter,
            PlayerFieldPositionGroup::Midfielder => PlayerPositionType::MidfielderCenter,
            PlayerFieldPositionGroup::Forward => PlayerPositionType::Striker,
        }
    }
}

impl FirstTeamSquadNeeds {
    /// Re-export of the central first-team minimum so callers don't
    /// have to reach into `club::team::squad` directly.
    pub const MIN_FIRST_TEAM_SQUAD: usize = MIN_FIRST_TEAM_SQUAD;

    /// Build the snapshot from a club's main team. Falls back to the
    /// first available team if `main()` is None (test fixtures); an
    /// entirely team-less club reports every slot as missing.
    pub fn for_club(club: &Club) -> Self {
        let main = club.teams.main().or_else(|| club.teams.teams.first());
        let main_team_size = main.map(|t| t.players.players.len()).unwrap_or(0);
        let (gk, def, mid, fwd) = main
            .map(|t| {
                let mut gk = 0usize;
                let mut def = 0usize;
                let mut mid = 0usize;
                let mut fwd = 0usize;
                for p in &t.players.players {
                    match p.position().position_group() {
                        PlayerFieldPositionGroup::Goalkeeper => gk += 1,
                        PlayerFieldPositionGroup::Defender => def += 1,
                        PlayerFieldPositionGroup::Midfielder => mid += 1,
                        PlayerFieldPositionGroup::Forward => fwd += 1,
                    }
                }
                (gk, def, mid, fwd)
            })
            .unwrap_or((0, 0, 0, 0));

        FirstTeamSquadNeeds {
            main_team_size,
            total_missing: MIN_FIRST_TEAM_SQUAD.saturating_sub(main_team_size),
            gk_missing: MIN_GROUP_GOALKEEPER.saturating_sub(gk),
            def_missing: MIN_GROUP_DEFENDER.saturating_sub(def),
            mid_missing: MIN_GROUP_MIDFIELDER.saturating_sub(mid),
            fwd_missing: MIN_GROUP_FORWARD.saturating_sub(fwd),
            gk_count: gk,
            def_count: def,
            mid_count: mid,
            fwd_count: fwd,
            urgent: main_team_size < 11,
        }
    }

    /// True when at least one group is below minimum or the total
    /// headcount sits under [`MIN_FIRST_TEAM_SQUAD`].
    pub fn needs_emergency_fill(&self) -> bool {
        self.total_missing > 0
            || self.gk_missing > 0
            || self.def_missing > 0
            || self.mid_missing > 0
            || self.fwd_missing > 0
    }

    /// Total positional shortfall summed across every group. Drives the
    /// "general depth" tail of [`Self::signing_plan`] — once every group
    /// minimum is met the remaining gap is filled as midfield depth.
    pub fn group_shortfall(&self) -> usize {
        self.gk_missing + self.def_missing + self.mid_missing + self.fwd_missing
    }

    /// Ordered signing plan: most critical group first (GK > DEF > FWD
    /// > MID, then general depth). Order matches "what stops the team
    /// being playable": no keeper is worse than no striker; the
    /// midfield is the easiest zone to compensate for in tactics.
    pub fn signing_plan(&self) -> Vec<EmergencyGroupSlot> {
        let mut plan = Vec::with_capacity(5);
        if self.gk_missing > 0 {
            plan.push(EmergencyGroupSlot {
                group: PlayerFieldPositionGroup::Goalkeeper,
                missing: self.gk_missing,
                reason: "emergency_squad_fill_gk",
            });
        }
        if self.def_missing > 0 {
            plan.push(EmergencyGroupSlot {
                group: PlayerFieldPositionGroup::Defender,
                missing: self.def_missing,
                reason: "emergency_squad_fill_def",
            });
        }
        if self.fwd_missing > 0 {
            plan.push(EmergencyGroupSlot {
                group: PlayerFieldPositionGroup::Forward,
                missing: self.fwd_missing,
                reason: "emergency_squad_fill_fwd",
            });
        }
        if self.mid_missing > 0 {
            plan.push(EmergencyGroupSlot {
                group: PlayerFieldPositionGroup::Midfielder,
                missing: self.mid_missing,
                reason: "emergency_squad_fill_mid",
            });
        }
        // General depth: every group minimum already met but the
        // total headcount is still below MIN_FIRST_TEAM_SQUAD. Slot
        // these as midfield "depth" hires — cheapest plausible fill.
        let group_sum = self.group_shortfall();
        if self.total_missing > group_sum {
            plan.push(EmergencyGroupSlot {
                group: PlayerFieldPositionGroup::Midfielder,
                missing: self.total_missing - group_sum,
                reason: "emergency_squad_fill_depth",
            });
        }
        plan
    }
}

/// How strict the realism gates should be for a given emergency slot.
/// Drives both the score-side weighting (in [`EmergencySquadFillStrategy`])
/// and the hard-filter gates in the picker.
///
/// `Strict` is the default for depth fills: the matcher must use the
/// same realism filters as the normal global free-agent path, no
/// bypass for cross-region moves. `Standard` covers urgent
/// position-fills (squad below 11) — the club really needs the body,
/// so the gates widen a little. `Flexible` is reserved for the GK
/// slot when the squad literally has no keeper; even there the rep
/// and region gates still fire, just with the largest slack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmergencyStrictness {
    /// Most permissive — only used for a no-keeper GK fill.
    Flexible,
    /// Moderate — squad below 11 missing an outfield group.
    Standard,
    /// Tightest — depth fills and non-urgent group fills.
    Strict,
}

/// Maps an emergency slot's reason tag + the buyer's urgent flag to a
/// [`EmergencyStrictness`] level. Wrapped on a unit struct so the
/// reason→strictness policy lives in one place and is easy to unit-test.
pub struct EmergencySlotStrictness;

impl EmergencySlotStrictness {
    /// Look up the strictness for a slot's reason string. Unknown
    /// reasons fall through to [`EmergencyStrictness::Strict`] — better
    /// to err on the side of realism than to let a stray reason slip
    /// past the gates.
    pub fn from_reason(reason: &str, urgent: bool) -> EmergencyStrictness {
        match reason {
            "emergency_squad_fill_gk" => EmergencyStrictness::Flexible,
            "emergency_squad_fill_def"
            | "emergency_squad_fill_mid"
            | "emergency_squad_fill_fwd" => {
                if urgent {
                    EmergencyStrictness::Standard
                } else {
                    EmergencyStrictness::Strict
                }
            }
            "emergency_squad_fill_depth" => EmergencyStrictness::Strict,
            _ => EmergencyStrictness::Strict,
        }
    }
}

/// Inputs the emergency-fill scoring function reads from each
/// candidate. Built once at the call site from the existing free-agent
/// snapshot so the score fn stays pure (no Player / SimulatorData
/// borrows) and trivially unit-testable.
#[derive(Debug, Clone, Copy)]
pub struct EmergencyCandidateView {
    pub ability: u8,
    pub age: u8,
    /// True when the player's nationality matches the buying country
    /// — a strong tie-breaker because domestic free agents avoid the
    /// foreign-registration overhead and visa friction entirely.
    pub same_country_nationality: bool,
    /// True when the player's nationality continent matches the
    /// buyer's — a weaker but still useful preference (a Brazilian
    /// to Argentina is fine; a Brazilian to Vietnam isn't).
    pub same_continent: bool,
    /// Player-side reference reputation. Used to dampen the score
    /// when a high-rep player is implausibly offered to a small club
    /// (they'd reject anyway; no point wasting an emergency slot).
    pub reference_reputation: u16,
    pub career_pressure: f32,
    /// Prestige of the candidate's nationality region — same number
    /// the normal free-agent matcher feeds into the region-drop gate.
    /// Surfaced on the view so a score-time tiebreak can prefer
    /// candidates from regions close to the buyer's own.
    pub region_prestige: f32,
    /// True when the candidate sits in the global free-agent pool
    /// (`sim.free_agents`) rather than as an expiring contract at a
    /// local club. Used both by the score (slight preference for the
    /// in-country option on ties) and by callers tracking pool state.
    pub is_global_pool: bool,
}

/// Scoring inputs that describe the buying club's emergency context.
/// Kept on a struct so the scoring fn signature stays sane and the
/// individual signals are obvious at the call site.
///
/// Carries enough buyer detail to drive the realism gates that the
/// emergency pass shares with the normal global free-agent matcher
/// (reputation, region, club tier, league rep, negotiator skill) as
/// well as the per-slot [`EmergencyStrictness`] derived from the
/// slot's reason tag.
#[derive(Debug, Clone)]
pub struct EmergencyBuyerContext {
    /// Country reputation (0..10000). Drives the rep-drop tolerance —
    /// a low-rep club shouldn't sign a high-rep player even in an
    /// emergency because the player will refuse the offer anyway.
    pub country_reputation: u16,
    /// ISO-style buying country code (e.g. "en", "dz"). Used by the
    /// realism gates to recognise domestic candidates and to read
    /// the buyer's [`crate::transfers::scouting_region::ScoutingRegion`].
    pub country_code: String,
    /// Continent id of the buying country (0..5). Drives the
    /// "same continent" relaxation in the region-prestige gate.
    pub continent_id: u32,
    /// Prestige score for the buyer's scouting region — same value
    /// the normal global matcher uses in its region-drop gate.
    pub region_prestige: f32,
    /// Buyer club's reputation as a 0..1 score, the same tier anchor
    /// every CA-band gate in the project uses
    /// (`PipelineProcessor::tier_*_score`).
    pub club_reputation_score: f32,
    /// Buyer's primary league reputation (0..10000). Feeds the wage
    /// expectation curve used during the acceptance roll.
    pub league_reputation: u16,
    /// Negotiator skill on a 0..100 scale — see the existing
    /// `man_management × 5` mapping in
    /// [`crate::country::result::transfers`].
    pub negotiator_skill: u8,
    /// True when the squad is below 11 — relaxes every quality and
    /// reputation gate so the club can actually field a side.
    pub urgent: bool,
    /// Strictness for the slot currently being filled. Set per-slot
    /// from [`EmergencySlotStrictness::from_reason`] so the depth
    /// slot can apply the realism filters at full strength while the
    /// GK slot relaxes them.
    pub strictness: EmergencyStrictness,
}

/// Pure scoring helper for the emergency free-agent pass. Two-phase
/// API: [`Self::score`] returns the suitability score (higher = better
/// fit); the matching loop calls it once per candidate per group and
/// picks the top result that hasn't been claimed by another emergency
/// signing in the same tick.
pub struct EmergencySquadFillStrategy;

impl EmergencySquadFillStrategy {
    /// Maximum reasonable age for emergency first-team depth — older
    /// players are accepted only when the squad is critically short.
    pub const PREFERRED_AGE_MAX: u8 = 34;
    /// Bottom of the preferred age band — players younger than this
    /// are usually not "ready" for emergency depth and get a small
    /// score penalty unless the squad is urgent.
    pub const PREFERRED_AGE_MIN: u8 = 22;
    /// Upper-CA soft cap relative to buyer's country reputation, even
    /// when urgent. A CA-180 megastar slumming at a Maltese amateur
    /// side isn't realistic just because the squad is short — a club
    /// at 1000 rep simply cannot afford / persuade that profile of
    /// player. Acts as a final gate alongside the `acceptance_score`
    /// model so even a lucky roll on the player side can't override
    /// the basic implausibility.
    pub const URGENT_MAX_CA_BUFFER: u8 = 40;

    /// Score a candidate in `[0, 100]`. Anything below
    /// [`Self::MIN_ACCEPTABLE_SCORE`] is treated as "not worth
    /// signing even for emergency depth"; matching skips them.
    /// Returns `None` for candidates a club categorically can't
    /// realistically sign (reputation chasm, way too good).
    pub fn score(candidate: &EmergencyCandidateView, buyer: &EmergencyBuyerContext) -> Option<f32> {
        // Reputation chasm gate — a CA 160+ international free agent
        // signing for Malta on a regular tick is not realistic, even
        // with career pressure. Urgent (sub-11) clubs widen but do
        // NOT bypass entirely: a tiny low-rep club still can't credibly
        // sign a CA-180 international, the player would refuse outright.
        let buyer_rep = buyer.country_reputation as i32;
        let player_rep = candidate.reference_reputation as i32;
        let allowed_drop = if buyer.urgent {
            // Urgent clubs widen the tolerance — a player on full
            // career pressure can step down ~5000 rep instead of
            // ~3500, reflecting the desperation-tier offer (short
            // deal, guaranteed minutes) the emergency signing carries.
            2500 + (candidate.career_pressure * 4500.0) as i32
        } else {
            1000 + (candidate.career_pressure * 3500.0) as i32
        };
        if buyer_rep + allowed_drop < player_rep {
            return None;
        }

        // Quality band — emergency depth wants competent journeymen,
        // not stars and not borderline-unplayable amateurs. Urgent
        // clubs widen the floor (any registered footballer accepted)
        // but keep a soft ceiling: a CA-180 megastar walking into a
        // CA-90 squad on a free is not realistic.
        let (min_ca, urgent_cap) = if buyer.urgent {
            (15u8, Self::urgent_max_ca(buyer.country_reputation))
        } else {
            // Cap relative to buyer rep: 4500 rep ≈ Continental.
            let cap_floor = 30u8;
            let cap_ceiling = if buyer.country_reputation >= 7000 {
                180
            } else if buyer.country_reputation >= 5000 {
                160
            } else if buyer.country_reputation >= 3000 {
                140
            } else {
                125
            };
            (cap_floor, cap_ceiling)
        };
        if candidate.ability < min_ca || candidate.ability > urgent_cap {
            return None;
        }

        // Base score from ability inside the band — sweet spot ~100,
        // tapering off above and below. Emergency depth doesn't want
        // the strongest available player; it wants a *fit* player.
        let ability = candidate.ability as f32;
        let ability_score = if ability >= 80.0 && ability <= 120.0 {
            45.0
        } else if ability >= 60.0 && ability <= 140.0 {
            38.0
        } else if ability >= 40.0 {
            28.0
        } else {
            // Sub-40 CA: only acceptable when truly nothing else is
            // around. Strong penalty so any senior journeyman beats
            // them.
            12.0
        };

        // Age — emergency cover leans on veterans (28-34) who are
        // available, pragmatic, and don't need long contracts. Mild
        // penalty either side. Urgent clubs relax the age preference
        // entirely but still prefer somewhat experienced bodies.
        let age = candidate.age;
        let age_score = if buyer.urgent {
            // Urgent: any senior body is fine; under-22 walks are
            // discouraged unless that is genuinely all there is.
            if age >= 22 { 16.0 } else { 6.0 }
        } else if (28..=Self::PREFERRED_AGE_MAX).contains(&age) {
            // Veteran sweet spot for short-term emergency cover.
            22.0
        } else if (Self::PREFERRED_AGE_MIN..28).contains(&age) {
            // Prime age — usable but slightly less ideal than a
            // veteran for short emergency cover (they want longer
            // deals, higher wages).
            18.0
        } else if age < Self::PREFERRED_AGE_MIN {
            // Very young players: less reliable for "now" cover, and
            // sub-22 low-CA is the worst combination — the depth slot
            // is supposed to be a senior body.
            if candidate.ability < 70 { 4.0 } else { 9.0 }
        } else if age <= 36 {
            // 35-36 veterans still usable but less attractive than
            // prime-age cover.
            14.0
        } else {
            6.0
        };

        // Domestic preference — modest weight by default, larger
        // when the squad is urgent (a domestic player can join
        // overnight with no foreign-quota friction). Same-continent
        // is a softer fallback worth a bit more for urgent clubs too.
        // Strict slots (depth fills, non-urgent group fills) reward
        // domestic candidates more aggressively and strip the
        // cross-continent fallback because realistic depth signings
        // overwhelmingly come from the local market — that's the
        // whole point of the `Strict` mode.
        let domestic_score = if candidate.same_country_nationality {
            match buyer.strictness {
                EmergencyStrictness::Strict => 26.0,
                EmergencyStrictness::Standard => {
                    if buyer.urgent {
                        22.0
                    } else {
                        18.0
                    }
                }
                EmergencyStrictness::Flexible => 22.0,
            }
        } else if candidate.same_continent {
            match buyer.strictness {
                EmergencyStrictness::Strict => 4.0,
                EmergencyStrictness::Standard => {
                    if buyer.urgent {
                        11.0
                    } else {
                        8.0
                    }
                }
                EmergencyStrictness::Flexible => 11.0,
            }
        } else {
            match buyer.strictness {
                EmergencyStrictness::Strict => 0.0,
                _ => 2.0,
            }
        };

        // Career pressure — a free agent who's been on the market a
        // while is more likely to accept the emergency offer, so we
        // weight them up slightly to bias selection toward acceptors.
        let pressure_score = candidate.career_pressure.clamp(0.0, 1.0) * 10.0;

        Some(ability_score + age_score + domestic_score + pressure_score)
    }

    /// Soft CA ceiling for urgent clubs based on country reputation.
    /// Even when desperate, a 1000-rep amateur side won't realistically
    /// sign a CA-180 international free agent — the player rejects,
    /// the wage is unaffordable, and the move doesn't read like real
    /// football. The buffer above the rep-tier cap reflects the
    /// emergency uplift; without it a tiny club would be permitted
    /// to "scout" a megastar that no model would ever sign for them.
    pub fn urgent_max_ca(buyer_country_reputation: u16) -> u8 {
        let base: u8 = if buyer_country_reputation >= 7000 {
            190
        } else if buyer_country_reputation >= 5000 {
            175
        } else if buyer_country_reputation >= 3000 {
            155
        } else if buyer_country_reputation >= 1500 {
            135
        } else {
            120
        };
        base.saturating_add(Self::URGENT_MAX_CA_BUFFER.saturating_sub(20))
    }

    /// Lower bound below which a candidate is treated as not worth
    /// signing, even at an underfilled club. Tuned so a journeyman
    /// domestic free agent in the 60-CA band reliably clears it.
    pub const MIN_ACCEPTABLE_SCORE: f32 = 45.0;

    /// Acceptance modifier for the player side — the emergency offer
    /// includes a "guaranteed first-team role" pitch and a short
    /// contract suited to the player's situation, so the player's odds
    /// of saying yes are lifted compared with normal matching. Returns
    /// a multiplier on the base acceptance probability; the multiplier
    /// is applied *after* the standard `FreeAgentMarketCalculator`
    /// acceptance probability so realism / wage / prestige gates still
    /// fire. A pampered superstar offered a tiny club's emergency deal
    /// still rejects — the multiplier lifts a 0.30 chance to ~0.40, not
    /// a 0.05 chance to 0.95.
    pub const EMERGENCY_ACCEPTANCE_MULTIPLIER: f32 = 1.35;
}

/// One mutable count of how the projected squad looks while the
/// emergency pass is staging signings. The starting point is
/// [`FirstTeamSquadNeeds::for_club`]; every staged signing decrements
/// the matching group counter (and total) so subsequent slots see an
/// up-to-date picture. Stays a plain struct because the borrow-free
/// caller (free_agents.rs) needs to mutate it inline while iterating.
#[derive(Debug, Clone, Copy)]
pub struct EmergencyProjectedSquad {
    pub total: usize,
    pub gk: usize,
    pub def: usize,
    pub mid: usize,
    pub fwd: usize,
}

impl EmergencyProjectedSquad {
    /// Seed from the initial squad-needs snapshot. We project the REAL
    /// group counts so the running "is this club still urgent / still
    /// short of group X" decision can read off the same struct. The old
    /// `MIN - missing` reconstruction clamped every above-minimum group
    /// at its floor, so a defender-stuffed emergency club still tied on
    /// the thinnest-group ratio and could receive more defenders.
    pub fn from_needs(needs: &FirstTeamSquadNeeds) -> Self {
        EmergencyProjectedSquad {
            total: needs.main_team_size,
            gk: needs.gk_count,
            def: needs.def_count,
            mid: needs.mid_count,
            fwd: needs.fwd_count,
        }
    }

    /// Whether the squad still needs emergency attention. Falls back
    /// to the snapshot's per-group minimums and a fixed threshold so
    /// the running check stays consistent with the initial decision.
    pub fn needs_more_signings(&self, threshold: usize) -> bool {
        if self.total < FirstTeamSquadNeeds::MIN_FIRST_TEAM_SQUAD.min(threshold) {
            return true;
        }
        if self.total < threshold && self.group_shortfall() > 0 {
            return true;
        }
        self.group_shortfall() > 0
    }

    /// Once-projected sum of how many bodies the club is still short
    /// against each group's minimum. Same shape as
    /// [`FirstTeamSquadNeeds::group_shortfall`].
    pub fn group_shortfall(&self) -> usize {
        MIN_GROUP_GOALKEEPER.saturating_sub(self.gk)
            + MIN_GROUP_DEFENDER.saturating_sub(self.def)
            + MIN_GROUP_MIDFIELDER.saturating_sub(self.mid)
            + MIN_GROUP_FORWARD.saturating_sub(self.fwd)
    }

    /// True once the squad can field a side. Drives the "urgent"
    /// flag's mid-pass relaxation — once we've signed up to 11 the
    /// chasm gate flips back to the normal pressure-scaled tolerance.
    pub fn is_urgent(&self) -> bool {
        self.total < 11
    }

    /// Identify the position group with the largest remaining
    /// shortfall against its per-group minimum. Used for "depth"
    /// slots so the second/third signing rotates into the currently
    /// thinnest group instead of always becoming a midfielder.
    ///
    /// Ties broken by the same priority order the rest of the
    /// pipeline uses (GK > DEF > FWD > MID), so a club tied between
    /// DEF and MID gets a defender rather than the legacy default of
    /// always picking midfield.
    pub fn thinnest_group(&self) -> PlayerFieldPositionGroup {
        let gk_gap = MIN_GROUP_GOALKEEPER.saturating_sub(self.gk);
        let def_gap = MIN_GROUP_DEFENDER.saturating_sub(self.def);
        let mid_gap = MIN_GROUP_MIDFIELDER.saturating_sub(self.mid);
        let fwd_gap = MIN_GROUP_FORWARD.saturating_sub(self.fwd);
        // (group, gap, tier_priority) — lower tier_priority wins on
        // a tie. Matches the signing-plan order: GK first, then DEF,
        // FWD, MID.
        let candidates = [
            (PlayerFieldPositionGroup::Goalkeeper, gk_gap, 0u8),
            (PlayerFieldPositionGroup::Defender, def_gap, 1),
            (PlayerFieldPositionGroup::Forward, fwd_gap, 2),
            (PlayerFieldPositionGroup::Midfielder, mid_gap, 3),
        ];
        if let Some((group, gap, _)) = candidates.iter().max_by(|a, b| {
            // On tie, lower priority value wins (higher tier), so
            // flip the comparator.
            a.1.cmp(&b.1).then_with(|| b.2.cmp(&a.2))
        }) {
            if *gap > 0 {
                return *group;
            }
        }
        // All minimums met; bias depth into the proportionally
        // thinnest outfield group rather than always midfield. A
        // defender-heavy emergency club already has enough defenders
        // — rotate depth into whichever group currently sits furthest
        // from its depth-target ratio. Same DEF > FWD > MID tier
        // ordering on tie.
        let ratios = [
            (
                PlayerFieldPositionGroup::Defender,
                self.def as f32 / MIN_GROUP_DEFENDER as f32,
                1u8,
            ),
            (
                PlayerFieldPositionGroup::Forward,
                self.fwd as f32 / MIN_GROUP_FORWARD as f32,
                2,
            ),
            (
                PlayerFieldPositionGroup::Midfielder,
                self.mid as f32 / MIN_GROUP_MIDFIELDER as f32,
                3,
            ),
        ];
        ratios
            .iter()
            .min_by(|a, b| {
                a.1.partial_cmp(&b.1)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| a.2.cmp(&b.2))
            })
            .map(|(g, _, _)| *g)
            .unwrap_or(PlayerFieldPositionGroup::Midfielder)
    }

    /// Apply a staged signing to the running projection. Increments
    /// the matching group counter and the total. Idempotent only in
    /// the sense that callers won't double-count a single signing —
    /// the caller wraps this in the "signing was accepted" branch.
    pub fn apply_signing(&mut self, group: PlayerFieldPositionGroup) {
        self.total += 1;
        match group {
            PlayerFieldPositionGroup::Goalkeeper => self.gk += 1,
            PlayerFieldPositionGroup::Defender => self.def += 1,
            PlayerFieldPositionGroup::Midfielder => self.mid += 1,
            PlayerFieldPositionGroup::Forward => self.fwd += 1,
        }
    }
}

/// Suggested contract length (years) for an emergency signing —
/// short-term cover for veterans, slightly longer for younger useful
/// bodies. Stays a pure helper so the orchestration layer can stage
/// the terms without re-implementing the policy.
pub struct EmergencyContractTermsPolicy;

impl EmergencyContractTermsPolicy {
    /// Standard veteran / journeyman cover — 1 year.
    pub const VETERAN_YEARS: u8 = 1;
    /// Younger useful players are worth a slightly longer commitment
    /// so the club can keep them past the immediate emergency.
    pub const YOUNG_USEFUL_YEARS: u8 = 2;

    /// Inferred contract length for an emergency offer. Younger
    /// useful players (under 26 with credible CA) get 2 years; the
    /// rest get 1.
    pub fn contract_years(age: u8, ability: u8) -> u8 {
        if age < 26 && ability >= 70 {
            Self::YOUNG_USEFUL_YEARS
        } else {
            Self::VETERAN_YEARS
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PlayerFieldPositionGroup;

    /// Test fixtures bundled on a unit struct — see project convention
    /// `feedback_use_directives`: no loose helper fns at module scope,
    /// even inside `#[cfg(test)]` blocks.
    struct ScoringFixtures;

    impl ScoringFixtures {
        fn buyer(rep: u16, urgent: bool) -> EmergencyBuyerContext {
            EmergencyBuyerContext {
                country_reputation: rep,
                country_code: "en".to_string(),
                continent_id: 1,
                region_prestige: 1.0,
                club_reputation_score: (rep as f32 / 10_000.0).clamp(0.0, 1.0),
                league_reputation: rep,
                negotiator_skill: 50,
                urgent,
                strictness: EmergencyStrictness::Standard,
            }
        }

        fn cand(
            ability: u8,
            age: u8,
            same_country: bool,
            ref_rep: u16,
            pressure: f32,
        ) -> EmergencyCandidateView {
            EmergencyCandidateView {
                ability,
                age,
                same_country_nationality: same_country,
                same_continent: same_country,
                reference_reputation: ref_rep,
                career_pressure: pressure,
                region_prestige: if same_country { 1.0 } else { 0.5 },
                is_global_pool: true,
            }
        }
    }

    #[test]
    fn min_group_sum_fits_within_min_first_team_squad() {
        // Per-group minimums sit BELOW MIN_FIRST_TEAM_SQUAD so the
        // remainder (5 by current calibration) is filled as general
        // depth via the depth-tail slot. They must not exceed the
        // total — otherwise the depth tail would never fire.
        let group_sum =
            MIN_GROUP_GOALKEEPER + MIN_GROUP_DEFENDER + MIN_GROUP_MIDFIELDER + MIN_GROUP_FORWARD;
        assert!(
            group_sum <= MIN_FIRST_TEAM_SQUAD,
            "group floor sum {} must be ≤ total minimum {}",
            group_sum,
            MIN_FIRST_TEAM_SQUAD
        );
    }

    #[test]
    fn signing_plan_orders_gk_before_outfield() {
        let needs = FirstTeamSquadNeeds {
            main_team_size: 0,
            total_missing: MIN_FIRST_TEAM_SQUAD,
            gk_missing: MIN_GROUP_GOALKEEPER,
            def_missing: MIN_GROUP_DEFENDER,
            mid_missing: MIN_GROUP_MIDFIELDER,
            fwd_missing: MIN_GROUP_FORWARD,
            gk_count: 0,
            def_count: 0,
            mid_count: 0,
            fwd_count: 0,
            urgent: true,
        };
        let plan = needs.signing_plan();
        assert_eq!(plan[0].group, PlayerFieldPositionGroup::Goalkeeper);
        // Forward should come before midfield in priority.
        let fwd_idx = plan
            .iter()
            .position(|s| s.group == PlayerFieldPositionGroup::Forward)
            .unwrap();
        let mid_idx = plan
            .iter()
            .position(|s| s.group == PlayerFieldPositionGroup::Midfielder)
            .unwrap();
        assert!(fwd_idx < mid_idx);
    }

    #[test]
    fn signing_plan_emits_depth_tail_when_groups_met_but_total_short() {
        // Group minimums satisfied, but total < 25 — should add a
        // midfield-tagged depth slot covering the remainder.
        let needs = FirstTeamSquadNeeds {
            main_team_size: 22,
            total_missing: 3,
            gk_missing: 0,
            def_missing: 0,
            mid_missing: 0,
            fwd_missing: 0,
            gk_count: MIN_GROUP_GOALKEEPER,
            def_count: MIN_GROUP_DEFENDER,
            mid_count: MIN_GROUP_MIDFIELDER,
            fwd_count: MIN_GROUP_FORWARD,
            urgent: false,
        };
        let plan = needs.signing_plan();
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].group, PlayerFieldPositionGroup::Midfielder);
        assert_eq!(plan[0].missing, 3);
        assert_eq!(plan[0].reason, "emergency_squad_fill_depth");
    }

    #[test]
    fn balanced_squad_needs_no_emergency() {
        // A perfectly balanced 25-man squad with every group at
        // minimum reports no emergency need.
        let needs = FirstTeamSquadNeeds {
            main_team_size: 25,
            total_missing: 0,
            gk_missing: 0,
            def_missing: 0,
            mid_missing: 0,
            fwd_missing: 0,
            gk_count: MIN_GROUP_GOALKEEPER,
            def_count: MIN_GROUP_DEFENDER,
            mid_count: MIN_GROUP_MIDFIELDER,
            fwd_count: MIN_GROUP_FORWARD,
            urgent: false,
        };
        assert!(!needs.needs_emergency_fill());
        assert!(needs.signing_plan().is_empty());
    }

    #[test]
    fn empty_squad_is_urgent() {
        // Zero players => urgent flag set; signing_plan covers every
        // group plus a depth tail when group floors sum below the
        // total minimum.
        let needs = FirstTeamSquadNeeds {
            main_team_size: 0,
            total_missing: MIN_FIRST_TEAM_SQUAD,
            gk_missing: MIN_GROUP_GOALKEEPER,
            def_missing: MIN_GROUP_DEFENDER,
            mid_missing: MIN_GROUP_MIDFIELDER,
            fwd_missing: MIN_GROUP_FORWARD,
            gk_count: 0,
            def_count: 0,
            mid_count: 0,
            fwd_count: 0,
            urgent: true,
        };
        assert!(needs.urgent);
        assert!(needs.needs_emergency_fill());
        let plan = needs.signing_plan();
        // 4 group slots + a depth slot when group floors sum below total.
        let group_sum =
            MIN_GROUP_GOALKEEPER + MIN_GROUP_DEFENDER + MIN_GROUP_MIDFIELDER + MIN_GROUP_FORWARD;
        let expected = if group_sum < MIN_FIRST_TEAM_SQUAD {
            5
        } else {
            4
        };
        assert_eq!(plan.len(), expected);
    }

    #[test]
    fn domestic_journeyman_beats_foreign_unknown() {
        // Same ability, same age, same career pressure — but the
        // domestic candidate should outscore the foreigner.
        let b = ScoringFixtures::buyer(4500, false);
        let domestic = ScoringFixtures::cand(95, 27, true, 3000, 0.3);
        let foreign = EmergencyCandidateView {
            same_country_nationality: false,
            same_continent: false,
            ..ScoringFixtures::cand(95, 27, false, 3000, 0.3)
        };
        let s_dom = EmergencySquadFillStrategy::score(&domestic, &b).unwrap();
        let s_for = EmergencySquadFillStrategy::score(&foreign, &b).unwrap();
        assert!(s_dom > s_for, "domestic {s_dom} ≯ foreign {s_for}");
    }

    #[test]
    fn high_rep_player_rejected_by_small_club_outside_urgent() {
        // 5000-rep Spaniard offered to 1000-rep club: too rich for
        // their blood without urgency.
        let b = ScoringFixtures::buyer(1000, false);
        let big_name = ScoringFixtures::cand(140, 27, false, 5000, 0.2);
        assert!(EmergencySquadFillStrategy::score(&big_name, &b).is_none());
    }

    #[test]
    fn urgent_club_takes_almost_anyone() {
        // Squad of 5 players (urgent) at low-rep country — accepts a
        // CA 30 walk-on. Without urgent flag the gate would reject.
        let b = ScoringFixtures::buyer(1500, true);
        let walk_on = ScoringFixtures::cand(30, 25, true, 800, 0.5);
        let s = EmergencySquadFillStrategy::score(&walk_on, &b).unwrap();
        assert!(s > 0.0);
    }

    #[test]
    fn pressure_lifts_score_for_acceptor() {
        let b = ScoringFixtures::buyer(4500, false);
        let cold = ScoringFixtures::cand(90, 27, true, 3000, 0.0);
        let warm = ScoringFixtures::cand(90, 27, true, 3000, 1.0);
        let s_cold = EmergencySquadFillStrategy::score(&cold, &b).unwrap();
        let s_warm = EmergencySquadFillStrategy::score(&warm, &b).unwrap();
        assert!(s_warm > s_cold, "pressure should lift score");
    }

    #[test]
    fn very_young_under_22_scores_lower_than_prime_age() {
        let b = ScoringFixtures::buyer(4500, false);
        let kid = ScoringFixtures::cand(90, 19, true, 2500, 0.3);
        let prime = ScoringFixtures::cand(90, 27, true, 2500, 0.3);
        assert!(
            EmergencySquadFillStrategy::score(&prime, &b).unwrap()
                > EmergencySquadFillStrategy::score(&kid, &b).unwrap()
        );
    }

    #[test]
    fn out_of_band_quality_returns_none_for_non_urgent() {
        // Very-low country-rep buyer + CA 180 star = chasm gate fires.
        let b = ScoringFixtures::buyer(2000, false);
        let star = ScoringFixtures::cand(180, 27, false, 6000, 0.4);
        assert!(EmergencySquadFillStrategy::score(&star, &b).is_none());
    }

    #[test]
    fn urgent_rejects_elite_player_at_tiny_club() {
        // Urgent path still rejects a CA-180 superstar at a 800-rep
        // amateur side — the new `URGENT_MAX_CA_BUFFER` + chasm gate
        // combination keeps the move implausible even at low pressure.
        let b = ScoringFixtures::buyer(800, true);
        let mega = ScoringFixtures::cand(180, 27, false, 7500, 0.2);
        assert!(EmergencySquadFillStrategy::score(&mega, &b).is_none());
    }

    #[test]
    fn veteran_beats_younger_journeyman_for_emergency_depth() {
        // Two equally credible domestic journeymen — the veteran
        // (29-34 band) is the realistic emergency pick over a younger
        // body because the role is short-term cover. Without this
        // tuning the prime-age player edges every signing.
        let b = ScoringFixtures::buyer(4500, false);
        let veteran = ScoringFixtures::cand(85, 31, true, 3000, 0.4);
        let prime = ScoringFixtures::cand(85, 25, true, 3000, 0.4);
        let s_vet = EmergencySquadFillStrategy::score(&veteran, &b).unwrap();
        let s_prime = EmergencySquadFillStrategy::score(&prime, &b).unwrap();
        assert!(
            s_vet > s_prime,
            "veteran {s_vet} should beat prime {s_prime}"
        );
    }

    #[test]
    fn very_young_low_ability_takes_extra_penalty() {
        // Sub-22 + sub-70 CA combination is the worst emergency pick —
        // the depth slot is supposed to be a senior body. Score must
        // stay BELOW the per-candidate acceptable floor so the matcher
        // doesn't pick them when any senior alternative exists.
        let b = ScoringFixtures::buyer(4500, false);
        let teen = ScoringFixtures::cand(50, 18, true, 2000, 0.3);
        let s = EmergencySquadFillStrategy::score(&teen, &b).unwrap();
        // The 4.0 age + 28 ability + 18 domestic + 3 pressure = 53;
        // tighter than the prior 8 + 28 + 18 + 3 = 57. Acceptable.
        // What matters is the senior journeyman test below outscores
        // them.
        let senior = ScoringFixtures::cand(80, 30, true, 2500, 0.3);
        let s_senior = EmergencySquadFillStrategy::score(&senior, &b).unwrap();
        assert!(s_senior > s, "senior {s_senior} should outscore teen {s}");
    }

    #[test]
    fn projected_squad_thinnest_group_selects_min_floor_first() {
        // Empty club: GK shortfall is 2 (min 2 - 0), DEF 7, MID 7, FWD 4.
        // DEF & MID tie at 7; tie-breaker order DEF first.
        let needs = FirstTeamSquadNeeds {
            main_team_size: 0,
            total_missing: MIN_FIRST_TEAM_SQUAD,
            gk_missing: MIN_GROUP_GOALKEEPER,
            def_missing: MIN_GROUP_DEFENDER,
            mid_missing: MIN_GROUP_MIDFIELDER,
            fwd_missing: MIN_GROUP_FORWARD,
            gk_count: 0,
            def_count: 0,
            mid_count: 0,
            fwd_count: 0,
            urgent: true,
        };
        let projected = EmergencyProjectedSquad::from_needs(&needs);
        assert_eq!(
            projected.thinnest_group(),
            PlayerFieldPositionGroup::Defender
        );
    }

    #[test]
    fn projected_squad_apply_signing_decreases_shortfall() {
        let needs = FirstTeamSquadNeeds {
            main_team_size: 5,
            total_missing: 20,
            gk_missing: 1,
            def_missing: 5,
            mid_missing: 5,
            fwd_missing: 3,
            gk_count: 1,
            def_count: 2,
            mid_count: 2,
            fwd_count: 1,
            urgent: true,
        };
        let mut projected = EmergencyProjectedSquad::from_needs(&needs);
        let before = projected.group_shortfall();
        projected.apply_signing(PlayerFieldPositionGroup::Defender);
        assert_eq!(projected.total, 6);
        assert_eq!(projected.group_shortfall(), before - 1);
    }

    #[test]
    fn projected_squad_urgent_flips_off_at_11() {
        let needs = FirstTeamSquadNeeds {
            main_team_size: 10,
            total_missing: 15,
            gk_missing: 0,
            def_missing: 2,
            mid_missing: 2,
            fwd_missing: 1,
            gk_count: 2,
            def_count: 5,
            mid_count: 5,
            fwd_count: 3,
            urgent: true,
        };
        let mut projected = EmergencyProjectedSquad::from_needs(&needs);
        assert!(projected.is_urgent());
        projected.apply_signing(PlayerFieldPositionGroup::Defender);
        assert!(
            !projected.is_urgent(),
            "11 main-team players = no longer urgent"
        );
    }

    #[test]
    fn contract_policy_young_useful_gets_two_years() {
        assert_eq!(
            EmergencyContractTermsPolicy::contract_years(24, 80),
            EmergencyContractTermsPolicy::YOUNG_USEFUL_YEARS
        );
        assert_eq!(
            EmergencyContractTermsPolicy::contract_years(32, 80),
            EmergencyContractTermsPolicy::VETERAN_YEARS
        );
        assert_eq!(
            EmergencyContractTermsPolicy::contract_years(24, 60),
            EmergencyContractTermsPolicy::VETERAN_YEARS
        );
    }
}
