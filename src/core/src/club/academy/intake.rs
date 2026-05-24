use super::{AcademyTier, ClubAcademy};
use crate::academy::result::ProduceYouthPlayersResult;
use crate::context::GlobalContext;
use crate::utils::IntegerUtils;
use crate::{
    AcademyGenerationContext, AcademyIntakeState, PlayerFieldPositionGroup, PlayerGenerator,
    PlayerPositionType,
};
use chrono::Datelike;
use log::debug;

impl ClubAcademy {
    pub(super) fn produce_youth_players(
        &mut self,
        ctx: GlobalContext<'_>,
    ) -> ProduceYouthPlayersResult {
        let current_year = ctx.simulation.date.year();
        let current_month = ctx.simulation.date.month();

        if !self.should_produce_players(current_year, current_month) {
            return ProduceYouthPlayersResult::new(Vec::new());
        }

        let club_name = ctx.club.as_ref().map(|c| c.name).unwrap_or("Unknown Club");

        let recruitment_quality = ctx.club_recruitment_quality();
        let pathway_score = self.pathway_reputation as f32 / 100.0;
        let club_rep_score = ctx.club_main_reputation() as f32 / 10000.0;
        let intake_count =
            self.calculate_annual_intake(recruitment_quality, pathway_score, club_rep_score);

        // Throttle against the academy's hard population cap. The
        // minimum-3 floor only applies when there is at least that much
        // headroom — otherwise we'd push the academy over its cap.
        let current = self.players.players.len();
        let intake_count =
            Self::clamp_intake_to_cap(intake_count, current, self.tuning.max_academy_players);

        if intake_count == 0 {
            // No headroom — still mark the year as processed so the
            // intake doesn't loop every day of the intake month.
            self.last_production_year = Some(current_year);
            return ProduceYouthPlayersResult::new(Vec::new());
        }

        debug!(
            "academy: {} producing {} youth players (level {}, recruitment={:.2}, pathway={:.2})",
            club_name, intake_count, self.level, recruitment_quality, pathway_score
        );

        let country_ctx = ctx.country.as_ref();
        let country_id = country_ctx.map(|c| c.id).unwrap_or(1);
        let people_names = match country_ctx.and_then(|c| c.people_names.as_ref()) {
            Some(names) => names,
            None => return ProduceYouthPlayersResult::new(Vec::new()),
        };

        let gen_ctx = AcademyGenerationContext::from_components(
            self.level,
            ctx.club_facilities_youth(),
            ctx.club_academy_quality(),
            recruitment_quality,
            ctx.club_youth_coaching_quality(),
            ctx.club_main_reputation(),
            ctx.club_league_reputation(),
            ctx.club_country_reputation(),
            self.pathway_reputation,
        );

        // ── Candidate pool ──────────────────────────────────────────
        // The recruiter scouts a wide net, scores each candidate, and
        // signs only the best. Better recruitment widens the search
        // (more candidates) rather than directly inflating every signed
        // player.
        let pool_size = self.calculate_pool_size(intake_count, recruitment_quality);
        let min_pool = self.tuning.min_pool_size.max(intake_count);
        let pool_size = pool_size.max(min_pool);

        let mut pool: Vec<crate::Player> = Vec::with_capacity(pool_size);
        let needs = self.recruitment_priorities.clone();
        let position_assigner = PositionAssigner::new(intake_count, &needs);

        // Generate the candidate pool *without* shared elite-class
        // damping. A scouted-but-unsigned high-PA candidate should not
        // dampen the next candidate's PA — that punished large pools
        // by collapsing the right tail before selection even ran.
        for i in 0..pool_size {
            let position = position_assigner.position_for_pool_index(i);
            let age = IntakeAgeDistribution::Annual.sample();
            let player = PlayerGenerator::generate_with_context(
                country_id,
                ctx.simulation.date.date(),
                position,
                people_names,
                &gen_ctx,
                age as i32,
                age as i32,
                None,
            );
            pool.push(player);
        }

        // Score-then-keep-best. Position-need score and personality
        // score are kept simple but meaningful: a 17-professionalism
        // prospect outranks a 5-professionalism prospect when CA/PA tie.
        let scorer = CandidateScorer::new(&needs);
        let mut scored: Vec<(f32, crate::Player)> = pool
            .into_iter()
            .map(|p| {
                let score = scorer.score(&p);
                (score, p)
            })
            .collect();
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

        // Elite-cluster control is applied at selection time: we walk
        // the sorted list and probabilistically reject *additional*
        // elite/world-class signings so a single class doesn't snap up
        // three world-class teenagers, while still letting the first
        // one through. Non-elite candidates are always eligible. See
        // `EliteSelectionGate` for the gating policy.
        let world_class_pa = self.tuning.world_class_pa_threshold;
        let elite_pa = self.tuning.elite_pa_threshold;
        let mut gate = EliteSelectionGate::new(world_class_pa, elite_pa);
        let mut signed: Vec<crate::Player> = Vec::with_capacity(intake_count);
        let mut rejected: Vec<crate::Player> = Vec::new();
        for (_, player) in scored.into_iter() {
            if signed.len() >= intake_count {
                break;
            }
            if gate.accept(player.player_attributes.potential_ability) {
                signed.push(player);
            } else {
                rejected.push(player);
            }
        }
        // Backfill: if class damping left the intake short, take the
        // best rejected (best-first because `scored` was already
        // sorted) candidates to fill the slots.
        for player in rejected.into_iter() {
            if signed.len() >= intake_count {
                break;
            }
            signed.push(player);
        }
        let generated_players = signed;

        self.last_production_year = Some(current_year);

        ProduceYouthPlayersResult::new(generated_players)
    }

    /// Clamp the calculated intake against the academy population cap.
    ///   * `current >= cap`        → 0
    ///   * `headroom in 1..3`      → at most `headroom`
    ///   * `headroom >= 3`         → at least 3, never above `headroom`
    pub(crate) fn clamp_intake_to_cap(
        calculated_intake: usize,
        current: usize,
        cap: usize,
    ) -> usize {
        if current >= cap {
            return 0;
        }
        let headroom = cap - current;
        let bounded = calculated_intake.min(headroom);
        if headroom >= 3 {
            bounded.max(3)
        } else {
            bounded
        }
    }

    fn should_produce_players(&self, current_year: i32, current_month: u32) -> bool {
        if current_month != self.tuning.intake_month {
            return false;
        }

        match self.last_production_year {
            Some(last_year) if last_year >= current_year => false,
            _ => true,
        }
    }

    /// Annual intake count.
    ///
    ///   base_intake = 4 + tier_norm * 4                          (4..8)
    ///   recruitment_adj = (recruitment - 0.35) * 5
    ///   pathway_adj     = (pathway - 0.50) * 2
    ///   club_adj        = (club_rep - 0.40) * 1.5
    ///
    /// Clamped to [3, `tuning.max_intake`].
    fn calculate_annual_intake(
        &self,
        recruitment_quality: f32,
        pathway_score: f32,
        club_rep_score: f32,
    ) -> usize {
        let tier_norm = AcademyTier::from_level(self.level).norm();
        let base_intake = 4.0 + tier_norm * 4.0;
        let recruitment_adj = (recruitment_quality - 0.35) * 5.0;
        let pathway_adj = (pathway_score - 0.50) * 2.0;
        let club_adj = (club_rep_score - 0.40) * 1.5;
        let raw = base_intake + recruitment_adj + pathway_adj + club_adj;
        let intake = raw.round() as i32;
        (intake.max(3) as usize).min(self.tuning.max_intake)
    }

    /// Candidate-pool size.
    ///
    ///   pool = intake * round(2 + recruitment * 7 + tier_norm * 3)
    /// Clamped to [12, 96].
    fn calculate_pool_size(&self, intake: usize, recruitment_quality: f32) -> usize {
        let tier_norm = AcademyTier::from_level(self.level).norm();
        let multiplier = (2.0 + recruitment_quality * 7.0 + tier_norm * 3.0).round() as i32;
        let raw = intake as i32 * multiplier.max(2);
        (raw.max(12).min(96)) as usize
    }

    pub(super) fn ensure_minimum_players(&mut self, ctx: GlobalContext<'_>) {
        let min_players = self.settings.players_count_range.start as usize;
        let current_count = self.players.players.len();
        if current_count >= min_players {
            return;
        }

        let needed = min_players - current_count;
        let country_ctx = ctx.country.as_ref();
        let country_id = country_ctx.map(|c| c.id).unwrap_or(1);
        let people_names = match country_ctx.and_then(|c| c.people_names.as_ref()) {
            Some(names) => names,
            None => return,
        };
        let date = ctx.simulation.date.date();

        let gen_ctx = AcademyGenerationContext::from_components(
            self.level,
            ctx.club_facilities_youth(),
            ctx.club_academy_quality(),
            ctx.club_recruitment_quality(),
            ctx.club_youth_coaching_quality(),
            ctx.club_main_reputation(),
            ctx.club_league_reputation(),
            ctx.club_country_reputation(),
            self.pathway_reputation,
        );
        let mut intake_state = AcademyIntakeState::new();

        for i in 0..needed {
            let position = self
                .recruitment_priority_position(i)
                .unwrap_or_else(|| self.select_position_for_youth_player(i, needed));
            let age = IntakeAgeDistribution::Backfill.sample();
            let player = PlayerGenerator::generate_with_context(
                country_id,
                date,
                position,
                people_names,
                &gen_ctx,
                age as i32,
                age as i32,
                Some(&mut intake_state),
            );
            self.players.add(player);
        }
    }

    pub(super) fn select_position_for_youth_player(
        &self,
        index: usize,
        total_players: usize,
    ) -> PlayerPositionType {
        if total_players >= 4 && index == 0 {
            PlayerPositionType::Goalkeeper
        } else {
            let position_roll = IntegerUtils::random(0, 100);

            match position_roll {
                0..=5 => PlayerPositionType::Goalkeeper,
                6..=20 => match IntegerUtils::random(0, 5) {
                    0 => PlayerPositionType::DefenderLeft,
                    1 => PlayerPositionType::DefenderRight,
                    2 | 3 => PlayerPositionType::DefenderCenter,
                    4 => PlayerPositionType::WingbackLeft,
                    _ => PlayerPositionType::WingbackRight,
                },
                21..=50 => match IntegerUtils::random(0, 3) {
                    0 => PlayerPositionType::DefensiveMidfielder,
                    1 => PlayerPositionType::MidfielderLeft,
                    2 => PlayerPositionType::MidfielderRight,
                    _ => PlayerPositionType::MidfielderCenter,
                },
                51..=75 => match IntegerUtils::random(0, 2) {
                    0 => PlayerPositionType::AttackingMidfielderLeft,
                    1 => PlayerPositionType::AttackingMidfielderRight,
                    _ => PlayerPositionType::AttackingMidfielderCenter,
                },
                _ => match IntegerUtils::random(0, 3) {
                    0 => PlayerPositionType::Striker,
                    1 => PlayerPositionType::ForwardLeft,
                    2 => PlayerPositionType::ForwardRight,
                    _ => PlayerPositionType::ForwardCenter,
                },
            }
        }
    }
}

/// Assigns positions for the candidate pool. The first `intake_count`
/// slots are filled against recruitment priorities; the rest fall back
/// to the generic position distribution. Owns the dispatch so the
/// per-index lookup stays inside one named type.
struct PositionAssigner<'a> {
    intake_count: usize,
    needs: &'a [PlayerFieldPositionGroup],
}

impl<'a> PositionAssigner<'a> {
    fn new(intake_count: usize, needs: &'a [PlayerFieldPositionGroup]) -> Self {
        PositionAssigner {
            intake_count,
            needs,
        }
    }

    fn position_for_pool_index(&self, pool_index: usize) -> PlayerPositionType {
        if pool_index < self.intake_count.min(self.needs.len()) {
            let group = self.needs[pool_index];
            Self::priority_position_for(group, pool_index)
        } else {
            Self::generic_position(pool_index, self.intake_count)
        }
    }

    fn priority_position_for(group: PlayerFieldPositionGroup, idx: usize) -> PlayerPositionType {
        match group {
            PlayerFieldPositionGroup::Goalkeeper => PlayerPositionType::Goalkeeper,
            PlayerFieldPositionGroup::Defender => match idx % 4 {
                0 => PlayerPositionType::DefenderCenter,
                1 => PlayerPositionType::DefenderLeft,
                2 => PlayerPositionType::DefenderRight,
                _ => PlayerPositionType::DefensiveMidfielder,
            },
            PlayerFieldPositionGroup::Midfielder => match idx % 5 {
                0 => PlayerPositionType::MidfielderCenter,
                1 => PlayerPositionType::MidfielderLeft,
                2 => PlayerPositionType::MidfielderRight,
                3 => PlayerPositionType::AttackingMidfielderCenter,
                _ => PlayerPositionType::DefensiveMidfielder,
            },
            PlayerFieldPositionGroup::Forward => match idx % 4 {
                0 => PlayerPositionType::Striker,
                1 => PlayerPositionType::ForwardLeft,
                2 => PlayerPositionType::ForwardRight,
                _ => PlayerPositionType::ForwardCenter,
            },
        }
    }

    fn generic_position(index: usize, total_players: usize) -> PlayerPositionType {
        if total_players >= 4 && index == 0 {
            return PlayerPositionType::Goalkeeper;
        }
        match IntegerUtils::random(0, 100) {
            0..=5 => PlayerPositionType::Goalkeeper,
            6..=20 => match IntegerUtils::random(0, 5) {
                0 => PlayerPositionType::DefenderLeft,
                1 => PlayerPositionType::DefenderRight,
                2 | 3 => PlayerPositionType::DefenderCenter,
                4 => PlayerPositionType::WingbackLeft,
                _ => PlayerPositionType::WingbackRight,
            },
            21..=50 => match IntegerUtils::random(0, 3) {
                0 => PlayerPositionType::DefensiveMidfielder,
                1 => PlayerPositionType::MidfielderLeft,
                2 => PlayerPositionType::MidfielderRight,
                _ => PlayerPositionType::MidfielderCenter,
            },
            51..=75 => match IntegerUtils::random(0, 2) {
                0 => PlayerPositionType::AttackingMidfielderLeft,
                1 => PlayerPositionType::AttackingMidfielderRight,
                _ => PlayerPositionType::AttackingMidfielderCenter,
            },
            _ => match IntegerUtils::random(0, 3) {
                0 => PlayerPositionType::Striker,
                1 => PlayerPositionType::ForwardLeft,
                2 => PlayerPositionType::ForwardRight,
                _ => PlayerPositionType::ForwardCenter,
            },
        }
    }
}

/// Intake-age distribution. `Annual` is used for the yearly intake;
/// `Backfill` is used by `ensure_minimum_players` to grow the squad
/// when it has dropped below the size band.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum IntakeAgeDistribution {
    /// 13 (20%), 14 (45%), 15 (25%), 16 (10%).
    Annual,
    /// 10-11 (15%), 12-13 (35%), 14-15 (35%), 16 (15%).
    Backfill,
}

impl IntakeAgeDistribution {
    pub fn sample(self) -> u8 {
        match self {
            IntakeAgeDistribution::Annual => match IntegerUtils::random(0, 99) {
                0..=19 => 13,
                20..=64 => 14,
                65..=89 => 15,
                _ => 16,
            },
            IntakeAgeDistribution::Backfill => match IntegerUtils::random(0, 99) {
                0..=14 => {
                    if IntegerUtils::random(0, 1) == 0 {
                        10
                    } else {
                        11
                    }
                }
                15..=49 => {
                    if IntegerUtils::random(0, 1) == 0 {
                        12
                    } else {
                        13
                    }
                }
                50..=84 => {
                    if IntegerUtils::random(0, 1) == 0 {
                        14
                    } else {
                        15
                    }
                }
                _ => 16,
            },
        }
    }
}

/// Candidate score driver.
///   0.45 * PA + 0.25 * CA + 0.10 * personality + 0.10 * position_need + 0.10 * hidden_random_fit
///
/// All axes are 0..100 — PA/CA are already in that range, personality
/// is summed from 0..20 traits, position need is binary on the
/// recruitment priorities list, and hidden_random_fit is the
/// recruiter's gut feel.
pub struct CandidateScorer<'a> {
    needs: &'a [PlayerFieldPositionGroup],
}

impl<'a> CandidateScorer<'a> {
    pub fn new(needs: &'a [PlayerFieldPositionGroup]) -> Self {
        CandidateScorer { needs }
    }

    pub fn score(&self, player: &crate::Player) -> f32 {
        let pa = player.player_attributes.potential_ability as f32;
        let ca = player.player_attributes.current_ability as f32;

        let personality_0_100 = ((player.attributes.professionalism * 0.40
            + player.attributes.ambition * 0.25
            + player.skills.mental.determination * 0.20
            + player.skills.mental.work_rate * 0.15)
            / 20.0
            * 100.0)
            .clamp(0.0, 100.0);

        let position_need = {
            let group = player.position().position_group();
            if self.needs.contains(&group) {
                100.0
            } else {
                40.0
            }
        };

        let hidden = rand::random::<f32>() * 100.0;

        0.45 * pa + 0.25 * ca + 0.10 * personality_0_100 + 0.10 * position_need + 0.10 * hidden
    }
}

/// Selection-time elite-class gate. Replaces the previous "dampen every
/// generated candidate" model: the candidate pool is generated cleanly,
/// then this gate decides whether each *signed* slot can take another
/// elite/world-class teenager. The first elite always gets through; each
/// subsequent elite has to pass a roll that shrinks exponentially with
/// how many already cleared the gate.
pub struct EliteSelectionGate {
    world_class_pa: u8,
    elite_pa: u8,
    world_class_seen: u32,
    elite_seen: u32,
}

impl EliteSelectionGate {
    pub fn new(world_class_pa: u8, elite_pa: u8) -> Self {
        EliteSelectionGate {
            world_class_pa,
            elite_pa,
            world_class_seen: 0,
            elite_seen: 0,
        }
    }

    /// Returns `true` if the candidate is signed. World-class candidates
    /// past the first need `roll < 0.35^world_class_seen`; elite past
    /// the first need `roll < 0.55^elite_seen`. Non-elite candidates
    /// are always accepted.
    pub fn accept(&mut self, pa: u8) -> bool {
        if pa >= self.world_class_pa {
            if self.world_class_seen == 0 {
                self.world_class_seen += 1;
                self.elite_seen += 1;
                true
            } else {
                let p = 0.35_f32.powi(self.world_class_seen as i32);
                if rand::random::<f32>() < p {
                    self.world_class_seen += 1;
                    self.elite_seen += 1;
                    true
                } else {
                    false
                }
            }
        } else if pa >= self.elite_pa {
            if self.elite_seen == 0 {
                self.elite_seen += 1;
                true
            } else {
                let p = 0.55_f32.powi(self.elite_seen as i32);
                if rand::random::<f32>() < p {
                    self.elite_seen += 1;
                    true
                } else {
                    false
                }
            }
        } else {
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cap_clamp_at_cap_yields_zero() {
        // Already at the cap: no new intake regardless of recruitment.
        assert_eq!(ClubAcademy::clamp_intake_to_cap(8, 64, 64), 0);
        assert_eq!(ClubAcademy::clamp_intake_to_cap(8, 65, 64), 0);
    }

    #[test]
    fn cap_clamp_headroom_one_caps_at_one() {
        // headroom == 1 → at most 1, ignore the min-3 floor.
        assert_eq!(ClubAcademy::clamp_intake_to_cap(8, 63, 64), 1);
        assert_eq!(ClubAcademy::clamp_intake_to_cap(3, 63, 64), 1);
    }

    #[test]
    fn cap_clamp_headroom_two_caps_at_two() {
        // headroom == 2 → at most 2, ignore the min-3 floor.
        assert_eq!(ClubAcademy::clamp_intake_to_cap(8, 62, 64), 2);
        assert_eq!(ClubAcademy::clamp_intake_to_cap(3, 62, 64), 2);
    }

    #[test]
    fn cap_clamp_headroom_three_applies_min_three() {
        // headroom >= 3 → at least 3 intake, never above headroom.
        assert_eq!(ClubAcademy::clamp_intake_to_cap(1, 61, 64), 3);
        assert_eq!(ClubAcademy::clamp_intake_to_cap(8, 61, 64), 3);
        assert_eq!(ClubAcademy::clamp_intake_to_cap(8, 30, 64), 8);
    }

    #[test]
    fn intake_count_clamps_to_three_minimum() {
        let academy = ClubAcademy::new(1);
        // Worst recruitment + worst club rep + worst pathway: still 3.
        let count = academy.calculate_annual_intake(0.0, 0.0, 0.0);
        assert!(count >= 3);
    }

    #[test]
    fn intake_count_scales_with_tier_recruitment_and_pathway() {
        let weak = ClubAcademy::new(2);
        let strong = ClubAcademy::new(20);
        let weak_count = weak.calculate_annual_intake(0.30, 0.40, 0.20);
        let strong_count = strong.calculate_annual_intake(0.85, 0.80, 0.90);
        assert!(strong_count > weak_count);
        assert!(strong_count <= weak.tuning.max_intake);
    }

    #[test]
    fn intake_age_distribution_is_diverse() {
        let mut counts = [0usize; 4];
        for _ in 0..2000 {
            let age = IntakeAgeDistribution::Annual.sample();
            let idx = match age {
                13 => 0,
                14 => 1,
                15 => 2,
                _ => 3,
            };
            counts[idx] += 1;
        }
        // Each bucket should hit at least some count — the test is
        // about diversity, not exact ratios.
        for c in counts {
            assert!(c > 0, "age bucket never seen: {:?}", counts);
        }
        // 14 should be the modal age.
        assert!(counts[1] > counts[0]);
        assert!(counts[1] > counts[2]);
        assert!(counts[1] > counts[3]);
    }

    #[test]
    fn pool_size_grows_with_recruitment_and_tier() {
        let weak = ClubAcademy::new(2);
        let strong = ClubAcademy::new(20);
        let weak_pool = weak.calculate_pool_size(8, 0.30);
        let strong_pool = strong.calculate_pool_size(8, 0.85);
        assert!(strong_pool > weak_pool);
        assert!(strong_pool <= 96);
    }

    #[test]
    fn intake_age_distribution_covers_13_through_16() {
        // Realism: a "year 14" intake must mix ages 13/14/15/16 rather
        // than minting every prospect at exactly age 14.
        let mut buckets: std::collections::HashMap<u8, usize> = Default::default();
        for _ in 0..1000 {
            let age = IntakeAgeDistribution::Annual.sample();
            *buckets.entry(age).or_insert(0) += 1;
        }
        for age in [13u8, 14, 15, 16] {
            assert!(
                buckets.get(&age).copied().unwrap_or(0) > 0,
                "age {} never appeared in 1000-sample annual intake",
                age
            );
        }
    }

    #[test]
    fn elite_gate_lets_first_world_class_through() {
        let mut gate = EliteSelectionGate::new(180, 160);
        assert!(
            gate.accept(185),
            "first world-class must always be accepted"
        );
    }

    #[test]
    fn elite_gate_dampens_successive_world_class() {
        // The first world-class passes; subsequent ones face
        // 0.35^seen. Across many runs we expect *at most* a small
        // fraction of pairs to both land.
        let mut second_passes = 0;
        for _ in 0..1000 {
            let mut gate = EliteSelectionGate::new(180, 160);
            assert!(gate.accept(190));
            if gate.accept(190) {
                second_passes += 1;
            }
        }
        // 0.35 ≈ 35%. Give a little margin for randomness.
        assert!(
            second_passes < 450,
            "second world-class passed {second_passes}/1000 — gate is not dampening"
        );
        assert!(
            second_passes > 200,
            "expected ~35% pass rate, saw {second_passes}/1000"
        );
    }

    #[test]
    fn elite_gate_dampens_successive_elite() {
        let mut second_passes = 0;
        for _ in 0..1000 {
            let mut gate = EliteSelectionGate::new(180, 160);
            assert!(gate.accept(165));
            if gate.accept(165) {
                second_passes += 1;
            }
        }
        // 0.55 ≈ 55%.
        assert!(
            second_passes < 670,
            "elite gate not dampening: {second_passes}/1000"
        );
        assert!(
            second_passes > 400,
            "elite gate over-dampening: {second_passes}/1000"
        );
    }

    #[test]
    fn elite_gate_passes_all_non_elites() {
        // Below the elite threshold the gate is always open.
        let mut gate = EliteSelectionGate::new(180, 160);
        for _ in 0..100 {
            assert!(gate.accept(140));
            assert!(gate.accept(120));
            assert!(gate.accept(90));
        }
    }

    #[test]
    fn backfill_age_distribution_reaches_foundation_phase() {
        // Backfill must include 10-12 year olds so the Foundation phase
        // isn't dead code.
        let mut foundation_seen = 0usize;
        for _ in 0..1000 {
            let age = IntakeAgeDistribution::Backfill.sample();
            if age <= 11 {
                foundation_seen += 1;
            }
        }
        assert!(
            foundation_seen >= 50,
            "expected at least ~5% foundation-age backfill, saw {}",
            foundation_seen
        );
    }
}
