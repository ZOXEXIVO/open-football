//! One club's scouting day, from the department's reach down to the report
//! a scout files on the man he watched.
//!
//! Pass 1 of the scouting tick, hoisted per club so the scan parallelizes.
//! Strictly read-only against the world: every mutation is staged on the
//! [`ClubScoutingStaged`] the scan returns, and pass 2 commits it.
//!
//! The club is read once, in [`ClubScan::new`]: which regions its network
//! reaches, and its [`Suitor`] account, which every candidate is read against
//! through the gate's one entry.

use super::*;
use crate::StaffKnowledge;
use crate::transfers::gate::{MoveTerms, PursuitAction, TransferMovePlausibility};
use crate::transfers::scouting::ScoutingPass;
use crate::transfers::scouting::judgement::ScoutJudgement;
use crate::transfers::view::club::ClubView;

/// One club's scouting scan.
pub(in crate::transfers::scouting) struct ClubScan<'a> {
    club: &'a Club,
    performance_lookup: &'a LeaguePerformanceLookup,
    config: &'a ScoutingConfig,
    date: NaiveDate,
    market_map: &'a MarketMap,
    country_id: u32,
    /// The buying country's language(s) — the data pre-filter nudges the
    /// foreign shortlist toward candidates who could communicate in the
    /// dressing room (local language first, English/Spanish as bridges).
    home_language_mask: u64,
    suitor: Suitor<'a>,
    /// Which regions of the world this club can spot talent in at all.
    club_scout_reach: FxHashSet<ScoutingRegion>,
    staged: ClubScoutingStaged,
}

/// What the scout came back with after a viewing: the numbers he put on the
/// man, and the sample they rest on.
struct TargetAssessment {
    obs_count: u32,
    target_region: ScoutingRegion,
    assessed_ability: u8,
    assessed_potential: u8,
}

impl<'a> ClubScan<'a> {
    pub(in crate::transfers::scouting) fn new(
        country: &'a Country,
        club: &'a Club,
        performance_lookup: &'a LeaguePerformanceLookup,
        config: &'a ScoutingConfig,
        date: NaiveDate,
        market_map: &'a MarketMap,
    ) -> Self {
        // The buying country's language(s) — the data pre-filter nudges the
        // foreign shortlist toward candidates who could communicate in the
        // dressing room (local language first, English/Spanish as bridges).
        let home_language_mask = Language::country_language_mask(&country.code);

        let suitor = Suitor::of(country, club, date);

        // The club's scouting NETWORK reach — which regions of the world it
        // can spot talent in. Widens CONTINUOUSLY with reputation (see
        // `reputation_scout_regions`): a minnow sees only its own backyard, a
        // mid club its main trade corridors, a giant the whole globe. No hard
        // tier cutoff. Built once per club and shared by every assignment.
        let home_region = ScoutingRegion::from_country(country.continent_id, &country.code);
        let club_overall_score = club
            .teams
            .main()
            .or_else(|| club.teams.teams.first())
            .map(|t| t.reputation.overall_score())
            .unwrap_or(0.0);
        let club_scout_reach: FxHashSet<ScoutingRegion> =
            ScoutingPass::reputation_scout_regions(home_region, club_overall_score)
                .into_iter()
                .collect();

        Self {
            club,
            performance_lookup,
            config,
            date,
            market_map,
            country_id: country.id,
            home_language_mask,
            suitor,
            club_scout_reach,
            staged: ClubScoutingStaged {
                observations: Vec::new(),
                reports: Vec::new(),
                staff_events: Vec::new(),
                familiarity_events: Vec::new(),
                rejected_events: Vec::new(),
                wanted_targets: Vec::new(),
                monitoring_updates: Vec::new(),
            },
        }
    }

    /// Every open assignment on the club's plan, in order.
    pub(in crate::transfers::scouting) fn run(
        mut self,
        domestic_by_group: &[Vec<&PlayerSummary>; PlayerFieldPositionGroup::COUNT],
        foreign_by_group: &[Vec<&PlayerSummary>; PlayerFieldPositionGroup::COUNT],
    ) -> ClubScoutingStaged {
        let club = self.club;
        let plan = &club.transfer_plan;
        for assignment in &plan.scouting_assignments {
            self.scan_assignment(assignment, domestic_by_group, foreign_by_group);
        }
        self.staged
    }

    /// One shirt's day: who is in reach for it, who the data department
    /// surfaces, and how many of them the scout on it gets to watch.
    fn scan_assignment(
        &mut self,
        assignment: &ScoutingAssignment,
        domestic_by_group: &[Vec<&PlayerSummary>; PlayerFieldPositionGroup::COUNT],
        foreign_by_group: &[Vec<&PlayerSummary>; PlayerFieldPositionGroup::COUNT],
    ) {
        if assignment.completed {
            return;
        }

        let club = self.club;
        let config = self.config;

        let (judging_ability, judging_potential) = if let Some(scout_id) = assignment.scout_staff_id
        {
            ClubView::get_scout_skills(club, scout_id)
        } else {
            let d = config.observation.default_judging_when_no_scout;
            (d, d)
        };

        // Borrow the scout's knowledge struct once — we need both
        // known_regions (slice) and familiarity (per-region lookup).
        let scout_knowledge = assignment
            .scout_staff_id
            .and_then(|sid| {
                club.teams
                    .iter()
                    .flat_map(|t| t.staffs.iter())
                    .find(|s| s.id == sid)
            })
            .map(|s| &s.staff_attributes.knowledge);

        const EMPTY_REGIONS: &[ScoutingRegion] = &[];
        let scout_known_regions: &[ScoutingRegion] = scout_knowledge
            .map(|k| k.known_regions.as_slice())
            .unwrap_or(EMPTY_REGIONS);

        let observe_chance = config.daily_observation_chance(judging_ability);
        if IntegerUtils::random(0, 100) > observe_chance {
            return;
        }

        if let Some(scout_id) = assignment.scout_staff_id {
            self.staged
                .staff_events
                .push((club.id, scout_id, StaffEventType::PlayerScouted));
        }

        let matching = self.candidates(
            assignment,
            scout_known_regions,
            domestic_by_group,
            foreign_by_group,
        );
        if matching.is_empty() {
            return;
        }
        let matching = self.prefilter(matching);

        let obs_per_day = config.observations_per_day(judging_ability);

        for _obs_round in 0..obs_per_day.min(matching.len()) {
            self.observation_round(
                assignment,
                &matching,
                judging_ability,
                judging_potential,
                scout_knowledge,
                scout_known_regions,
            );
        }
    }

    /// Everyone this assignment could name: the domestic pool in full, and the
    /// foreign pool as far as the club's network — or a scout's own patch —
    /// reaches. Read-only, and deliberately generous: the geography enters as
    /// a graded preference in [`Self::prefilter`], not as a wall here.
    fn candidates<'p>(
        &self,
        assignment: &ScoutingAssignment,
        scout_known_regions: &[ScoutingRegion],
        domestic_by_group: &[Vec<&'p PlayerSummary>; PlayerFieldPositionGroup::COUNT],
        foreign_by_group: &[Vec<&'p PlayerSummary>; PlayerFieldPositionGroup::COUNT],
    ) -> Vec<&'p PlayerSummary> {
        let club = self.club;
        let date = self.date;
        let config = self.config;
        let suitor = &self.suitor;
        let market_map = self.market_map;
        let club_scout_reach = &self.club_scout_reach;

        // Find matching players from OTHER clubs (domestic + foreign known regions)
        let target_group = assignment.target_position.position_group();
        let philosophy = &club.philosophy;

        // DevelopAndSell clubs widen the net for young promising players
        let (age_min, age_max, ability_floor) = match philosophy {
            ClubPhilosophy::DevelopAndSell => {
                let youth_floor = assignment.min_ability.saturating_sub(20);
                (
                    assignment.preferred_age_min.min(16),
                    assignment.preferred_age_max,
                    youth_floor,
                )
            }
            ClubPhilosophy::SignToCompete => (
                assignment.preferred_age_min,
                assignment.preferred_age_max,
                assignment.min_ability,
            ),
            _ => (
                assignment.preferred_age_min,
                assignment.preferred_age_max,
                assignment.min_ability,
            ),
        };

        let player_filter = |p: &&PlayerSummary| -> bool {
            // Capability, not label: the assignment names a shirt, and
            // anyone who can wear it is a candidate for it. The pools
            // are already bucketed the same way, so this only re-states
            // the contract for callers that pass an unbucketed slice.
            if p.club_id == club.id
                || !p.coverage.covers_group(target_group)
                || club.is_rival(p.club_id)
            {
                return false;
            }
            if club.transfer_plan.is_rejected(p.player_id, date) {
                return false;
            }
            let effective_min =
                if p.age <= 21 && matches!(philosophy, ClubPhilosophy::DevelopAndSell) {
                    ability_floor
                } else {
                    assignment.min_ability
                };
            // Gate on ability blended with sustained MATCH OUTPUT —
            // the same `stats_bonus` (rating over a real appearance
            // sample) the scout applies downstream when grading the
            // report. The gate previously read raw skill only, so a
            // modest-CA striker banging in goals (a high-rated
            // season) was filtered out of the pool before any club
            // that could afford him ever looked: the overperformer
            // was invisible by construction. A thin sample yields a
            // ~0 bonus, so a two-game fluke still can't sneak in.
            let form_bonus = config.stats_bonus(p.appearances, p.average_rating);
            let effective_ability = (p.skill_ability as i16 + form_bonus).clamp(1, 200) as u8;
            // The gate is the dearest read here, so it runs last, on the
            // few the shirt's age and level have not already ruled out.
            p.age >= age_min
                && p.age <= age_max
                && effective_ability >= effective_min
                && TransferMovePlausibility::read(suitor, p, MoveTerms::purchase(true), market_map)
                    .reaches(PursuitAction::FillNow.needs())
        };

        // Domestic players (always visible) — this assignment's
        // position group only; the other groups can't pass
        // `player_filter` anyway.
        let mut matching: Vec<&PlayerSummary> = domestic_by_group[target_group.index()]
            .iter()
            .copied()
            .filter(player_filter)
            .collect();

        // Foreign players are visible if their region falls inside the
        // club's reputation-driven network reach OR a scout personally
        // knows it. `club_scout_reach` always holds at least the home
        // region, so the foreign sweep runs for every club; how far it
        // reaches is what scales with reputation.
        let foreign_matching: Vec<&PlayerSummary> = foreign_by_group[target_group.index()]
            .iter()
            .copied()
            .filter(|p| {
                club_scout_reach.contains(&p.region) || scout_known_regions.contains(&p.region)
            })
            .filter(player_filter)
            .collect();
        matching.extend(foreign_matching);

        matching
    }

    /// The club's data department narrows the pool from "everyone who matches
    /// position/age/ability" to "people the numbers say deserve an eye-test."
    /// Higher data skill = tighter pool with less noise; low skill ≈ random.
    fn prefilter<'p>(&self, mut matching: Vec<&'p PlayerSummary>) -> Vec<&'p PlayerSummary> {
        let club = self.club;
        let config = self.config;
        let country_id = self.country_id;
        let home_language_mask = self.home_language_mask;
        let market_map = self.market_map;
        let performance_lookup = self.performance_lookup;
        let suitor = &self.suitor;

        // Data-first pre-filter: the club's data department narrows the
        // candidate pool from "everyone who matches position/age/ability"
        // to "people the numbers say deserve an eye-test." Higher data
        // skill = tighter pool with less noise; low skill ≈ random.
        // This is what real clubs do — Opta/Wyscout shortlists come
        // first, scouts watch the narrowed list in person.
        let data_skill = ClubView::club_data_analysis_skill(club);
        if let Some(target_pool) = config.data_prefilter_target(matching.len(), data_skill) {
            let noise = config.data_prefilter_noise(data_skill);
            let mut scored: Vec<(&PlayerSummary, f32)> = matching
                .iter()
                .map(|p| {
                    let score = ScoutJudgement::player_data_score(p, performance_lookup);
                    let jitter = IntegerUtils::random(-noise, noise) as f32;
                    // Language affinity — a graded preference, not a
                    // gate: a foreign candidate who speaks the club
                    // country's language (or a football bridge
                    // language) rises in the eye-test shortlist, one
                    // with no common language sinks. Domestic
                    // candidates are unaffected — playing in the
                    // league already answers the communication
                    // question.
                    let language_bonus = if p.country_id != country_id {
                        (p.language_profile.affinity_for(home_language_mask) - 0.5) * 24.0
                    } else {
                        0.0
                    };
                    // Geography — a graded preference, like the
                    // language term beside it, and for the same
                    // reason: the club's data department surfaces
                    // players from markets the club works, because
                    // those are the markets it has data on and
                    // people in. Falls to a small penalty for a
                    // country nobody here has ever signed from,
                    // never to a wall: a scout may watch anyone.
                    let market_bonus = if p.country_id != country_id {
                        (suitor.reach_to(p, market_map) - 0.35) * 24.0
                    } else {
                        0.0
                    };
                    (*p, score + jitter + language_bonus + market_bonus)
                })
                .collect();
            scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(Ordering::Equal));
            matching = scored
                .into_iter()
                .take(target_pool)
                .map(|(p, _)| p)
                .collect();
        }

        matching
    }

    /// One viewing: pick the man, grade him, file what it produced.
    fn observation_round(
        &mut self,
        assignment: &ScoutingAssignment,
        matching: &[&PlayerSummary],
        judging_ability: u8,
        judging_potential: u8,
        scout_knowledge: Option<&StaffKnowledge>,
        scout_known_regions: &[ScoutingRegion],
    ) {
        let config = self.config;
        let country_id = self.country_id;
        let target = Self::pick_target(assignment, matching, config);

        // How often this scout has watched the man, read off the department's
        // durable file rather than the assignment's own ledger. An assignment
        // retires after its first report, so its ledger never records a second
        // viewing — reading it pinned every pool report at the
        // single-observation confidence and put the meeting bar out of reach.
        let obs_count = assignment
            .scout_staff_id
            .and_then(|sid| {
                self.club
                    .transfer_plan
                    .find_monitoring(sid, target.player_id)
            })
            .map(|m| m.times_watched as u32)
            .unwrap_or_else(|| {
                assignment
                    .observations
                    .iter()
                    .find(|o| o.player_id == target.player_id)
                    .map(|o| o.observation_count)
                    .unwrap_or(0)
            });

        // Region penalty blends structural knowledge (in known_regions?)
        // with empirical experience (familiarity, 0-100). A veteran
        // scout who's been scouting a region for years is sharper than
        // a brand-new assignee, even to a "known" region.
        let target_region = ScoutingRegion::from_country(target.continent_id, &target.country_code);
        let is_domestic = target.country_id == country_id;
        let is_known_region = scout_known_regions.contains(&target_region);
        let familiarity = scout_knowledge
            .map(|k| k.familiarity_for(target_region))
            .unwrap_or(0);
        let region_penalty = config.region_penalty(is_domestic, is_known_region, familiarity);

        let ability_error =
            config.effective_error(judging_ability, obs_count as u8, region_penalty, false);
        let potential_error =
            config.effective_error(judging_potential, obs_count as u8, region_penalty, false);

        // Assess ability from visible skills, boosted by match performance
        let performance_bonus = config.performance_bonus(target.appearances, target.average_rating);

        let assessed_ability = (target.skill_ability as i32
            + performance_bonus
            + IntegerUtils::random(-ability_error, ability_error))
        .clamp(1, 200) as u8;

        // Estimate potential from age, mental attributes, and current skill level
        // Young players with strong mentals (determination, work rate) suggest higher ceiling
        let growth_potential = ScoutJudgement::estimate_growth_potential(
            target.age,
            target.determination,
            target.work_rate,
            target.composure,
            target.anticipation,
            target.skill_ability,
        );
        let assessed_potential = (target.skill_ability as i32
            + growth_potential as i32
            + IntegerUtils::random(-potential_error, potential_error))
        .clamp(1, 200) as u8;

        self.file_observation(
            assignment,
            target,
            &TargetAssessment {
                obs_count,
                target_region,
                assessed_ability,
                assessed_potential,
            },
        );
    }

    /// Deepen existing knowledge most of the time, widen the pool
    /// occasionally — and when deepening, go back to the man seen LEAST.
    fn pick_target<'p>(
        assignment: &ScoutingAssignment,
        matching: &'p [&'p PlayerSummary],
        config: &ScoutingConfig,
    ) -> &'p PlayerSummary {
        // Configurable re-observe vs discover chance: deepen
        // existing knowledge most of the time, widen the pool
        // occasionally. Default ~60/40.
        let re_observe_chance = config.observation.re_observe_chance_pct;
        let already_observed_ids: Vec<u32> = assignment
            .observations
            .iter()
            .map(|o| o.player_id)
            .collect();

        (if !already_observed_ids.is_empty() && IntegerUtils::random(0, 100) < re_observe_chance {
            // Go back to a player already on the watch list —
            // preferring the one seen LEAST. This used to take the
            // first match in prefilter order, which is a fixed
            // choice: the scout re-watched the same single name
            // every time the re-observe branch fired, so his
            // confidence deepened on one player and the rest of his
            // list stayed at one viewing forever. Weighting by how
            // little he has seen a man spreads the coverage the way
            // a scout actually builds a picture, and the draw keeps
            // it from being another fixed order.
            let seen: Vec<(&&PlayerSummary, f32)> = matching
                .iter()
                .filter(|p| already_observed_ids.contains(&p.player_id))
                .map(|p| {
                    let times = assignment
                        .observations
                        .iter()
                        .find(|o| o.player_id == p.player_id)
                        .map(|o| o.observation_count)
                        .unwrap_or(0);
                    // The draw raises weights to its own sharpness
                    // exponent, so take the root here: the odds a
                    // man gets the next viewing then fall as a
                    // plain inverse of how often he has already
                    // been seen. A scout catches up on the names he
                    // knows least without ever abandoning the one
                    // he most wants a second look at.
                    (
                        p,
                        (1.0 / (1.0 + times as f32)).powf(1.0 / InterestDraw::SHARPNESS),
                    )
                })
                .collect();
            let slate: Vec<(u32, f32)> = seen
                .iter()
                .enumerate()
                .map(|(i, (_, w))| (i as u32, *w))
                .collect();
            match InterestDraw::pick(&slate) {
                Some(i) => seen[i as usize].0,
                None => matching.first().unwrap(),
            }
        } else {
            // Discover new player — reputation-weighted selection
            // Famous players are more visible to scouts (media coverage, word of mouth)
            let new_players: Vec<&&PlayerSummary> = matching
                .iter()
                .filter(|p| !already_observed_ids.contains(&p.player_id))
                .collect();
            if !new_players.is_empty() {
                ScoutingPass::pick_reputation_weighted(&new_players)
            } else {
                ScoutingPass::pick_reputation_weighted(&matching.iter().collect::<Vec<_>>())
            }
        }) as _
    }

    /// The observation, the monitoring row, and — when the scout rates him and
    /// the move could credibly go public — the interest the club shows out loud.
    fn file_observation(
        &mut self,
        assignment: &ScoutingAssignment,
        target: &PlayerSummary,
        assessed: &TargetAssessment,
    ) {
        let club = self.club;
        let config = self.config;
        let country_id = self.country_id;
        let market_map = self.market_map;
        let suitor = &self.suitor;
        let obs_count = assessed.obs_count;
        let target_region = assessed.target_region;
        let assessed_ability = assessed.assessed_ability;
        let assessed_potential = assessed.assessed_potential;
        let observations = &mut self.staged.observations;
        let reports = &mut self.staged.reports;
        let familiarity_events = &mut self.staged.familiarity_events;
        let rejected_events = &mut self.staged.rejected_events;
        let wanted_targets = &mut self.staged.wanted_targets;
        let monitoring_updates = &mut self.staged.monitoring_updates;

        let is_new = !assignment.has_observation_for(target.player_id);
        let assessment =
            TransferMovePlausibility::read(suitor, target, MoveTerms::purchase(true), market_map);

        // Skip if we already queued an observation for this player this round
        if observations.iter().any(|o| {
            o.club_id == club.id
                && o.assignment_id == assignment.id
                && o.player_id == target.player_id
        }) {
            return;
        }

        observations.push(ScoutingObservationResult {
            club_id: club.id,
            assignment_id: assignment.id,
            player_id: target.player_id,
            assessed_ability,
            assessed_potential,
            is_new,
        });

        if let Some(scout_id) = assignment.scout_staff_id
            && target.country_id != country_id
        {
            familiarity_events.push((club.id, scout_id, target_region, target.country_id));
        }

        let final_obs_count = obs_count + 1;
        let confidence = config.pool_report_confidence(final_obs_count as u8);
        let youth_bonus = config.youth_bonus(target.age, assessed_ability, assessed_potential);
        let stats_bonus = config.stats_bonus(target.appearances, target.average_rating);
        let effective_ability = assessed_ability as i16 + youth_bonus + stats_bonus;
        let recommendation = config.recommendation_for(
            effective_ability,
            assessed_ability,
            assessed_potential,
            assignment.min_ability,
        );

        let role_fit_now = assignment.role_profile.fit(
            target.technical_avg,
            target.mental_avg,
            target.physical_avg,
        );
        let risk_flags_now = ScoutJudgement::evaluate_risk_flags(
            target.is_injured,
            target.determination,
            target.age,
            target.contract_months_remaining,
            target.world_reputation,
            ClubView::club_world_reputation(club),
        );

        // Always update the monitoring row when a real scout
        // is on this assignment — even if the recommendation
        // is Pass, the scout has formed an opinion that the
        // recruitment meeting will see.
        if let Some(scout_id) = assignment.scout_staff_id {
            monitoring_updates.push(MonitoringUpdate {
                club_id: club.id,
                reading: FileReading::of(&assessment, self.date),
                seen: FileObservation {
                    scout_staff_id: scout_id,
                    player_id: target.player_id,
                    source: ScoutMonitoringSource::TransferRequest,
                    transfer_request_id: Some(assignment.transfer_request_id),
                    origin_assignment_id: Some(assignment.id),
                    region: Some(target_region),
                    assessed_ability,
                    assessed_potential,
                    confidence,
                    role_fit: role_fit_now,
                    estimated_value: target.estimated_value,
                    risk_flags: risk_flags_now.clone(),
                    is_match: false,
                },
            });
        }

        if recommendation == ScoutingRecommendation::Pass {
            rejected_events.push((club.id, target.player_id));
        } else {
            // Public interest (`Wnt`) is separated from the
            // scouting report. A scout liking a player produces
            // a private report (which still feeds the internal
            // shortlist) — it does NOT, on its own, make the
            // interest public. `Wnt` is set only when:
            //   * the report is a Buy / StrongBuy (a Consider is
            //     private monitoring), AND
            //   * the move clears the public-interest stage of
            //     the shared plausibility model (the player /
            //     agent wouldn't immediately dismiss it — level,
            //     affordability and willingness are credible).
            // A lower club can therefore quietly scout a strong
            // first-team player at a bigger club without ever
            // setting `Wnt`.
            let buyable = matches!(
                recommendation,
                ScoutingRecommendation::StrongBuy | ScoutingRecommendation::Buy
            );
            let public_interest_ok = buyable && assessment.reaches(PursuitAction::GoPublic.needs());

            if public_interest_ok {
                if !wanted_targets.contains(&(club.id, target.player_id)) {
                    wanted_targets.push((club.id, target.player_id));
                }
            } else if buyable {
                // The scout rates him a buy, but the move can't
                // credibly go public yet — keep the private report
                // (it still feeds the internal shortlist) and
                // record WHY no public interest was shown.
                debug!(
                    "scouting: club {} watched player {} privately, no public interest — {}",
                    club.id,
                    target.player_id,
                    assessment.diagnostics.explain()
                );
            }
            reports.push(ScoutingReportResult {
                club_id: club.id,
                report: DetailedScoutingReport {
                    player_id: target.player_id,
                    assignment_id: assignment.id,
                    assessed_ability,
                    assessed_potential,
                    confidence,
                    estimated_value: target.estimated_value,
                    recommendation,
                    role_fit: role_fit_now,
                    risk_flags: risk_flags_now,
                },
                assignment_id: assignment.id,
            });
        }
    }
}

#[cfg(test)]
mod countryman_tests {
    use super::*;
    use crate::club::player::transfer::LevelFloor;
    use crate::transfers::tests::kit::{TestClub, TestCountry, TestDate, TestSummary};
    use crate::{PlayerPositionType, PlayerSquadStatus};

    /// A club in a small country, and a countryman on the fringe of a club
    /// in a far stronger one whose floor has widened to its division and
    /// whose ask it could hold. The country he plays in outranks its own;
    /// the gate, not that, decides whether it may watch him.
    #[test]
    fn a_countryman_abroad_may_be_watched_from_a_smaller_country() {
        let mut club = TestClub::new(1).reputation(2_500).build();
        club.transfer_plan.total_budget = 5_000_000.0;
        let country = TestCountry::new(1)
            .code("pt")
            .reputation(3_000)
            .league_reputation(2_500)
            .clubs(vec![club])
            .build();
        let mut countryman = TestSummary::new(40)
            .club_id(9)
            .country(2, "es")
            .nationality(1)
            .ability(100)
            .league(8_000, 0.85)
            .squad_status(PlayerSquadStatus::MainBackupPlayer)
            .rank(3)
            .appearances(2, 20)
            .salary(60_000)
            .value(300_000.0)
            .level_floor(LevelFloor {
                standing: 3_000.0,
                tolerance: 1_000.0,
                plays_in: 8_000.0,
            })
            .build();
        countryman.country_reputation = 8_000;
        let foreign = [countryman];
        let assignment = ScoutingAssignment::new(
            1,
            1,
            None,
            PlayerPositionType::MidfielderCenter,
            80,
            18,
            32,
            1_000_000.0,
        );
        let config = ScoutingConfig::default();
        let lookup = LeaguePerformanceLookup::build(&country);
        let map = MarketMap::default();
        let scan = ClubScan::new(
            &country,
            &country.clubs[0],
            &lookup,
            &config,
            TestDate::today(),
            &map,
        );
        let domestic: [Vec<&PlayerSummary>; PlayerFieldPositionGroup::COUNT] = Default::default();
        let candidates = scan.candidates(
            &assignment,
            &[],
            &domestic,
            &ScoutingPass::by_group(foreign.iter()),
        );
        assert_eq!(
            candidates.iter().map(|p| p.player_id).collect::<Vec<_>>(),
            vec![40]
        );
    }
}
