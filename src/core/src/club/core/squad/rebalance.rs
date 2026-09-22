//! The weekly squad rebalance: who is on the wrong team sheet.
//!
//! Four phases, in order. The progression walk decides which non-main player
//! has outgrown his squad or out-levelled the first team's last man; the
//! surplus walk decides who the first team's depth cap has squeezed out; the
//! execution applies both, honouring the squad-size guards; and the backfill
//! tops the first team back up if it is still short.

use std::collections::HashSet;

use chrono::NaiveDate;
use log::debug;

use crate::club::staff::perception::{AbilityEstimator, CoachProfile};
use crate::transfers::pipeline::{LoanOutReason, TransferTrace};
use crate::{Club, Person, PlayerFieldPositionGroup, PlayerPlan, PlayerStatusType, Team, TeamType};

use super::depth::{PromotionBar, SquadSize, YouthSquadDepth};
use super::promotion::{ProfessionalContractPromotion, PromotionEvidence};

/// Why the rebalance wants a player moved. The execution phase reads the
/// reason rather than a string: an overage player and a depth-cap
/// surplus must leave whatever it does to the squad they are on.
#[derive(Clone, Copy, PartialEq, Eq)]
enum MoveReason {
    /// Skill level ready for the first team.
    PromotionReady,
    /// Too old for the age-limited squad he is on.
    Overage,
    /// Beyond the first team's depth cap at his position.
    SurplusAtPosition,
    /// Registered above the youngest squad he is eligible for while that
    /// squad cannot name a matchday side.
    AgeGroup,
}

impl MoveReason {
    /// Overage players and depth-cap surplus leave regardless of what it does
    /// to the source squad's size — the backfill restores it from youth.
    fn ignores_squad_minimum(self) -> bool {
        matches!(
            self,
            MoveReason::Overage | MoveReason::SurplusAtPosition | MoveReason::AgeGroup
        )
    }

    fn label(self) -> &'static str {
        match self {
            MoveReason::PromotionReady => "skill level ready for first team",
            MoveReason::Overage => "overage for current team",
            MoveReason::SurplusAtPosition => "surplus at position",
            MoveReason::AgeGroup => "back to his age group",
        }
    }
}

/// One roster move the rebalance has decided on.
struct PendingMove {
    from: usize,
    to: usize,
    player_id: u32,
    reason: MoveReason,
    /// The promotion is obvious enough that the squad it empties does not get
    /// a veto — see [`SquadSize::PROMOTION_CLEAR_MARGIN`].
    clears_by_margin: bool,
    /// He was carrying a loan intent when the promotion fired, so the intent
    /// is withdrawn as part of the move.
    withdraws_loan: bool,
}

/// A player who clears the first team's bar, waiting on a place in his group.
struct PromotionCandidate {
    group: PlayerFieldPositionGroup,
    level: u8,
    promote: PendingMove,
    /// Where he goes if the first team has no place for him this week.
    fallback: Option<PendingMove>,
}

impl Club {
    /// Weekly squad rebalance across all teams.
    ///
    /// Evaluates every player's current team placement and moves them when
    /// the fit is wrong. A single pass handles three situations:
    ///
    /// 1. **Overage** — player too old for their age-limited team (U18/U19),
    ///    move to the next team in progression or main.
    /// 2. **Talent promotion** — youth/reserve player whose ability is
    ///    competitive with the main squad gets pulled up.
    /// 3. **Squad deficit** — if the main team is still below minimum after
    ///    the above, backfill with the best available youth.
    pub(in crate::club::core) fn rebalance_squads(&mut self, date: NaiveDate) {
        let Some(main_idx) = self.teams.main_index() else {
            return;
        };

        let mut moves = self.collect_progression_moves(date, main_idx);
        self.collect_surplus_demotions(date, main_idx, &mut moves);
        self.collect_age_group_moves(date, main_idx, &mut moves);
        let taken = self.execute_moves(date, main_idx, moves);
        self.backfill_main_squad(date, main_idx, &taken);
    }

    /// Phase 1: every non-main player who should step up a level, jump to the
    /// first team, or graduate out of an age-limited squad.
    fn collect_progression_moves(&self, date: NaiveDate, main_idx: usize) -> Vec<PendingMove> {
        let bar = PromotionBar::snapshot(&self.teams.teams[main_idx]);

        // Club-level promotion aggressiveness, computed once: a head coach
        // who judges potential well moves on a prospect earlier, and a
        // develop-and-sell club earlier still. The per-player senior-cameo
        // discount stacks on top inside the loop.
        let head_coach_profile =
            CoachProfile::from_staff(self.teams.teams[main_idx].staffs.head_coach());
        let club_discount = PromotionEvidence::club_discount(&head_coach_profile, &self.philosophy);

        let mut moves: Vec<PendingMove> = Vec::new();
        let mut promotions: Vec<PromotionCandidate> = Vec::new();

        for (ti, team) in self.teams.iter().enumerate() {
            if ti == main_idx || team.team_type == TeamType::Main {
                continue;
            }

            // Graduate-out age for a development squad — use
            // `development_age_cap` (U18→18 … U23→23), NOT `max_age`, which
            // bounds only U18/U19. With `max_age` a U20/U21/U23 player was
            // never flagged overage, so a keeper (or anyone) not good enough
            // for a talent promotion could sit in a youth squad into his
            // mid-20s: never playing senior football, and invisible to every
            // ambition audit (which cover Main / B / Reserve / Second only).
            let graduate_out_age = team.team_type.development_age_cap();

            for p in team.players.iter() {
                let age = p.age(date);
                let level = AbilityEstimator::observable_level(p);
                let overage = graduate_out_age.is_some_and(|limit| age > limit);

                // Never promote a player the club has decided to SELL —
                // a sale is a decision, and the first team is not where a
                // listed player waits for it. A loan intent is a different
                // thing: it is the club's answer to "he has no football
                // here", and the promotion is a better one.
                let listed = p.statuses.has(PlayerStatusType::Lst);
                let loan_intent = p.statuses.has(PlayerStatusType::Loa);

                // Senior cameos already earned via matchday call-ups are
                // direct evidence the player belongs — each one buys the
                // bar down, so the staged pipeline (call-up → cameos →
                // promotion) converges instead of waiting for the kid to
                // out-level a senior on the training pitch alone.
                let group = p.position().position_group();
                let cameo_discount = PromotionEvidence::cameo_discount(p, team.team_type);
                let floor = bar
                    .floor(group)
                    .saturating_sub(club_discount + cameo_discount);
                let clears_by_margin = level
                    >= floor.saturating_add(SquadSize::PROMOTION_CLEAR_MARGIN)
                    || bar.main_short(group);
                if TransferTrace::is(p.id) {
                    TransferTrace::line(
                        p.id,
                        "squad",
                        format!(
                            "rebalance squad={:?} level={level} floor={floor} \
                             clears_by_margin={clears_by_margin} lst={listed} loa={loan_intent} \
                             source_size={} overage={overage}",
                            team.team_type,
                            team.players.len(),
                        ),
                    );
                }
                // A group at its depth cap has no vacancy: the newcomer has
                // to be clearly better than the man he would displace, or
                // the surplus pass sends one of them straight back down and
                // the two keep swapping places week after week.
                let displaces = !bar.full(group) || clears_by_margin;
                let overage_move = if overage {
                    self.overage_move(
                        ti,
                        team.team_type,
                        p.id,
                        age,
                        listed || loan_intent,
                        main_idx,
                    )
                } else {
                    None
                };
                if level >= floor && !listed && (!loan_intent || clears_by_margin) && displaces {
                    promotions.push(PromotionCandidate {
                        group,
                        level,
                        promote: PendingMove {
                            from: ti,
                            to: main_idx,
                            player_id: p.id,
                            reason: MoveReason::PromotionReady,
                            clears_by_margin,
                            withdraws_loan: loan_intent,
                        },
                        fallback: overage_move,
                    });
                    continue;
                }
                moves.extend(overage_move);
            }
        }

        Self::admit_promotions(&bar, promotions, &mut moves);
        moves
    }

    /// Overage → the next youth tier he still fits, else a senior reserve.
    fn overage_move(
        &self,
        from: usize,
        team_type: TeamType,
        player_id: u32,
        age: u8,
        leaving: bool,
        main_idx: usize,
    ) -> Option<PendingMove> {
        let next = self
            .find_next_youth_team(team_type, age)
            .or_else(|| self.find_demotion_target(age));
        // Listed / loan-bound players are never parked on the main bench,
        // but a senior reserve is fine — they keep playing while the market
        // works. Clubs whose only non-main squads are league-less youth
        // teams leave such a player where he is; his exit is the market
        // listing itself. Anyone else too old for every youth tier lands on
        // the main bench only when the club has no reserve at all, where the
        // positional-surplus pass then loans or lists him.
        let to = if leaving {
            next?
        } else {
            next.unwrap_or(main_idx)
        };
        Some(PendingMove {
            from,
            to,
            player_id,
            reason: MoveReason::Overage,
            clears_by_margin: false,
            withdraws_loan: false,
        })
    }

    /// Promotions into a group are bounded by its places: the best
    /// candidates fill the room it has, and beyond that only a man who
    /// clearly out-levels its weakest member goes up. Uncapped, one hole
    /// let every youth above the gap floor in at once, and the surplus
    /// pass sent most of them straight back down the following week.
    fn admit_promotions(
        bar: &PromotionBar,
        mut promotions: Vec<PromotionCandidate>,
        moves: &mut Vec<PendingMove>,
    ) {
        promotions.sort_by(|a, b| b.level.cmp(&a.level));
        let mut admitted = [0usize; PlayerFieldPositionGroup::COUNT];
        for c in promotions {
            let slot = &mut admitted[c.group.index()];
            if *slot < bar.room(c.group) || bar.displaces_worst(c.group, c.level) {
                *slot += 1;
                moves.push(c.promote);
            } else {
                if TransferTrace::is(c.promote.player_id) {
                    TransferTrace::line(
                        c.promote.player_id,
                        "squad",
                        format!("promotion=deferred no_room group={:?}", c.group),
                    );
                }
                moves.extend(c.fallback);
            }
        }
    }

    /// Phase 1b: positional-surplus demotion from the first team.
    ///
    /// Enforce a depth cap per position group on the main team. Players
    /// ranked beyond the cap — by the same observable level the promotion
    /// bar reads, so the two passes cannot disagree about who the worst man
    /// in a group is — are surplus: offer them out
    /// and push them down the progression so they can get match practice in
    /// reserve/youth. Loan-ins count against the cap (they occupy a slot) but
    /// are never demoted — they belong to another club.
    fn collect_surplus_demotions(
        &mut self,
        date: NaiveDate,
        main_idx: usize,
        moves: &mut Vec<PendingMove>,
    ) {
        let mut surplus: Vec<(u32, u8)> = Vec::new();
        for group in PlayerFieldPositionGroup::ALL {
            let depth = group.main_depth_cap();
            let mut ranked: Vec<(u32, u8, u8, bool, bool)> = self.teams.teams[main_idx]
                .players
                .iter()
                .filter(|p| p.position().position_group() == group)
                .map(|p| {
                    (
                        p.id,
                        AbilityEstimator::observable_level(p),
                        p.age(date),
                        p.is_on_loan(),
                        // Manager-pinned players and signings still inside
                        // their evaluation window are never the surplus
                        // body — the club committed to them, so the excess
                        // has to be somebody else (or the squad simply runs
                        // deep until the plan is served).
                        p.is_force_match_selection || p.signing_protection_active(date),
                    )
                })
                .collect();
            ranked.sort_by(|a, b| b.1.cmp(&a.1));
            for (player_id, _, age, is_loan_in, is_protected) in ranked.into_iter().skip(depth) {
                if is_loan_in || is_protected {
                    continue;
                }
                surplus.push((player_id, age));
            }
        }

        // Main-team players the depth cap has squeezed out who should be
        // offered for loan. Collected here and pushed onto the transfer
        // plan after the roster walk, so the `&mut teams` borrow above
        // doesn't collide with `&mut transfer_plan`.
        let mut loan_out_intents: Vec<u32> = Vec::new();

        for &(player_id, age) in &surplus {
            // Where can we send them? Single-team clubs (Maltese top
            // flight, San Marino, etc.) often return None here because
            // there's no reserve/youth team to absorb the demotion.
            let demotion_target = self.find_demotion_target(age);

            if let Some(p) = self.teams.teams[main_idx].players.find_mut(player_id) {
                // No reserve/youth to demote to AND the player is too
                // old to loan? Flag for transfer instead so the surplus
                // can actually leave the club. Without this, single-
                // team clubs accumulate veterans indefinitely (see
                // Gzira: 4× 33-35 GKs sitting on the main roster
                // because they can't loan and can't demote).
                //
                // Convention: club-scoped listers set
                // `contract.is_transfer_listed` only — the country
                // listing pass owns the market listing + `Lst` status.
                // Stamping `Lst` (or `Loa`) here trips that pass's
                // already-listed guard, so the veteran showed as
                // "Listed" while never actually reaching the market.
                if demotion_target.is_none() && age >= 30 {
                    let newly_flagged = p
                        .contract
                        .as_mut()
                        .map(|c| {
                            let first = !c.is_transfer_listed;
                            c.is_transfer_listed = true;
                            first
                        })
                        .unwrap_or(false);
                    if newly_flagged {
                        TransferTrace::list(p, date, "rebalance_squads", "surplus_squad");
                        p.decision_history.add(
                            date,
                            "dec_transfer_listed".to_string(),
                            "dec_reason_surplus_squad".to_string(),
                            "dec_decided_board".to_string(),
                        );
                    }
                } else if !p.statuses.has(PlayerStatusType::Loa) {
                    // Same convention as the transfer branch above: record
                    // the club's INTENT and let the country listing pass
                    // own the market row and the badge. Stamping `Loa`
                    // here instead used to trip that pass's already-listed
                    // guard, so the badge showed on the player while no
                    // loan listing ever existed — a surplus main-team
                    // player advertised to nobody.
                    loan_out_intents.push(player_id);
                }
            }
            if let Some(dest) = demotion_target {
                moves.push(PendingMove {
                    from: main_idx,
                    to: dest,
                    player_id,
                    reason: MoveReason::SurplusAtPosition,
                    clears_by_margin: false,
                    withdraws_loan: false,
                });
            }
        }

        // Register the loan intents the surplus walk raised, through the
        // one club-side producer — the purpose has to survive to the
        // borrower, and the pathway has to know he is on his way out.
        for player_id in loan_out_intents {
            self.on_pathway_loan_staged(player_id, LoanOutReason::Surplus, date);
        }
    }

    /// Phase 1c: a youth squad that plays in a league but cannot name a
    /// matchday side takes back the boys of its age the club has registered
    /// in older squads below the first team.
    ///
    /// A player's squad is the youngest one he is eligible for; an older
    /// reserve is where he goes when he outgrows it or the first team sends
    /// him down, not where he waits while his own age group forfeits. The
    /// first team is never raided — a boy there earned it. League-less
    /// squads give first (nobody plays there), then the least-used, weakest
    /// boys, so a reserve keeps the ones it actually plays.
    fn collect_age_group_moves(
        &self,
        date: NaiveDate,
        main_idx: usize,
        moves: &mut Vec<PendingMove>,
    ) {
        let teams = &self.teams.teams;
        let mut size: Vec<usize> = teams.iter().map(|t| t.players.len()).collect();
        for m in moves.iter() {
            size[m.from] = size[m.from].saturating_sub(1);
            size[m.to] += 1;
        }
        let mut moving: HashSet<u32> = moves.iter().map(|m| m.player_id).collect();

        for (home_idx, home) in teams.iter().enumerate() {
            if !home.team_type.is_youth() || home.league_id.is_none() {
                continue;
            }
            let need = SquadSize::YOUTH_MATCHDAY.saturating_sub(size[home_idx]);
            if need == 0 {
                continue;
            }
            let mut lines = [0usize; PlayerFieldPositionGroup::COUNT];
            for p in home.players.iter() {
                lines[p.position().position_group().index()] += 1;
            }

            // (source plays in a league, appearances, level, source, id, group)
            let mut candidates: Vec<(bool, u16, u8, usize, u32, PlayerFieldPositionGroup)> =
                Vec::new();
            for (ti, team) in teams.iter().enumerate() {
                if ti == home_idx || ti == main_idx || team.team_type == TeamType::Main {
                    continue;
                }
                for p in team.players.iter() {
                    if moving.contains(&p.id)
                        || p.is_on_loan()
                        || p.is_force_match_selection
                        || self.find_youth_team_for_age(p.age(date)) != Some(home_idx)
                    {
                        continue;
                    }
                    candidates.push((
                        team.league_id.is_some(),
                        p.statistics.played + p.statistics.played_subs,
                        AbilityEstimator::observable_level(p),
                        ti,
                        p.id,
                        p.position().position_group(),
                    ));
                }
            }
            candidates.sort_by_key(|c| (c.0, c.1, c.2, c.3, c.4));

            let mut taken = 0;
            for (_, _, _, from, player_id, group) in candidates {
                if taken == need {
                    break;
                }
                if lines[group.index()] >= YouthSquadDepth::keep_for(group)
                    || size[from] <= Self::age_group_source_floor(&teams[from])
                {
                    continue;
                }
                moves.push(PendingMove {
                    from,
                    to: home_idx,
                    player_id,
                    reason: MoveReason::AgeGroup,
                    clears_by_margin: false,
                    withdraws_loan: false,
                });
                moving.insert(player_id);
                lines[group.index()] += 1;
                size[from] -= 1;
                size[home_idx] += 1;
                taken += 1;
            }
        }
    }

    /// Size a squad keeps while handing boys back to their age group: its
    /// own matchday needs when it plays, nothing when it does not.
    fn age_group_source_floor(team: &Team) -> usize {
        if team.league_id.is_none() {
            0
        } else if team.team_type.is_own_team() {
            SquadSize::MIN_MAIN
        } else {
            SquadSize::YOUTH_MATCHDAY
        }
    }

    /// Phase 2: apply the moves, respecting the squad-size guards. Returns how
    /// many players each team gave up, which the backfill needs.
    fn execute_moves(
        &mut self,
        date: NaiveDate,
        main_idx: usize,
        mut moves: Vec<PendingMove>,
    ) -> Vec<usize> {
        let club_id = self.id;
        // Talent promotions (to main) first, then overage moves.
        moves.sort_by(|a, b| {
            let a_main = (a.to == main_idx) as u8;
            let b_main = (b.to == main_idx) as u8;
            b_main.cmp(&a_main)
        });

        // Track how many players we've taken from each source team
        // so we don't drain any team below minimum.
        let mut taken: Vec<usize> = vec![0; self.teams.teams.len()];

        for m in &moves {
            let source_size = self.teams.teams[m.from].players.players.len();
            let already_taken = taken[m.from];

            let min_for_source = if m.from == main_idx {
                SquadSize::MIN_MAIN
            } else {
                SquadSize::MIN_YOUTH
            };
            if !m.reason.ignores_squad_minimum()
                && !m.clears_by_margin
                && source_size.saturating_sub(already_taken) <= min_for_source
            {
                if TransferTrace::is(m.player_id) {
                    TransferTrace::line(
                        m.player_id,
                        "squad",
                        format!(
                            "move={} blocked_by=min_squad source_size={source_size} \
                             taken={already_taken} min={min_for_source}",
                            m.reason.label(),
                        ),
                    );
                }
                continue;
            }

            let from_info = self.teams.teams[m.from].history_info();
            let to_info = self.teams.teams[m.to].history_info();
            let from_senior = self.teams.teams[m.from].team_type.is_own_team();
            let to_senior = self.teams.teams[m.to].team_type.is_own_team();

            if let Some(mut player) = self.teams.teams[m.from].players.take_player(&m.player_id) {
                // A club does not loan out the boy who has just become
                // first-team ready. The badge goes, the candidate row goes,
                // and the country pass is told to pull his live loan
                // listing — otherwise the row keeps advertising a player
                // the club has just promoted.
                if m.withdraws_loan {
                    player.statuses.remove(PlayerStatusType::Loa);
                    // The stage is the same decision as the badge and the
                    // row: a boy the club has just promoted is not still
                    // staged for a loan.
                    let promoted = player.pathway_stage().promoted();
                    player.on_pathway_advanced(
                        club_id,
                        promoted,
                        PlayerPlan::SHORT_REVIEW_DAYS,
                        date,
                    );
                    player.decision_history.add(
                        date,
                        "dec_loan_withdrawn".to_string(),
                        "dec_reason_promoted_instead".to_string(),
                        "dec_decided_board".to_string(),
                    );
                    self.transfer_plan
                        .loan_out_candidates
                        .retain(|c| c.player_id != m.player_id);
                    if !self.transfer_plan.loan_withdrawals.contains(&m.player_id) {
                        self.transfer_plan.loan_withdrawals.push(m.player_id);
                    }
                    if TransferTrace::is(m.player_id) {
                        TransferTrace::line(
                            m.player_id,
                            "squad",
                            "promotion=granted loan_intent=withdrawn \
                             reason=promoted_instead",
                        );
                    }
                }
                if TransferTrace::is(m.player_id) {
                    TransferTrace::line(
                        m.player_id,
                        "squad",
                        format!(
                            "move={} from={} to={} clears_by_margin={}",
                            m.reason.label(),
                            self.teams.teams[m.from].name,
                            self.teams.teams[m.to].name,
                            m.clears_by_margin,
                        ),
                    );
                }
                // Upgrade youth contract to full when promoting to main
                if m.to == main_idx {
                    ProfessionalContractPromotion::upgrade(
                        &mut player,
                        date,
                        self.teams.teams[main_idx].reputation.world,
                    );
                    // Career-defining promotion to senior football. Long
                    // cooldown (effectively one-shot per spell) keeps the
                    // event scarce — a player who yo-yos between reserve
                    // and main shouldn't get a fresh "breakthrough" each
                    // bounce.
                    player.on_youth_breakthrough(date);
                }

                // Close the previous spell and open one on the destination
                // team so future official matches accumulate against the
                // team the player actually plays for. Without this, B-team
                // appearances kept being recorded under the Main row.
                player.on_intra_club_move(&from_info, &to_info, from_senior, to_senior, date);

                debug!(
                    "squad rebalance: {} (CA={}, age={}) {} → {} ({})",
                    player.full_name,
                    player.player_attributes.current_ability,
                    player.age(date),
                    from_info.name,
                    to_info.name,
                    m.reason.label(),
                );
                self.teams.teams[m.to].players.add(player);
                taken[m.from] += 1;
            }
        }

        taken
    }

    /// Phase 3: top the first team back up if it is still short of a working
    /// squad, taking the best available youth.
    fn backfill_main_squad(&mut self, date: NaiveDate, main_idx: usize, taken: &[usize]) {
        let main_count = self.teams.teams[main_idx].players.players.len();
        if main_count >= SquadSize::MIN_MAIN {
            return;
        }

        let deficit = SquadSize::MIN_MAIN - main_count;
        let mut candidates: Vec<(usize, u32, u8)> = Vec::new();

        for (ti, team) in self.teams.iter().enumerate() {
            if ti == main_idx || team.team_type == TeamType::Main {
                continue;
            }
            let available = team.players.len().saturating_sub(taken[ti]);
            if available <= SquadSize::MIN_YOUTH && team.team_type.max_age().is_some() {
                continue;
            }
            for p in team.players.iter() {
                if p.statuses.has(PlayerStatusType::Lst) || p.statuses.has(PlayerStatusType::Loa) {
                    continue;
                }
                if p.is_force_match_selection {
                    continue;
                }
                candidates.push((ti, p.id, AbilityEstimator::observable_level(p)));
            }
        }

        candidates.sort_by(|a, b| b.2.cmp(&a.2));
        candidates.truncate(deficit);

        for (team_idx, player_id, _) in candidates {
            let from_info = self.teams.teams[team_idx].history_info();
            let to_info = self.teams.teams[main_idx].history_info();
            let from_senior = self.teams.teams[team_idx].team_type.is_own_team();
            let to_senior = self.teams.teams[main_idx].team_type.is_own_team();
            if let Some(mut player) = self.teams.teams[team_idx].players.take_player(&player_id) {
                ProfessionalContractPromotion::upgrade(
                    &mut player,
                    date,
                    self.teams.teams[main_idx].reputation.world,
                );
                player.on_youth_breakthrough(date);
                player.on_intra_club_move(&from_info, &to_info, from_senior, to_senior, date);
                debug!(
                    "backfill to main: {} (CA={}, age={}) from {}",
                    player.full_name,
                    player.player_attributes.current_ability,
                    player.age(date),
                    from_info.name
                );
                self.teams.teams[main_idx].players.add(player);
            }
        }
    }

    /// Find the next youth team in progression (U18→U19→U20→U21→U23)
    /// that exists in this club and can accept a player of the given age.
    fn find_next_youth_team(&self, current_type: TeamType, player_age: u8) -> Option<usize> {
        let progression = TeamType::YOUTH_PROGRESSION;

        let current_pos = progression.iter().position(|t| *t == current_type)?;

        for next_type in &progression[current_pos + 1..] {
            // Skip a tier the player has already outgrown (graduate-out age),
            // so an overage player lands on the youngest tier that still fits
            // — or, if too old for all of them, `None` falls through to a
            // senior squad at the call site.
            let age_ok = match next_type.development_age_cap() {
                Some(cap) => player_age <= cap,
                None => true,
            };
            if age_ok {
                if let Some(idx) = self.teams.index_of_type(*next_type) {
                    return Some(idx);
                }
            }
        }

        None
    }

    /// Best non-main destination for a demoted main-team player.
    /// Adult teams (Reserve, B) come first so surplus seniors keep
    /// playing competitive matches; absent those, fall back to the
    /// youth team that fits the player's age.
    fn find_demotion_target(&self, age: u8) -> Option<usize> {
        for t in [TeamType::Reserve, TeamType::B, TeamType::Second] {
            if let Some(idx) = self.teams.index_of_type(t) {
                return Some(idx);
            }
        }
        self.find_youth_team_for_age(age)
    }

    /// Find the best-fitting youth team for a player of the given age.
    /// Returns the youngest team the player is eligible for.
    pub(in crate::club::core) fn find_youth_team_for_age(&self, player_age: u8) -> Option<usize> {
        let targets: [(TeamType, u8); 5] = [
            (TeamType::U18, 18),
            (TeamType::U19, 19),
            (TeamType::U20, 20),
            (TeamType::U21, 21),
            (TeamType::U23, 23),
        ];

        for (team_type, max_age) in targets {
            if player_age <= max_age {
                if let Some(idx) = self.teams.index_of_type(team_type) {
                    return Some(idx);
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod promotion_evidence_tests {
    use super::*;
    use crate::academy::ClubAcademy;
    use crate::club::player::core::builder::PlayerBuilder;
    use crate::shared::Location;
    use crate::shared::fullname::FullName;
    use crate::{
        ClubColors, ClubFacilities, ClubFinances, ClubStatus, PersonAttributes, Player,
        PlayerAttributes, PlayerClubContract, PlayerCollection, PlayerPosition, PlayerPositionType,
        PlayerPositions, PlayerSkills, StaffCollection, TeamBuilder, TeamCollection,
        TeamReputation, TrainingSchedule,
    };
    use chrono::{Datelike, NaiveTime};

    struct Fx;

    impl Fx {
        fn date() -> NaiveDate {
            NaiveDate::from_ymd_opt(2026, 10, 1).unwrap()
        }

        fn schedule() -> TrainingSchedule {
            TrainingSchedule::new(
                NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
                NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
            )
        }

        fn player(id: u32, position: PlayerPositionType, ability: u8, age: u8) -> Player {
            let date = Self::date();
            let mut attrs = PlayerAttributes::default();
            attrs.current_ability = ability;
            attrs.potential_ability = ability;
            attrs.condition = 10_000;
            PlayerBuilder::new()
                .id(id)
                .full_name(FullName::new("T".to_string(), format!("P{id}")))
                .birth_date(NaiveDate::from_ymd_opt(date.year() - age as i32, 1, 1).unwrap())
                .country_id(1)
                .attributes(PersonAttributes::default())
                .skills(PlayerSkills::flat_for_ability(ability))
                .positions(PlayerPositions {
                    positions: vec![PlayerPosition {
                        position,
                        level: 18,
                    }],
                })
                .player_attributes(attrs)
                .contract(Some(PlayerClubContract::new(
                    20_000,
                    NaiveDate::from_ymd_opt(2029, 6, 30).unwrap(),
                )))
                .build()
                .unwrap()
        }

        /// 22 seniors at observable ~120, groups inside both the minimum
        /// depth and the surplus caps, so neither the depth-gap floor nor
        /// the demotion pass interferes with the promotion bar under test.
        fn main_roster() -> Vec<Player> {
            let mut players = Vec::new();
            let mut id = 100u32;
            let push = |pos: PlayerPositionType, n: usize, id: &mut u32, out: &mut Vec<Player>| {
                for _ in 0..n {
                    out.push(Self::player(*id, pos, 120, 27));
                    *id += 1;
                }
            };
            push(PlayerPositionType::Goalkeeper, 2, &mut id, &mut players);
            push(PlayerPositionType::DefenderCenter, 8, &mut id, &mut players);
            push(
                PlayerPositionType::MidfielderCenter,
                6,
                &mut id,
                &mut players,
            );
            push(PlayerPositionType::Striker, 6, &mut id, &mut players);
            players
        }

        /// U19 squad of twelve: the candidate plus eleven fillers far
        /// below any promotion bar, keeping the squad-minimum guard open.
        fn u19_roster(candidate: Player) -> Vec<Player> {
            let mut players = vec![candidate];
            let mut id = 300u32;
            for _ in 0..4 {
                players.push(Self::player(id, PlayerPositionType::DefenderCenter, 50, 17));
                id += 1;
            }
            for _ in 0..4 {
                players.push(Self::player(
                    id,
                    PlayerPositionType::MidfielderCenter,
                    50,
                    17,
                ));
                id += 1;
            }
            for _ in 0..3 {
                players.push(Self::player(id, PlayerPositionType::Striker, 50, 17));
                id += 1;
            }
            players
        }

        fn club(candidate: Player) -> Club {
            let main = TeamBuilder::new()
                .id(10)
                .league_id(Some(1))
                .club_id(100)
                .name("Main".to_string())
                .slug("main".to_string())
                .team_type(TeamType::Main)
                .players(PlayerCollection::new(Self::main_roster()))
                .staffs(StaffCollection::new(Vec::new()))
                .reputation(TeamReputation::new(500, 500, 500))
                .training_schedule(Self::schedule())
                .build()
                .unwrap();
            let u19 = TeamBuilder::new()
                .id(19)
                .league_id(None)
                .club_id(100)
                .name("U19".to_string())
                .slug("u19".to_string())
                .team_type(TeamType::U19)
                .players(PlayerCollection::new(Self::u19_roster(candidate)))
                .staffs(StaffCollection::new(Vec::new()))
                .reputation(TeamReputation::new(300, 300, 300))
                .training_schedule(Self::schedule())
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
                TeamCollection::new(vec![main, u19]),
                ClubFacilities::default(),
            )
        }

        fn on_team(club: &Club, team_idx: usize, id: u32) -> bool {
            club.teams.teams[team_idx]
                .players
                .players
                .iter()
                .any(|p| p.id == id)
        }
    }

    /// The staged pipeline converging: a near-senior U19 midfielder who
    /// has already collected senior cameos (official appearances while
    /// youth-rostered) clears the discounted promotion bar and moves up.
    #[test]
    fn senior_cameos_accelerate_promotion() {
        let mut candidate = Fx::player(1, PlayerPositionType::MidfielderCenter, 112, 17);
        candidate.statistics.played = 5;
        for _ in 0..5 {
            candidate.statistics.record_match_rating(7.0, 90, true);
        }
        let mut club = Fx::club(candidate);

        club.rebalance_squads(Fx::date());

        assert!(
            Fx::on_team(&club, 0, 1),
            "senior-cameo evidence promotes the near-level prospect to the first team"
        );
    }

    /// The same prospect without a single senior appearance stays in the
    /// academy — the bar only comes down on evidence.
    #[test]
    fn no_cameo_evidence_keeps_prospect_in_the_academy() {
        let candidate = Fx::player(1, PlayerPositionType::MidfielderCenter, 112, 17);
        let mut club = Fx::club(candidate);

        club.rebalance_squads(Fx::date());

        assert!(
            !Fx::on_team(&club, 0, 1),
            "without cameo evidence the promotion bar holds"
        );
        assert!(Fx::on_team(&club, 1, 1), "the prospect stays with the U19s");
    }
}

#[cfg(test)]
mod rebalance_patience_tests {
    //! The weekly rebalance's positional-surplus demotion (Phase 1b) must
    //! honour the signing plan: a player the club bought weeks ago is not
    //! the surplus body, however he ranks against the depth cap — that
    //! bought-then-loan-listed churn is exactly what the patience gate
    //! exists to stop.

    use super::*;
    use crate::PlayerFieldPositionGroup;
    use crate::academy::ClubAcademy;
    use crate::club::board::mandate::{MandateAuthor, MandatePurpose, SigningMandate};
    use crate::club::player::core::builder::PlayerBuilder;
    use crate::shared::Location;
    use crate::shared::fullname::FullName;
    use crate::{
        ClubColors, ClubFacilities, ClubFinances, ClubStatus, PersonAttributes, Player,
        PlayerAttributes, PlayerClubContract, PlayerCollection, PlayerPlan, PlayerPosition,
        PlayerPositionType, PlayerPositions, PlayerSkills, StaffCollection, TeamBuilder,
        TeamCollection, TeamReputation, TrainingSchedule,
    };
    use chrono::{Datelike, Duration, NaiveTime};

    struct Fx;

    impl Fx {
        fn date() -> NaiveDate {
            NaiveDate::from_ymd_opt(2026, 10, 1).unwrap()
        }

        fn schedule() -> TrainingSchedule {
            TrainingSchedule::new(
                NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
                NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
            )
        }

        fn player(id: u32, position: PlayerPositionType, ability: u8, age: u8) -> Player {
            let date = Self::date();
            let mut attrs = PlayerAttributes::default();
            attrs.current_ability = ability;
            attrs.potential_ability = ability;
            attrs.condition = 10_000;
            PlayerBuilder::new()
                .id(id)
                .full_name(FullName::new("T".to_string(), format!("P{id}")))
                .birth_date(NaiveDate::from_ymd_opt(date.year() - age as i32, 1, 1).unwrap())
                .country_id(1)
                .attributes(PersonAttributes::default())
                .skills(PlayerSkills::flat_for_ability(ability))
                .positions(PlayerPositions {
                    positions: vec![PlayerPosition {
                        position,
                        level: 18,
                    }],
                })
                .player_attributes(attrs)
                .contract(Some(PlayerClubContract::new(
                    20_000,
                    NaiveDate::from_ymd_opt(2029, 6, 30).unwrap(),
                )))
                .build()
                .unwrap()
        }

        /// Main roster with TEN central midfielders — one beyond the
        /// group's depth cap of 9 — where the tenth (id 1, lowest CA) is
        /// the natural demotion candidate. Other groups stay inside caps.
        fn overloaded_main(tenth_midfielder: Player) -> Vec<Player> {
            let mut players = vec![tenth_midfielder];
            let mut id = 100u32;
            let push =
                |pos: PlayerPositionType, n: usize, ca: u8, id: &mut u32, out: &mut Vec<Player>| {
                    for _ in 0..n {
                        out.push(Self::player(*id, pos, ca, 27));
                        *id += 1;
                    }
                };
            push(
                PlayerPositionType::Goalkeeper,
                2,
                120,
                &mut id,
                &mut players,
            );
            push(
                PlayerPositionType::DefenderCenter,
                8,
                120,
                &mut id,
                &mut players,
            );
            push(
                PlayerPositionType::MidfielderCenter,
                9,
                120,
                &mut id,
                &mut players,
            );
            push(PlayerPositionType::Striker, 5, 120, &mut id, &mut players);
            players
        }

        fn club(tenth_midfielder: Player) -> Club {
            let main = TeamBuilder::new()
                .id(10)
                .league_id(Some(1))
                .club_id(100)
                .name("Main".to_string())
                .slug("main".to_string())
                .team_type(TeamType::Main)
                .players(PlayerCollection::new(Self::overloaded_main(
                    tenth_midfielder,
                )))
                .staffs(StaffCollection::new(Vec::new()))
                .reputation(TeamReputation::new(500, 500, 500))
                .training_schedule(Self::schedule())
                .build()
                .unwrap();
            let reserve_players: Vec<Player> = (300..312)
                .map(|id| Self::player(id, PlayerPositionType::MidfielderCenter, 50, 22))
                .collect();
            let reserve = TeamBuilder::new()
                .id(20)
                .league_id(None)
                .club_id(100)
                .name("Reserve".to_string())
                .slug("reserve".to_string())
                .team_type(TeamType::Reserve)
                .players(PlayerCollection::new(reserve_players))
                .staffs(StaffCollection::new(Vec::new()))
                .reputation(TeamReputation::new(300, 300, 300))
                .training_schedule(Self::schedule())
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
                TeamCollection::new(vec![main, reserve]),
                ClubFacilities::default(),
            )
        }

        fn on_main(club: &Club, id: u32) -> Option<&Player> {
            club.teams.teams[0]
                .players
                .players
                .iter()
                .find(|p| p.id == id)
        }
    }

    #[test]
    fn overdepth_veteran_without_plan_is_demoted_and_loan_listed() {
        // Control: the tenth midfielder with no signing plan ranks outside
        // the depth cap and takes the normal surplus route.
        let mut club = Fx::club(Fx::player(1, PlayerPositionType::MidfielderCenter, 100, 27));

        club.rebalance_squads(Fx::date());

        assert!(
            Fx::on_main(&club, 1).is_none(),
            "the unprotected over-depth midfielder is demoted off the main squad"
        );
    }

    #[test]
    fn overdepth_new_signing_with_active_plan_stays_on_main() {
        // The same tenth midfielder, but signed three weeks ago: his plan
        // is active, so he is neither demoted nor loan-listed — the squad
        // simply runs deep until the club has actually evaluated him.
        let mut signing = Fx::player(1, PlayerPositionType::MidfielderCenter, 100, 27);
        signing.plan = Some(PlayerPlan::from_mandate(
            SigningMandate::new(
                MandatePurpose::Starter,
                PlayerFieldPositionGroup::Midfielder,
                27,
                Fx::date() - Duration::days(21),
                MandateAuthor::Manager,
            )
            .with_money(2_000_000.0, 0.0),
            Fx::date() - Duration::days(21),
        ));
        let mut club = Fx::club(signing);

        club.rebalance_squads(Fx::date());

        let kept = Fx::on_main(&club, 1)
            .expect("a signing inside his evaluation window stays on the main squad");
        assert!(
            !kept.statuses.has(PlayerStatusType::Loa),
            "a signing inside his evaluation window must not be loan-listed"
        );
    }
}

#[cfg(test)]
mod overage_graduation_tests {
    use super::*;
    use crate::academy::ClubAcademy;
    use crate::club::player::core::builder::PlayerBuilder;
    use crate::shared::Location;
    use crate::shared::fullname::FullName;
    use crate::{
        ClubColors, ClubFacilities, ClubFinances, ClubStatus, PersonAttributes, Player,
        PlayerAttributes, PlayerClubContract, PlayerCollection, PlayerPosition, PlayerPositionType,
        PlayerPositions, PlayerSkills, StaffCollection, TeamBuilder, TeamCollection,
        TeamReputation, TrainingSchedule,
    };
    use chrono::{Datelike, NaiveTime};

    struct Fx;

    impl Fx {
        fn date() -> NaiveDate {
            NaiveDate::from_ymd_opt(2026, 10, 1).unwrap()
        }

        fn schedule() -> TrainingSchedule {
            TrainingSchedule::new(
                NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
                NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
            )
        }

        fn player(id: u32, position: PlayerPositionType, ability: u8, age: u8) -> Player {
            let date = Self::date();
            let mut attrs = PlayerAttributes::default();
            attrs.current_ability = ability;
            attrs.potential_ability = ability;
            attrs.condition = 10_000;
            PlayerBuilder::new()
                .id(id)
                .full_name(FullName::new("T".to_string(), format!("P{id}")))
                .birth_date(NaiveDate::from_ymd_opt(date.year() - age as i32, 1, 1).unwrap())
                .country_id(1)
                .attributes(PersonAttributes::default())
                .skills(PlayerSkills::flat_for_ability(ability))
                .positions(PlayerPositions {
                    positions: vec![PlayerPosition {
                        position,
                        level: 18,
                    }],
                })
                .player_attributes(attrs)
                .contract(Some(PlayerClubContract::new(
                    20_000,
                    NaiveDate::from_ymd_opt(2029, 6, 30).unwrap(),
                )))
                .build()
                .unwrap()
        }

        /// 22 seniors at observable ~120 — the main GK slots sit far above
        /// any promotion bar the candidate could clear, so only the overage
        /// path can move him.
        fn main_roster() -> Vec<Player> {
            let mut players = Vec::new();
            let mut id = 100u32;
            let push = |pos: PlayerPositionType, n: usize, id: &mut u32, out: &mut Vec<Player>| {
                for _ in 0..n {
                    out.push(Self::player(*id, pos, 120, 27));
                    *id += 1;
                }
            };
            push(PlayerPositionType::Goalkeeper, 2, &mut id, &mut players);
            push(PlayerPositionType::DefenderCenter, 8, &mut id, &mut players);
            push(
                PlayerPositionType::MidfielderCenter,
                6,
                &mut id,
                &mut players,
            );
            push(PlayerPositionType::Striker, 6, &mut id, &mut players);
            players
        }

        /// U20 squad: the overage keeper plus eleven age-appropriate fillers.
        fn u20_roster(candidate: Player) -> Vec<Player> {
            let mut players = vec![candidate];
            let mut id = 300u32;
            for _ in 0..4 {
                players.push(Self::player(id, PlayerPositionType::DefenderCenter, 50, 18));
                id += 1;
            }
            for _ in 0..4 {
                players.push(Self::player(
                    id,
                    PlayerPositionType::MidfielderCenter,
                    50,
                    18,
                ));
                id += 1;
            }
            for _ in 0..3 {
                players.push(Self::player(id, PlayerPositionType::Striker, 50, 18));
                id += 1;
            }
            players
        }

        /// Eleven senior reserves so the Second team is a valid demotion
        /// target (its own size never blocks an incoming move).
        fn second_roster() -> Vec<Player> {
            let mut players = Vec::new();
            let mut id = 500u32;
            for _ in 0..11 {
                players.push(Self::player(id, PlayerPositionType::DefenderCenter, 70, 24));
                id += 1;
            }
            players
        }

        fn team(id: u32, slug: &str, tt: TeamType, players: Vec<Player>) -> crate::Team {
            TeamBuilder::new()
                .id(id)
                .league_id(if tt == TeamType::U20 { None } else { Some(1) })
                .club_id(100)
                .name(slug.to_string())
                .slug(slug.to_string())
                .team_type(tt)
                .players(PlayerCollection::new(players))
                .staffs(StaffCollection::new(Vec::new()))
                .reputation(TeamReputation::new(400, 400, 400))
                .training_schedule(Self::schedule())
                .build()
                .unwrap()
        }

        fn club(candidate: Player, with_second: bool) -> Club {
            let mut teams = vec![
                Self::team(10, "main", TeamType::Main, Self::main_roster()),
                Self::team(20, "u20", TeamType::U20, Self::u20_roster(candidate)),
            ];
            if with_second {
                teams.push(Self::team(
                    80,
                    "second",
                    TeamType::Second,
                    Self::second_roster(),
                ));
            }
            Club::new(
                100,
                "Club".to_string(),
                Location::new(1),
                ClubFinances::new(10_000_000, Vec::new()),
                ClubAcademy::new(3),
                ClubStatus::Professional,
                ClubColors::default(),
                TeamCollection::new(teams),
                ClubFacilities::default(),
            )
        }

        fn team_idx_of(club: &Club, tt: TeamType) -> Option<usize> {
            club.teams.teams.iter().position(|t| t.team_type == tt)
        }

        fn on_team(club: &Club, tt: TeamType, id: u32) -> bool {
            Self::team_idx_of(club, tt)
                .map(|idx| {
                    club.teams.teams[idx]
                        .players
                        .players
                        .iter()
                        .any(|p| p.id == id)
                })
                .unwrap_or(false)
        }
    }

    /// The reported bug: a modest keeper too old for a talent promotion but
    /// past the U20 age cap must not rot in the youth squad forever. He
    /// graduates out — to the senior reserve when one exists.
    #[test]
    fn overage_keeper_graduates_out_of_u20_to_senior_reserve() {
        let candidate = Fx::player(1, PlayerPositionType::Goalkeeper, 50, 25);
        let mut club = Fx::club(candidate, /* with_second */ true);

        club.rebalance_squads(Fx::date());

        assert!(
            !Fx::on_team(&club, TeamType::U20, 1),
            "an overage keeper must not stay parked in the U20 squad"
        );
        assert!(
            Fx::on_team(&club, TeamType::Second, 1),
            "he graduates to the senior reserve where he plays competitive football"
        );
    }

    /// With no senior reserve, the overage player still leaves the youth
    /// squad — onto the main bench, where the surplus/loan machinery owns him.
    #[test]
    fn overage_keeper_leaves_u20_even_without_a_reserve() {
        let candidate = Fx::player(1, PlayerPositionType::Goalkeeper, 50, 25);
        let mut club = Fx::club(candidate, /* with_second */ false);

        club.rebalance_squads(Fx::date());

        assert!(
            !Fx::on_team(&club, TeamType::U20, 1),
            "with no reserve he still must not be stuck in the U20 squad"
        );
        assert!(
            Fx::on_team(&club, TeamType::Main, 1),
            "absent a reserve, he lands on the main roster for the surplus pass to route"
        );
    }
}

#[cfg(test)]
mod promotion_guard_tests {
    //! WI-2: the two guards that kept four first-team players inside an
    //! eight-man U20 for months on end. `SquadSize::MIN_YOUTH` refused any
    //! non-overage promotion out of a squad already at eleven — which is
    //! exactly the state such a squad sits in — and a `Loa` badge closed
    //! the door for good the moment any loan intent landed on the player.

    use super::*;
    use crate::academy::ClubAcademy;
    use crate::club::player::core::builder::PlayerBuilder;
    use crate::shared::Location;
    use crate::shared::fullname::FullName;
    use crate::transfers::pipeline::{
        LoanDestinationPreference, LoanOutCandidate, LoanOutReason, LoanOutStatus,
    };
    use crate::{
        ClubColors, ClubFacilities, ClubFinances, ClubStatus, PersonAttributes, Player,
        PlayerAttributes, PlayerClubContract, PlayerCollection, PlayerPosition, PlayerPositionType,
        PlayerPositions, PlayerSkills, StaffCollection, TeamBuilder, TeamCollection,
        TeamReputation, TrainingSchedule,
    };
    use crate::{PathwayStage, PlayerFieldPositionGroup, PlayerPlan, PlayerPlanRole};
    use chrono::{Datelike, NaiveTime};

    struct Fx;

    impl Fx {
        /// Well clear of the main roster's promotion bar (~121) by more
        /// than [`SquadSize::PROMOTION_CLEAR_MARGIN`].
        const READY: u8 = 160;

        fn date() -> NaiveDate {
            NaiveDate::from_ymd_opt(2026, 10, 1).unwrap()
        }

        fn schedule() -> TrainingSchedule {
            TrainingSchedule::new(
                NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
                NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
            )
        }

        fn player(id: u32, position: PlayerPositionType, ability: u8, age: u8) -> Player {
            let date = Self::date();
            let mut attrs = PlayerAttributes::default();
            attrs.current_ability = ability;
            attrs.potential_ability = ability;
            attrs.condition = 10_000;
            PlayerBuilder::new()
                .id(id)
                .full_name(FullName::new("T".to_string(), format!("P{id}")))
                .birth_date(NaiveDate::from_ymd_opt(date.year() - age as i32, 1, 1).unwrap())
                .country_id(1)
                .attributes(PersonAttributes::default())
                .skills(PlayerSkills::flat_for_ability(ability))
                .positions(PlayerPositions {
                    positions: vec![PlayerPosition {
                        position,
                        level: 18,
                    }],
                })
                .player_attributes(attrs)
                .contract(Some(PlayerClubContract::new(
                    20_000,
                    NaiveDate::from_ymd_opt(2029, 6, 30).unwrap(),
                )))
                .build()
                .unwrap()
        }

        /// 22 seniors at ~120 across the four groups, all inside the
        /// minimum depths, so the promotion bar is "better than the worst
        /// man here" rather than a depth-gap floor.
        fn main_roster() -> Vec<Player> {
            let mut players = Vec::new();
            let mut id = 100u32;
            let mut push = |pos: PlayerPositionType, n: usize| {
                for _ in 0..n {
                    players.push(Self::player(id, pos, 120, 27));
                    id += 1;
                }
            };
            push(PlayerPositionType::Goalkeeper, 2);
            push(PlayerPositionType::DefenderCenter, 8);
            push(PlayerPositionType::MidfielderCenter, 6);
            push(PlayerPositionType::Striker, 6);
            players
        }

        /// The live-site picture: an eight-man U20, already under
        /// `SquadSize::MIN_YOUTH`.
        fn u20_roster(candidate: Player) -> Vec<Player> {
            let mut players = vec![candidate];
            let mut id = 300u32;
            for _ in 0..7 {
                players.push(Self::player(id, PlayerPositionType::DefenderCenter, 50, 18));
                id += 1;
            }
            players
        }

        fn club(candidate: Player) -> Club {
            let main = TeamBuilder::new()
                .id(10)
                .league_id(Some(1))
                .club_id(100)
                .name("Main".to_string())
                .slug("main".to_string())
                .team_type(TeamType::Main)
                .players(PlayerCollection::new(Self::main_roster()))
                .staffs(StaffCollection::new(Vec::new()))
                .reputation(TeamReputation::new(500, 500, 500))
                .training_schedule(Self::schedule())
                .build()
                .unwrap();
            let u20 = TeamBuilder::new()
                .id(20)
                .league_id(None)
                .club_id(100)
                .name("U20".to_string())
                .slug("u20".to_string())
                .team_type(TeamType::U20)
                .players(PlayerCollection::new(Self::u20_roster(candidate)))
                .staffs(StaffCollection::new(Vec::new()))
                .reputation(TeamReputation::new(300, 300, 300))
                .training_schedule(Self::schedule())
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
                TeamCollection::new(vec![main, u20]),
                ClubFacilities::default(),
            )
        }

        fn on_main(club: &Club, id: u32) -> bool {
            club.teams.teams[0]
                .players
                .players
                .iter()
                .any(|p| p.id == id)
        }

        fn find(club: &Club, id: u32) -> &Player {
            club.teams
                .teams
                .iter()
                .flat_map(|t| t.players.players.iter())
                .find(|p| p.id == id)
                .expect("player must stay somewhere in the club")
        }
    }

    #[test]
    fn an_eight_man_youth_squad_still_releases_a_first_team_ready_player() {
        let candidate = Fx::player(1, PlayerPositionType::MidfielderCenter, Fx::READY, 19);
        let mut club = Fx::club(candidate);
        assert!(club.teams.teams[1].players.len() < SquadSize::MIN_YOUTH);

        club.rebalance_squads(Fx::date());

        assert!(
            Fx::on_main(&club, 1),
            "fielding the youth side is the academy's job — it cannot veto a promotion"
        );
    }

    /// …but only when he clears the bar by the margin. A prospect who is
    /// merely at the bar is still held by the squad-size guard, which is
    /// what stops the youth side being emptied by ordinary churn.
    #[test]
    fn a_marginal_prospect_is_still_held_by_the_squad_minimum() {
        // One point over the worst senior in his group, nowhere near
        // `SquadSize::PROMOTION_CLEAR_MARGIN` clear of it.
        let candidate = Fx::player(1, PlayerPositionType::MidfielderCenter, 122, 19);
        let mut club = Fx::club(candidate);

        club.rebalance_squads(Fx::date());

        assert!(!Fx::on_main(&club, 1));
    }

    #[test]
    fn a_loan_intent_is_withdrawn_when_the_boy_is_promoted_instead() {
        let mut candidate = Fx::player(1, PlayerPositionType::MidfielderCenter, Fx::READY, 19);
        candidate.statuses.add(Fx::date(), PlayerStatusType::Loa);
        candidate.plan = Some(PlayerPlan::from_existing(
            PlayerPlanRole::Development,
            PathwayStage::LoanOut,
            PlayerFieldPositionGroup::Midfielder,
            19,
            Fx::date(),
        ));
        let mut club = Fx::club(candidate);
        club.transfer_plan
            .loan_out_candidates
            .push(LoanOutCandidate {
                player_id: 1,
                reason: LoanOutReason::DevelopmentPathway,
                status: LoanOutStatus::Listed,
                loan_fee: 0.0,
                preferred_destination: LoanDestinationPreference::Any,
                from_pathway: false,
                band_target: None,
            });

        club.rebalance_squads(Fx::date());

        assert!(Fx::on_main(&club, 1), "promotion beats a loan intent");
        let promoted = Fx::find(&club, 1);
        assert!(
            !promoted.statuses.has(PlayerStatusType::Loa),
            "the badge goes with the intent"
        );
        assert!(club.transfer_plan.loan_out_candidates.is_empty());
        assert_eq!(
            promoted.pathway_stage(),
            PathwayStage::Rotation,
            "the stage is the same decision as the badge and the row"
        );
        assert_eq!(
            club.transfer_plan.loan_withdrawals,
            vec![1],
            "the country pass is told to pull the live listing"
        );
        assert!(
            promoted
                .decision_history
                .items
                .iter()
                .any(|d| d.decision == "dec_reason_promoted_instead"),
            "the withdrawal is recorded on the player"
        );
    }

    /// `Lst` keeps blocking. A sale is a decision the club has made about
    /// him; the first team is not where a listed player waits for it.
    #[test]
    fn a_transfer_listing_still_blocks_the_promotion() {
        let mut candidate = Fx::player(1, PlayerPositionType::MidfielderCenter, Fx::READY, 19);
        candidate.statuses.add(Fx::date(), PlayerStatusType::Lst);
        let mut club = Fx::club(candidate);

        club.rebalance_squads(Fx::date());

        assert!(!Fx::on_main(&club, 1));
        assert!(club.transfer_plan.loan_withdrawals.is_empty());
    }
}

#[cfg(test)]
mod squad_balance_tests {
    use super::*;
    use crate::academy::ClubAcademy;
    use crate::club::player::core::builder::PlayerBuilder;
    use crate::shared::Location;
    use crate::shared::fullname::FullName;
    use crate::{
        ClubColors, ClubFacilities, ClubFinances, ClubStatus, PersonAttributes, Player,
        PlayerAttributes, PlayerClubContract, PlayerCollection, PlayerPosition, PlayerPositionType,
        PlayerPositions, PlayerSkills, StaffCollection, TeamBuilder, TeamCollection,
        TeamReputation, TrainingSchedule,
    };
    use chrono::{Datelike, NaiveTime};

    struct Fx;

    impl Fx {
        fn date() -> NaiveDate {
            NaiveDate::from_ymd_opt(2026, 10, 1).unwrap()
        }

        fn player(id: u32, position: PlayerPositionType, ability: u8, age: u8) -> Player {
            let date = Self::date();
            let mut attrs = PlayerAttributes::default();
            attrs.current_ability = ability;
            attrs.potential_ability = ability;
            attrs.condition = 10_000;
            PlayerBuilder::new()
                .id(id)
                .full_name(FullName::new("T".to_string(), format!("P{id}")))
                .birth_date(NaiveDate::from_ymd_opt(date.year() - age as i32, 1, 1).unwrap())
                .country_id(1)
                .attributes(PersonAttributes::default())
                .skills(PlayerSkills::flat_for_ability(ability))
                .positions(PlayerPositions {
                    positions: vec![PlayerPosition {
                        position,
                        level: 18,
                    }],
                })
                .player_attributes(attrs)
                .contract(Some(PlayerClubContract::new(
                    20_000,
                    NaiveDate::from_ymd_opt(2029, 6, 30).unwrap(),
                )))
                .build()
                .unwrap()
        }

        fn many(
            first_id: u32,
            position: PlayerPositionType,
            n: u32,
            ability: u8,
            age: u8,
        ) -> Vec<Player> {
            (first_id..first_id + n)
                .map(|id| Self::player(id, position, ability, age))
                .collect()
        }

        /// Seniors at 120: two keepers, eight defenders, `mids` central
        /// midfielders and six strikers — 22 with six midfielders.
        fn main_roster(mids: u32) -> Vec<Player> {
            let mut players = Self::many(100, PlayerPositionType::Goalkeeper, 2, 120, 27);
            players.extend(Self::many(
                110,
                PlayerPositionType::DefenderCenter,
                8,
                120,
                27,
            ));
            players.extend(Self::many(
                130,
                PlayerPositionType::MidfielderCenter,
                mids,
                120,
                27,
            ));
            players.extend(Self::many(150, PlayerPositionType::Striker, 6, 120, 27));
            players
        }

        fn team(id: u32, tt: TeamType, league: bool, players: Vec<Player>) -> crate::Team {
            TeamBuilder::new()
                .id(id)
                .league_id(league.then_some(id))
                .club_id(100)
                .name(format!("{tt:?}"))
                .slug(format!("{tt:?}").to_lowercase())
                .team_type(tt)
                .players(PlayerCollection::new(players))
                .staffs(StaffCollection::new(Vec::new()))
                .reputation(TeamReputation::new(400, 400, 400))
                .training_schedule(TrainingSchedule::new(
                    NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
                    NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
                ))
                .build()
                .unwrap()
        }

        fn club(teams: Vec<crate::Team>) -> Club {
            Club::new(
                100,
                "Club".to_string(),
                Location::new(1),
                ClubFinances::new(10_000_000, Vec::new()),
                ClubAcademy::new(3),
                ClubStatus::Professional,
                ClubColors::default(),
                TeamCollection::new(teams),
                ClubFacilities::default(),
            )
        }

        fn size(club: &Club, tt: TeamType) -> usize {
            club.teams
                .teams
                .iter()
                .find(|t| t.team_type == tt)
                .map_or(0, |t| t.players.len())
        }
    }

    /// Spartak's day-one shape: the second team holds forty boys of U19 age
    /// while the U19 cannot name a side. The U19 takes its own age group back.
    #[test]
    fn a_short_youth_side_takes_its_age_group_back_from_the_second_team() {
        let mut second = Fx::many(500, PlayerPositionType::DefenderCenter, 16, 55, 18);
        second.extend(Fx::many(
            520,
            PlayerPositionType::MidfielderCenter,
            16,
            55,
            18,
        ));
        second.extend(Fx::many(540, PlayerPositionType::Striker, 8, 55, 18));
        let mut u19 = Fx::many(300, PlayerPositionType::Goalkeeper, 2, 55, 17);
        u19.extend(Fx::many(
            310,
            PlayerPositionType::MidfielderCenter,
            4,
            55,
            17,
        ));
        let mut club = Fx::club(vec![
            Fx::team(10, TeamType::Main, true, Fx::main_roster(6)),
            Fx::team(80, TeamType::Second, true, second),
            Fx::team(19, TeamType::U19, true, u19),
        ]);

        club.rebalance_squads(Fx::date());

        assert_eq!(Fx::size(&club, TeamType::U19), SquadSize::YOUTH_MATCHDAY);
        assert_eq!(Fx::size(&club, TeamType::Second), 40 - 12);
    }

    /// A second team that is itself down to a matchday squad keeps it.
    #[test]
    fn the_second_team_never_drops_below_its_own_matchday_squad() {
        let second = Fx::many(500, PlayerPositionType::DefenderCenter, 23, 55, 18);
        let u19 = Fx::many(300, PlayerPositionType::MidfielderCenter, 6, 55, 17);
        let mut club = Fx::club(vec![
            Fx::team(10, TeamType::Main, true, Fx::main_roster(6)),
            Fx::team(80, TeamType::Second, true, second),
            Fx::team(19, TeamType::U19, true, u19),
        ]);

        club.rebalance_squads(Fx::date());

        assert_eq!(Fx::size(&club, TeamType::Second), SquadSize::MIN_MAIN);
        assert_eq!(Fx::size(&club, TeamType::U19), 7);
    }

    /// A U21 that plays no fixtures has nothing to fill, and its boys are
    /// the first handed back to a U19 that does.
    #[test]
    fn a_league_less_squad_gives_its_boys_to_the_one_that_plays() {
        let u21 = Fx::many(700, PlayerPositionType::MidfielderCenter, 5, 50, 15);
        let u19 = Fx::many(300, PlayerPositionType::DefenderCenter, 6, 55, 17);
        let mut club = Fx::club(vec![
            Fx::team(10, TeamType::Main, true, Fx::main_roster(6)),
            Fx::team(21, TeamType::U21, false, u21),
            Fx::team(19, TeamType::U19, true, u19),
        ]);

        club.rebalance_squads(Fx::date());

        assert_eq!(Fx::size(&club, TeamType::U21), 0);
        assert_eq!(Fx::size(&club, TeamType::U19), 11);
    }

    /// A first team one midfielder short opens one place, not a door: the
    /// best candidate fills it and the rest stay where they play.
    #[test]
    fn a_hole_in_the_first_team_takes_one_man_not_every_youth_above_the_gap_floor() {
        let second = Fx::many(500, PlayerPositionType::MidfielderCenter, 22, 70, 22);
        let mut club = Fx::club(vec![
            Fx::team(10, TeamType::Main, true, Fx::main_roster(5)),
            Fx::team(80, TeamType::Second, true, second),
        ]);
        let main_before = Fx::size(&club, TeamType::Main);

        club.rebalance_squads(Fx::date());

        assert_eq!(Fx::size(&club, TeamType::Main), main_before + 1);
    }
}
