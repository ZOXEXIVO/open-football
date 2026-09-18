//! Who the club sends out to play somewhere else this window.
//!
//! One walk over the squad, and for each man a ladder of questions that stops
//! at the first yes: is he loanable at all, where does he sit in his own
//! position group, has he failed to settle abroad, does his club's tier have a
//! reason to lend him out, and — if nothing else fired — is he simply surplus.
//!
//! Every trigger below is graded against the DEPTH CUSHION rather than gated
//! by a rank cut-off: the higher up the pecking order a player sits, the
//! stronger the signal has to be before anything fires, and no rung is ever
//! closed outright. A club's first choice can still go; he just needs a reason
//! the fourth choice would not.

use super::*;
use crate::club::player::mind::MindClock;
use crate::transfers::loan::agreement::ParentWillingness;
use crate::transfers::squad::SquadReviewPass;

/// The loan-out scan.
pub(in crate::transfers::squad) struct LoanOutScan;

/// How aggressively the club's philosophy sends people out on loan.
#[derive(Clone, Copy)]
struct LoanOutBands {
    age_threshold: u8,
    ability_gap: i16,
    min_appearances_pct: u16,
}

/// Where a player sits in his own position group. Read once per player and
/// handed to every trigger, so they all grade him against the same pecking
/// order rather than against the outfield-dominated squad mean.
#[derive(Clone, Copy)]
struct GroupDepth {
    group: PlayerFieldPositionGroup,
    group_count: usize,
    group_avg: u8,
    group_best: u8,
    depth_cushion: i16,
}

impl LoanOutScan {
    /// Identify loan-out candidates based on club reputation tier.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::transfers::squad) fn identify(
        club: &Club,
        squad: &[SquadPlayerInfo],
        rep_level: &ReputationLevel,
        avg_ability: u8,
        date: NaiveDate,
        players: &[Player],
        loan_outs: &mut Vec<LoanOutCandidate>,
        philosophy: &ClubPhilosophy,
        formation_positions: &[PlayerPositionType; 11],
        current_window: Option<(NaiveDate, NaiveDate)>,
        early_season: bool,
        is_january: bool,
        home: &SquadHomeContext<'_>,
        club_reputation_score: f32,
    ) {
        // Philosophy-based loan-out aggressiveness
        let bands = match philosophy {
            ClubPhilosophy::DevelopAndSell => LoanOutBands {
                age_threshold: 21,
                ability_gap: 5,
                min_appearances_pct: 30,
            }, // Aggressively loan young players
            ClubPhilosophy::SignToCompete => LoanOutBands {
                age_threshold: 19,
                ability_gap: 10,
                min_appearances_pct: 20,
            }, // Only loan clearly surplus
            ClubPhilosophy::LoanFocused => LoanOutBands {
                age_threshold: 23,
                ability_gap: 3,
                min_appearances_pct: 40,
            }, // Loan to reduce wages
            ClubPhilosophy::Balanced => LoanOutBands {
                age_threshold: 21,
                ability_gap: 8,
                min_appearances_pct: 25,
            }, // Standard
        };

        for player_info in squad {
            let player = match players.iter().find(|p| p.id == player_info.player_id) {
                Some(p) => p,
                None => continue,
            };

            if Self::blocked(player, player_info, club, date, current_window) {
                continue;
            }

            let Some(depth) =
                Self::group_depth(squad, player_info, formation_positions, avg_ability)
            else {
                continue;
            };

            if Self::unsettled_abroad(
                player,
                player_info,
                date,
                home,
                club_reputation_score,
                formation_positions,
                depth,
                loan_outs,
            ) {
                continue;
            }

            if Self::tier_verdict(
                player_info,
                squad,
                rep_level,
                avg_ability,
                early_season,
                is_january,
                bands,
                depth,
                loan_outs,
            ) {
                continue;
            }

            Self::surplus(player_info, philosophy, is_january, depth, loan_outs);
        }
    }

    /// The reasons a man cannot be loaned out at all.
    ///
    /// Physical only, now: he is already away, he has no contract to
    /// lend, he is with his country, his manager has pinned him, or the
    /// club committed to him this window. Everything that used to sit
    /// here and was a JUDGEMENT rather than a fact — thirty and over,
    /// fifteen appearances, two previous spells, a first-team label, the
    /// club's own first choice — is priced in
    /// [`ParentWillingness`] instead, because each of them is a reason a
    /// club is less likely to lend somebody rather than a reason it
    /// cannot.
    fn blocked(
        player: &Player,
        player_info: &SquadPlayerInfo,
        club: &Club,
        date: NaiveDate,
        current_window: Option<(NaiveDate, NaiveDate)>,
    ) -> bool {
        if player.is_on_loan() || player.contract.is_none() {
            return true;
        }

        // Manager-pinned: never propose a loan-out, regardless of
        // philosophy / playing-time / surplus signals. The pin is the
        // manager's decision; the AI must respect it.
        if player.is_force_match_selection {
            return true;
        }

        // A player away on international duty isn't being benched by a
        // club choice — his low minutes are an artefact of the call-up,
        // not evidence he is unwanted.
        if player.statuses.is_on_international_duty() {
            return true;
        }

        // Same-window protection: the club committed to him weeks ago
        // and has not yet had a chance to be wrong about it. A
        // development pathway is the exception, because loaning him IS
        // the commitment.
        let on_a_development_pathway = player
            .plan
            .as_ref()
            .map(|plan| plan.role == PlayerPlanRole::Development)
            .unwrap_or(false);
        if !on_a_development_pathway {
            if let (Some(transfer_date), Some((window_start, window_end))) =
                (player.last_transfer_date, current_window)
            {
                if transfer_date >= window_start && transfer_date <= window_end {
                    return true;
                }
            }
            // …and the evaluation window it bought him. A club that just
            // signed a man does not lend him out because a depth cap says
            // so.
            if let Some(ref plan) = player.plan {
                let appearances = player.statistics.played + player.statistics.played_subs;
                if !plan.is_evaluated(date, appearances) && !plan.is_expired(date) {
                    return true;
                }
            }
        }

        // And the one reading that is about the club's position rather
        // than the player's paperwork: below this it is not refusing a
        // destination, it is refusing the conversation. His own side of
        // it — a request, a listing, a plan pushing for a season away —
        // lifts the hold his standing would otherwise put on him.
        let opened = player.statuses.has(PlayerStatusType::Req)
            || player.statuses.has(PlayerStatusType::Loa)
            || player
                .mind
                .career
                .plan_view(MindClock::day(date))
                .loan_push()
                >= ParentWillingness::PLAN_OPENS_AT;
        let willingness = ParentWillingness::held_as_first_team(
            LoanAssetGuard::willingness_for(club, player, date),
            player_info.asset_class.is_first_team_protected(),
            opened,
        );
        willingness < ParentWillingness::ENTERTAINS
    }

    /// His place in the pecking order, and the cushion it buys him. `None`
    /// means the group is already at the formation's floor, so nobody in it
    /// can leave whatever else is true of him.
    fn group_depth(
        squad: &[SquadPlayerInfo],
        player_info: &SquadPlayerInfo,
        formation_positions: &[PlayerPositionType; 11],
        avg_ability: u8,
    ) -> Option<GroupDepth> {
        let group = player_info.primary_position.position_group();

        // Count players in same position group
        let group_count = squad
            .iter()
            .filter(|p| p.primary_position.position_group() == group)
            .count();

        // Minimum players needed per group from formation
        let min_needed = SquadReviewPass::group_min_needed(group, formation_positions);

        // Don't loan out if we'd drop below minimum
        if group_count <= min_needed {
            return None;
        }

        // Depth-chart position in the player's group. Used later as
        // a graduated resistance — the higher up the pecking order
        // a player sits, the harder it is for any loan-out trigger
        // to fire. No hard cut-off: an utterly surplus #1 can still
        // go, it just needs much stronger signals than the 4th-choice
        // would to get there.
        let mut group_ranks: Vec<(u32, u8)> = squad
            .iter()
            .filter(|p| p.primary_position.position_group() == group)
            .map(|p| (p.player_id, p.current_ability))
            .collect();
        group_ranks.sort_by(|a, b| b.1.cmp(&a.1));
        let rank = group_ranks
            .iter()
            .position(|(pid, _)| *pid == player_info.player_id)
            .unwrap_or(usize::MAX);
        // Position-group average CA — compares the player to their own
        // role peer group rather than the outfield-dominated starting
        // XI mean (which quietly branded first-choice keepers "below
        // average" and kept shipping them out).
        let group_avg: u8 = if !group_ranks.is_empty() {
            let sum: u32 = group_ranks.iter().map(|(_, ca)| *ca as u32).sum();
            (sum / group_ranks.len() as u32) as u8
        } else {
            avg_ability
        };
        // The best man in his position — what "blocked" is measured
        // against. A group average says nothing about whether the man
        // ahead of him is going to move.
        let group_best: u8 = group_ranks
            .first()
            .map(|(_, ca)| *ca)
            .unwrap_or(player_info.current_ability);
        // Depth cushion: extra CA-below-group-average the player needs
        // to exceed before any "surplus / lack of minutes" branch will
        // fire. Halved from the ladder it replaced — the hold on a
        // club's own first choice is priced in [`ParentWillingness`]
        // now, so the cushion no longer has to do that job twice.
        let depth_cushion = ParentWillingness::depth_cushion(rank);

        Some(GroupDepth {
            group,
            group_count,
            group_avg,
            group_best,
            depth_cushion,
        })
    }

    /// The young foreigner who has not settled — the single most common loan
    /// in world football, and the one every other branch here is blind to,
    /// because they all ask about a CEILING and this one does not.
    #[allow(clippy::too_many_arguments)]
    fn unsettled_abroad(
        player: &Player,
        player_info: &SquadPlayerInfo,
        date: NaiveDate,
        home: &SquadHomeContext<'_>,
        club_reputation_score: f32,
        formation_positions: &[PlayerPositionType; 11],
        depth: GroupDepth,
        loan_outs: &mut Vec<LoanOutCandidate>,
    ) -> bool {
        let group_best = depth.group_best;
        let depth_cushion = depth.depth_cushion;

        // ── The young foreigner who has not settled ─────────────
        //
        // Every branch below is about a CEILING: is he good enough to
        // grow, is he blocked by a better man, is he surplus. None of
        // them is about whether he has settled, so a Brazilian
        // twenty-one-year-old a season into the Premier League with
        // three starts and no language was invisible to all of them —
        // and the single most common loan in world football never
        // happened here.
        //
        // This one fires at EVERY reputation tier and bypasses the
        // potential / age-threshold gates, because it is not a
        // judgement about how good he is. It keeps every protection
        // that matters: the first-team asset guard (which ran above),
        // the same-window rule and the squad-minimum floor.
        //
        // What it must NOT keep is a depth-rank floor. The archetype
        // is the second-best man in his position who never starts —
        // a `rank >= 2` test excluded exactly him, at every tier, and
        // the cushion was applied in the LOOSENING direction where
        // every sibling branch tightens with it. The candidate test is
        // his START SHARE (inside `UnsettledAbroadScan`) plus a
        // ceiling read against the best man in his group, tightened by
        // how high up the chart he sits — the same shape as the
        // branches below.
        if player_info.age <= UnsettledAbroadScan::MAX_AGE
            && (player_info.current_ability as i16) < group_best as i16 - depth_cushion
        {
            let adaptation = Some(
                player.adaptation_score(
                    date,
                    home.country_code,
                    club_reputation_score,
                    Some(formation_positions),
                    &AdaptationSquadContext {
                        same_language_teammates: player
                            .squad_social_view
                            .as_ref()
                            .map(|v| v.same_language_teammates)
                            .unwrap_or(0),
                        same_nationality_teammates: player
                            .squad_social_view
                            .as_ref()
                            .map(|v| v.same_nationality_teammates)
                            .unwrap_or(0),
                        // Neutral, as the weekly pass reads it. The
                        // struct default is 0.0 — "the dressing room
                        // is at war" — a −7.5 adaptation bias that
                        // inflated the candidate list wherever it was
                        // unknown.
                        squad_chemistry: 50.0,
                        ..AdaptationSquadContext::default()
                    },
                ),
            );
            if let Some(scan) = UnsettledAbroadScan::read(player, home, adaptation, date) {
                if scan.is_candidate() {
                    loan_outs.push(LoanOutCandidate {
                        player_id: player_info.player_id,
                        reason: LoanOutReason::UnsettledAbroad,
                        status: LoanOutStatus::Identified,
                        loan_fee: 0.0,
                        preferred_destination: scan.preference,
                    });
                    return true;
                }
            }
        }

        false
    }

    /// What the club's own tier considers a reason to lend somebody out. A
    /// giant farms out prospects and the blocked; a national-tier club needs a
    /// wider believed gap because its staff read potential less well; below
    /// that, only the very young go at all.
    #[allow(clippy::too_many_arguments)]
    fn tier_verdict(
        player_info: &SquadPlayerInfo,
        squad: &[SquadPlayerInfo],
        rep_level: &ReputationLevel,
        avg_ability: u8,
        early_season: bool,
        is_january: bool,
        bands: LoanOutBands,
        depth: GroupDepth,
        loan_outs: &mut Vec<LoanOutCandidate>,
    ) -> bool {
        let age_threshold = bands.age_threshold;
        let ability_gap = bands.ability_gap;
        let min_appearances_pct = bands.min_appearances_pct;
        let group = depth.group;
        let group_avg = depth.group_avg;
        let depth_cushion = depth.depth_cushion;

        match rep_level {
            ReputationLevel::Elite | ReputationLevel::Continental => {
                // Young players who need game time. Compare to the
                // position-group average + depth cushion so the main
                // at any position isn't routed to "dev minutes".
                // Confidence gate: only act on a clear coach
                // opinion (≥ 0.4). Borderline reads stay neutral —
                // a low-judging coach shouldn't ship kids out on a
                // hunch.
                if player_info.age <= age_threshold
                    && player_info.estimated_potential > player_info.current_ability + 5
                    && player_info.potential_confidence >= 0.40
                    && (player_info.current_ability as i16)
                        < group_avg as i16 - ability_gap - depth_cushion
                {
                    loan_outs.push(LoanOutCandidate {
                        player_id: player_info.player_id,
                        reason: LoanOutReason::NeedsGameTime,
                        status: LoanOutStatus::Identified,
                        loan_fee: 0.0,
                        preferred_destination: LoanDestinationPreference::Any,
                    });
                    return true;
                }

                // Players blocked by better players. Suppressed in the
                // early-season low-evidence window: a handful of games
                // into the season, low appearances are sample noise, not
                // proof a player is blocked and needs to leave.
                if !early_season
                    && player_info.age <= 25
                    && player_info.current_ability >= avg_ability.saturating_sub(10)
                    && player_info.appearances < min_appearances_pct
                {
                    // Check if there's a clearly better player in same position
                    let better_exists = squad.iter().any(|other| {
                        other.player_id != player_info.player_id
                            && other.primary_position.position_group() == group
                            && other.current_ability > player_info.current_ability + 10
                    });

                    if better_exists {
                        loan_outs.push(LoanOutCandidate {
                            player_id: player_info.player_id,
                            reason: LoanOutReason::BlockedByBetterPlayer,
                            status: LoanOutStatus::Identified,
                            loan_fee: 0.0,
                            preferred_destination: LoanDestinationPreference::Any,
                        });
                        return true;
                    }
                }

                // Post-injury fitness
                if player_info.age <= 25
                    && player_info.is_injured
                    && player_info.recovery_days <= 14
                    && player_info.injury_days > 60
                {
                    loan_outs.push(LoanOutCandidate {
                        player_id: player_info.player_id,
                        reason: LoanOutReason::PostInjuryFitness,
                        status: LoanOutStatus::Identified,
                        loan_fee: 0.0,
                        preferred_destination: LoanDestinationPreference::Any,
                    });
                    return true;
                }

                // Lack of playing time (January window)
                if is_january
                    && player_info.age <= 26
                    && player_info.appearances < 5
                    && player_info.current_ability >= avg_ability.saturating_sub(15)
                {
                    loan_outs.push(LoanOutCandidate {
                        player_id: player_info.player_id,
                        reason: LoanOutReason::LackOfPlayingTime,
                        status: LoanOutStatus::Identified,
                        loan_fee: 0.0,
                        preferred_destination: LoanDestinationPreference::Any,
                    });
                    return true;
                }
            }
            ReputationLevel::National => {
                // Young players with high potential gap — group-relative
                // deficit + depth cushion keeps the starter unscathed.
                // National-tier staff have weaker judging eyes, so
                // demand a wider believed gap (10) and reasonable
                // confidence (≥ 0.35).
                if player_info.age <= 22
                    && player_info.estimated_potential > player_info.current_ability + 10
                    && player_info.potential_confidence >= 0.35
                    && (player_info.current_ability as i16) < group_avg as i16 - 5 - depth_cushion
                {
                    loan_outs.push(LoanOutCandidate {
                        player_id: player_info.player_id,
                        reason: LoanOutReason::NeedsGameTime,
                        status: LoanOutStatus::Identified,
                        loan_fee: 0.0,
                        preferred_destination: LoanDestinationPreference::Any,
                    });
                    return true;
                }

                // Lack of playing time (January)
                if is_january && player_info.age <= 24 && player_info.appearances < 3 {
                    loan_outs.push(LoanOutCandidate {
                        player_id: player_info.player_id,
                        reason: LoanOutReason::LackOfPlayingTime,
                        status: LoanOutStatus::Identified,
                        loan_fee: 0.0,
                        preferred_destination: LoanDestinationPreference::Any,
                    });
                    return true;
                }
            }
            _ => {
                // Regional/Local/Amateur: only loan very young players.
                // Group-relative + depth cushion, same logic as above.
                // Smaller-club staff are the weakest judges of
                // potential — require the widest believed gap (15)
                // and at least baseline confidence (≥ 0.30) before
                // acting.
                if player_info.age <= 21
                    && player_info.estimated_potential > player_info.current_ability + 15
                    && player_info.potential_confidence >= 0.30
                    && (player_info.current_ability as i16) < group_avg as i16 - 10 - depth_cushion
                {
                    loan_outs.push(LoanOutCandidate {
                        player_id: player_info.player_id,
                        reason: LoanOutReason::NeedsGameTime,
                        status: LoanOutStatus::Identified,
                        loan_fee: 0.0,
                        preferred_destination: LoanDestinationPreference::Any,
                    });
                    return true;
                }
            }
        }

        false
    }

    /// Nothing about him in particular, then — just too many bodies in his
    /// shirt, or a wage the club would rather someone else paid.
    fn surplus(
        player_info: &SquadPlayerInfo,
        philosophy: &ClubPhilosophy,
        is_january: bool,
        depth: GroupDepth,
        loan_outs: &mut Vec<LoanOutCandidate>,
    ) {
        let group = depth.group;
        let group_count = depth.group_count;
        let group_avg = depth.group_avg;
        let depth_cushion = depth.depth_cushion;

        // Surplus detection (all tiers)
        let surplus_threshold = if is_january {
            match group {
                PlayerFieldPositionGroup::Goalkeeper => 3,
                PlayerFieldPositionGroup::Defender => 5,
                PlayerFieldPositionGroup::Midfielder => 5,
                PlayerFieldPositionGroup::Forward => 3,
            }
        } else {
            match group {
                PlayerFieldPositionGroup::Goalkeeper => 3,
                PlayerFieldPositionGroup::Defender => 6,
                PlayerFieldPositionGroup::Midfielder => 6,
                PlayerFieldPositionGroup::Forward => 4,
            }
        };

        // Surplus fires on a position-group-relative deficit — a GK
        // sitting below the outfield-dominated squad mean is normal.
        // Depth cushion makes the first-choice extremely hard to flag.
        let deficit_vs_group = group_avg as i16 - player_info.current_ability as i16;
        if group_count >= surplus_threshold && deficit_vs_group >= 5 + depth_cushion {
            loan_outs.push(LoanOutCandidate {
                player_id: player_info.player_id,
                reason: LoanOutReason::Surplus,
                status: LoanOutStatus::Identified,
                loan_fee: 0.0,
                preferred_destination: LoanDestinationPreference::Any,
            });
            return;
        }

        // Financial relief (LoanFocused philosophy). The depth cushion
        // protects starters here too — you don't dump your first-choice
        // for wage relief.
        if *philosophy == ClubPhilosophy::LoanFocused
            && deficit_vs_group >= depth_cushion
            && player_info.appearances < 10
        {
            loan_outs.push(LoanOutCandidate {
                player_id: player_info.player_id,
                reason: LoanOutReason::FinancialRelief,
                status: LoanOutStatus::Identified,
                loan_fee: 0.0,
                preferred_destination: LoanDestinationPreference::Any,
            });
        }
    }
}
