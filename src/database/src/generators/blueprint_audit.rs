//! Audit of the blueprint projection: every player of the opening world and a
//! thousand academy intakes, read for weights held at the edge of their band
//! and for targets that do not score the player's potential.
//!
//! `cargo test -p database --release blueprint_audit -- --ignored --nocapture`

use crate::{DatabaseGenerator, DatabaseLoader};
use chrono::NaiveDate;
use core::club::player::development::SkillBlueprint;
use core::{PeopleNameGeneratorData, Player, PlayerGenerator, PlayerPositionType};

#[derive(Default)]
struct BlueprintAudit {
    players: usize,
    clamped: usize,
    light: usize,
    heavy: usize,
    weights: usize,
    off_potential: usize,
    worst: f32,
}

impl BlueprintAudit {
    fn read(&mut self, player: &Player) {
        self.players += 1;
        let blueprint = player.development.blueprint;
        if blueprint.clamped() {
            self.clamped += 1;
        }
        for w in blueprint.weights().into_iter().filter(|w| *w > 0.0) {
            self.weights += 1;
            if w <= 0.05 {
                self.light += 1;
            }
            if w >= 2.5 {
                self.heavy += 1;
            }
        }
        let position = player.position();
        let pa = player.player_attributes.potential_ability;
        let targets = SkillBlueprint::targets(&blueprint.weights(), position, pa);
        let miss = (SkillBlueprint::score(position, &targets) - pa as f32).abs();
        self.worst = self.worst.max(miss);
        if miss > 1.0 {
            self.off_potential += 1;
        }
    }

    fn print(&self, label: &str) {
        println!(
            "{label}: {} players, {} with a clamped weight ({:.2}%; of {} weights {} at the floor, {} at the cap), {} whose targets miss their potential by more than 1 (worst {:.2})",
            self.players,
            self.clamped,
            100.0 * self.clamped as f32 / self.players.max(1) as f32,
            self.weights,
            self.light,
            self.heavy,
            self.off_potential,
            self.worst
        );
    }
}

#[test]
#[ignore]
fn blueprint_audit() {
    let data = DatabaseGenerator::generate(&DatabaseLoader::load());
    let mut world = BlueprintAudit::default();
    for continent in &data.continents {
        for country in &continent.countries {
            for club in &country.clubs {
                for team in &club.teams.teams {
                    for player in &team.players.players {
                        world.read(player);
                    }
                }
            }
        }
    }
    for player in data.free_agents.iter() {
        world.read(player);
    }
    world.print("opening world");

    let names = PeopleNameGeneratorData {
        first_names: vec!["Audit".into()],
        last_names: vec!["Intake".into()],
        nicknames: vec![],
    };
    let today = NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();
    let positions = [
        PlayerPositionType::Goalkeeper,
        PlayerPositionType::DefenderCenter,
        PlayerPositionType::DefenderLeft,
        PlayerPositionType::DefensiveMidfielder,
        PlayerPositionType::MidfielderCenter,
        PlayerPositionType::MidfielderRight,
        PlayerPositionType::AttackingMidfielderCenter,
        PlayerPositionType::ForwardLeft,
        PlayerPositionType::Striker,
    ];
    let mut intakes = BlueprintAudit::default();
    for i in 0..1000 {
        let player = PlayerGenerator::generate(1, today, positions[i % 9], (i % 20) as u8, &names);
        intakes.read(&player);
    }
    intakes.print("academy intakes");
    assert_eq!(
        world.off_potential, 0,
        "a world player's blueprint misses his potential"
    );
    assert_eq!(
        intakes.off_potential, 0,
        "an intake's blueprint misses his potential"
    );
}
