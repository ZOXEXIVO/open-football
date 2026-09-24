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
use std::cmp::Reverse;

/// The loan-out scan.
pub(in crate::transfers::squad) struct LoanOutScan;

/// How aggressively the club's philosophy sends people out on loan.
#[derive(Clone, Copy)]
struct LoanOutBands {
    /// Observable points of believed upside the club wants to see before
    /// a spell away is about development rather than depth.
    believed_growth: i16,
    /// Appearances below which it reads him as not being picked.
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
        home: &SquadHomeContext<'_>,
        club_reputation_score: f32,
    ) {
        // Philosophy-based loan-out aggressiveness
        let bands = match philosophy {
            // A club that trades acts on the faintest upside; one that
            // buys its way out of every hole wants to be sure before it
            // gives a shirt away.
            ClubPhilosophy::DevelopAndSell => LoanOutBands {
                believed_growth: 5,
                min_appearances_pct: 30,
            },
            ClubPhilosophy::SignToCompete => LoanOutBands {
                believed_growth: 10,
                min_appearances_pct: 20,
            },
            ClubPhilosophy::LoanFocused => LoanOutBands {
                believed_growth: 3,
                min_appearances_pct: 40,
            },
            ClubPhilosophy::Balanced => LoanOutBands {
                believed_growth: 8,
                min_appearances_pct: 25,
            },
        };

        for player_info in squad {
            let player = match players.iter().find(|p| p.id == player_info.player_id) {
                Some(p) => p,
                None => continue,
            };

            if Self::blocked(player, player_info, club, date, current_window) {
                continue;
            }

            let Some(depth) = Self::group_depth(squad, player_info, avg_ability) else {
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

            if let Some(reason) = Self::purpose(
                player_info,
                rep_level,
                philosophy,
                bands,
                early_season,
                depth,
            ) {
                loan_outs.push(LoanOutCandidate {
                    player_id: player_info.player_id,
                    reason,
                    status: LoanOutStatus::Identified,
                    loan_fee: 0.0,
                    preferred_destination: LoanDestinationPreference::Any,
                    from_pathway: false,
                    band_target: None,
                });
            }
        }
    }

    /// The reasons a man cannot be loaned out at all.
    ///
    /// Physical only: he is already away, he has no contract to lend, he
    /// is with his country, his manager has pinned him, or the club
    /// committed to him this window. A JUDGEMENT — thirty and over,
    /// fifteen appearances, two previous spells, a first-team label, the
    /// club's own first choice — belongs in [`ParentWillingness`], since
    /// each is a reason a club is less likely to lend somebody rather
    /// than a reason it cannot.
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
                && transfer_date >= window_start
                && transfer_date <= window_end
            {
                return true;
            }
            // …and the evaluation window it bought him. A club that just
            // signed a man does not lend him out because a depth cap says
            // so.
            if player.signing_protection_active(date) {
                return true;
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
            LoanAssetGuard::willingness_for(club, player, date).score,
            player_info.asset_class.is_first_team_protected(),
            opened,
        );
        willingness < ParentWillingness::ENTERTAINS
    }

    /// His place in the pecking order, and the cushion it buys him.
    /// `None` only when his own group cannot be read.
    ///
    /// The fielding floor that used to short-circuit here is gone: it
    /// said a club at its formation's minimum lends nobody, which is the
    /// parent's own position and is priced as `depth_room` in
    /// [`ParentWillingness`]. Two readings of one fact, one of them a
    /// veto.
    fn group_depth(
        squad: &[SquadPlayerInfo],
        player_info: &SquadPlayerInfo,
        avg_ability: u8,
    ) -> Option<GroupDepth> {
        let group = player_info.primary_position.position_group();

        // Count players in same position group
        let group_count = squad
            .iter()
            .filter(|p| p.primary_position.position_group() == group)
            .count();

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
        group_ranks.sort_by_key(|g| Reverse(g.1));
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
            if let Some(scan) = UnsettledAbroadScan::read(player, home, adaptation, date)
                && scan.is_candidate()
            {
                loan_outs.push(LoanOutCandidate {
                    player_id: player_info.player_id,
                    reason: LoanOutReason::UnsettledAbroad,
                    status: LoanOutStatus::Identified,
                    loan_fee: 0.0,
                    preferred_destination: scan.preference,
                    from_pathway: false,
                    band_target: None,
                });
                return true;
            }
        }

        false
    }

    /// Why the club would send this man out, when it would send him out
    /// at all.
    ///
    /// Never WHETHER: [`Self::blocked`] has already asked the parent
    /// what it makes of lending him, and the agreement prices the
    /// destination. Each tier used to own a different ladder of its own —
    /// a giant farmed out prospects, a national club needed a wider
    /// believed gap, everybody below that lent only teenagers — so a
    /// thirty-one-year-old squad player his club was perfectly willing
    /// to lend could not be a loan candidate at any club in the world.
    ///
    /// The tier survives as the one thing it genuinely says: how clearly
    /// a staff of that standard has to SEE an upside before the club
    /// acts on it.
    fn purpose(
        player_info: &SquadPlayerInfo,
        rep_level: &ReputationLevel,
        philosophy: &ClubPhilosophy,
        bands: LoanOutBands,
        early_season: bool,
        depth: GroupDepth,
    ) -> Option<LoanOutReason> {
        // Behind the men in front of him, with the cushion his own place
        // in the queue buys him.
        let behind =
            depth.group_avg as i16 - player_info.current_ability as i16 - depth.depth_cushion;

        // Nearly fit after a long lay-off: a spell somewhere is match
        // practice, whatever else is true of him.
        if player_info.is_injured
            && player_info.recovery_days <= Self::NEARLY_FIT_DAYS
            && player_info.injury_days > Self::LONG_LAY_OFF_DAYS
        {
            return Some(LoanOutReason::PostInjuryFitness);
        }

        // A staff that can see a lot left in him, and a man not playing
        // for it here.
        let believed_growth =
            player_info.estimated_potential as i16 - player_info.current_ability as i16;
        if believed_growth >= bands.believed_growth
            && player_info.potential_confidence >= Self::confidence_bar(rep_level)
            && behind >= Self::BEHIND_THE_GROUP
        {
            return Some(LoanOutReason::NeedsGameTime);
        }

        // Somebody clearly better in his shirt. Suppressed in the
        // early-season low-evidence window: a handful of games in, low
        // appearances are sample noise rather than proof.
        let not_picked = !early_season && player_info.appearances < bands.min_appearances_pct;
        if not_picked
            && behind >= 0
            && depth.group_best as i16 > player_info.current_ability as i16 + Self::CLEARLY_BETTER
        {
            return Some(LoanOutReason::BlockedByBetterPlayer);
        }

        // Simply not being picked.
        if not_picked && behind >= 0 {
            return Some(LoanOutReason::LackOfPlayingTime);
        }

        // Too many bodies in the shirt.
        if depth.group_count >= depth.group.ideal_squad_depth() && behind >= 0 {
            return Some(LoanOutReason::Surplus);
        }

        // A wage the club would rather somebody else paid.
        if *philosophy == ClubPhilosophy::LoanFocused
            && behind >= 0
            && player_info.appearances < Self::QUIET_SEASON_APPS
        {
            return Some(LoanOutReason::FinancialRelief);
        }

        None
    }

    /// Observable points below his own position group at which the club
    /// reads him as behind the men in front.
    const BEHIND_THE_GROUP: i16 = 5;
    /// … and at which the man ahead of him is plainly a better player.
    const CLEARLY_BETTER: i16 = 10;
    /// Appearances that make a season a quiet one whatever the reason.
    const QUIET_SEASON_APPS: u16 = 10;
    /// Days from full fitness at which a spell elsewhere is match
    /// practice, and the lay-off that makes him need it.
    const NEARLY_FIT_DAYS: u16 = 14;
    const LONG_LAY_OFF_DAYS: u16 = 60;

    /// How clearly a staff of this standard has to see an upside before
    /// the club acts on it. The one thing a tier genuinely says about a
    /// loan: a smaller club's judges are weaker, so it needs more of a
    /// reading — not a younger player.
    fn confidence_bar(rep_level: &ReputationLevel) -> f32 {
        match rep_level {
            ReputationLevel::Elite | ReputationLevel::Continental => 0.40,
            ReputationLevel::National => 0.35,
            _ => 0.30,
        }
    }
}
