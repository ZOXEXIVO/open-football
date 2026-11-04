//! A per-day digest of the world, to tell an optimisation from a behaviour
//! change: two runs of the same seed at one worker per pool must print the
//! same lines, and a refactor that moves a line changed what the world does.
//!
//! Walks `Vec` order only, hashes floats by their bits, and sorts anything
//! that comes out of a hash map before hashing it. Each subsystem gets its
//! own sub-digest so a mismatch says where to look.

use core::{Player, SimulationResult, SimulatorData};
use std::fmt::Write as _;
use std::hash::{DefaultHasher, Hash, Hasher};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorldFingerprint {
    pub matches: u64,
    pub players: u64,
    pub clubs: u64,
    pub staff: u64,
    pub markets: u64,
    pub free_agents: u64,
    pub match_store: u64,
}

impl WorldFingerprint {
    pub fn take(data: &SimulatorData, result: &SimulationResult) -> Self {
        WorldFingerprint {
            matches: Self::matches(result),
            players: Self::players(data),
            clubs: Self::clubs(data),
            staff: Self::staff(data),
            markets: Self::markets(data),
            free_agents: Self::free_agents(data),
            match_store: Self::match_store(data),
        }
    }

    pub fn digest(&self) -> u64 {
        let mut h = DefaultHasher::new();
        self.parts().iter().for_each(|(_, v)| v.hash(&mut h));
        h.finish()
    }

    pub fn line(&self) -> String {
        let mut out = format!("{:016x}", self.digest());
        for (name, value) in self.parts() {
            let _ = write!(out, "  {name} {:08x}", value >> 32);
        }
        out
    }

    /// The sub-digests that disagree between two fingerprints.
    pub fn differences(&self, other: &WorldFingerprint) -> Vec<&'static str> {
        self.parts()
            .iter()
            .zip(other.parts().iter())
            .filter(|((_, a), (_, b))| a != b)
            .map(|((name, _), _)| *name)
            .collect()
    }

    fn parts(&self) -> [(&'static str, u64); 7] {
        [
            ("matches", self.matches),
            ("players", self.players),
            ("clubs", self.clubs),
            ("staff", self.staff),
            ("markets", self.markets),
            ("free", self.free_agents),
            ("store", self.match_store),
        ]
    }

    fn matches(result: &SimulationResult) -> u64 {
        let mut h = DefaultHasher::new();
        let mut text = String::new();
        for m in &result.match_results {
            m.id.hash(&mut h);
            m.league_id.hash(&mut h);
            m.home_team_id.hash(&mut h);
            m.away_team_id.hash(&mut h);
            m.score.home_team.get().hash(&mut h);
            m.score.away_team.get().hash(&mut h);
            m.friendly.hash(&mut h);
            let Some(details) = &m.details else {
                continue;
            };
            let mut stats: Vec<_> = details.player_stats.iter().collect();
            stats.sort_by_key(|(id, _)| **id);
            for (id, s) in stats {
                id.hash(&mut h);
                text.clear();
                let _ = write!(text, "{s:?}");
                text.hash(&mut h);
            }
            details.substitutions.len().hash(&mut h);
        }
        h.finish()
    }

    /// One line per country — its club order, its players, its staff — so
    /// a mismatch can be pinned to a country, and to whether the world was
    /// built in a different order or with different contents.
    pub fn country_lines(data: &SimulatorData) -> Vec<String> {
        let mut text = String::new();
        data.continents
            .iter()
            .flat_map(|c| &c.countries)
            .map(|country| {
                let mut order = DefaultHasher::new();
                let mut players = DefaultHasher::new();
                let mut staff = DefaultHasher::new();
                for club in &country.clubs {
                    club.id.hash(&mut order);
                    for team in club.teams.iter() {
                        team.id.hash(&mut order);
                        for p in team.players.iter() {
                            p.id.hash(&mut order);
                            Self::player(&mut players, &mut text, p);
                        }
                        for s in &team.staffs.staffs {
                            s.id.hash(&mut staff);
                        }
                    }
                }
                format!(
                    "country {:>4} {:<24} order {:016x} players {:016x} staff {:016x}",
                    country.id,
                    country.name,
                    order.finish(),
                    players.finish(),
                    staff.finish()
                )
            })
            .collect()
    }

    /// One line per player of a country, each part of the player hashed on
    /// its own, so a mismatch between two runs names the field it starts in.
    pub fn player_lines(data: &SimulatorData, country_id: u32) -> Vec<String> {
        let part = |f: &dyn Fn(&mut DefaultHasher)| {
            let mut h = DefaultHasher::new();
            f(&mut h);
            h.finish() >> 40
        };
        data.continents
            .iter()
            .flat_map(|c| &c.countries)
            .filter(|c| c.id == country_id)
            .flat_map(|c| &c.clubs)
            .flat_map(|c| c.teams.iter())
            .flat_map(|t| t.players.iter())
            .map(|p| {
                let a = &p.player_attributes;
                let mut relations: Vec<(u32, u32)> = p
                    .relations
                    .player_relations_iter()
                    .map(|(id, r)| (*id, r.level.to_bits()))
                    .collect();
                relations.sort_unstable();
                format!(
                    "{:>8} listed {:?} loan {:?}/{:?} onloan {} banned {} injury {} intl {}/{} cond {:06x} attr {:06x} stats {:06x} morale {:06x} events {:>3} rel {:06x} skills {:06x} contract {:06x}",
                    p.id,
                    p.contract.as_ref().map(|c| c.is_transfer_listed),
                    p.contract.as_ref().and_then(|c| c.loan_from_club_id),
                    p.contract.as_ref().and_then(|c| c.loan_to_club_id),
                    p.contract_loan.is_some(),
                    a.is_banned,
                    a.injury_days_remaining,
                    a.international_apps,
                    a.international_goals,
                    part(&|h| (a.condition, a.fitness, a.jadedness).hash(h)),
                    part(&|h| {
                        (a.current_ability, a.potential_ability, a.value, a.is_injured).hash(h);
                        (a.current_reputation, a.home_reputation, a.world_reputation).hash(h)
                    }),
                    part(&|h| {
                        let s = &p.statistics;
                        (s.played, s.played_subs, s.goals, s.assists, s.average_rating.to_bits()).hash(h)
                    }),
                    part(&|h| p.happiness.morale.to_bits().hash(h)),
                    p.happiness.recent_events.len(),
                    part(&|h| relations.hash(h)),
                    part(&|h| format!("{:?}", p.skills).hash(h)),
                    part(&|h| p.contract.as_ref().map(|c| (c.salary, c.expiration)).hash(h)),
                )
            })
            .collect()
    }

    fn players(data: &SimulatorData) -> u64 {
        let mut h = DefaultHasher::new();
        let mut text = String::new();
        for country in data.continents.iter().flat_map(|c| &c.countries) {
            for club in &country.clubs {
                for team in club.teams.iter() {
                    team.id.hash(&mut h);
                    for p in team.players.iter() {
                        Self::player(&mut h, &mut text, p);
                    }
                }
            }
            country.retired_players.len().hash(&mut h);
            for p in &country.retired_players {
                p.id.hash(&mut h);
            }
        }
        h.finish()
    }

    fn player(h: &mut DefaultHasher, text: &mut String, p: &Player) {
        p.id.hash(h);
        if let Some(c) = &p.contract {
            c.salary.hash(h);
            c.expiration.hash(h);
            c.loan_from_club_id.hash(h);
            c.loan_to_club_id.hash(h);
            c.is_transfer_listed.hash(h);
        }
        p.contract_loan.is_some().hash(h);
        let a = &p.player_attributes;
        a.condition.hash(h);
        a.fitness.hash(h);
        a.jadedness.hash(h);
        a.is_injured.hash(h);
        a.is_banned.hash(h);
        a.injury_days_remaining.hash(h);
        a.current_ability.hash(h);
        a.potential_ability.hash(h);
        a.current_reputation.hash(h);
        a.home_reputation.hash(h);
        a.world_reputation.hash(h);
        a.value.hash(h);
        a.international_apps.hash(h);
        a.international_goals.hash(h);
        let s = &p.statistics;
        s.played.hash(h);
        s.played_subs.hash(h);
        s.goals.hash(h);
        s.assists.hash(h);
        s.average_rating.to_bits().hash(h);
        p.happiness.morale.to_bits().hash(h);
        p.happiness.recent_events.len().hash(h);
        let mut relations: Vec<(u32, u32)> = p
            .relations
            .player_relations_iter()
            .map(|(id, r)| (*id, r.level.to_bits()))
            .collect();
        relations.sort_unstable();
        relations.hash(h);
        text.clear();
        let _ = write!(text, "{:?}", p.skills);
        text.hash(h);
    }

    fn clubs(data: &SimulatorData) -> u64 {
        let mut h = DefaultHasher::new();
        for club in data
            .continents
            .iter()
            .flat_map(|c| &c.countries)
            .flat_map(|c| &c.clubs)
        {
            club.id.hash(&mut h);
            club.finance.balance.balance.hash(&mut h);
            club.finance
                .transfer_budget
                .as_ref()
                .map(|b| b.amount.to_bits())
                .hash(&mut h);
            for team in club.teams.iter() {
                team.id.hash(&mut h);
                team.league_id.hash(&mut h);
                team.players.players.len().hash(&mut h);
            }
        }
        h.finish()
    }

    fn staff(data: &SimulatorData) -> u64 {
        let mut h = DefaultHasher::new();
        for team in data
            .continents
            .iter()
            .flat_map(|c| &c.countries)
            .flat_map(|c| &c.clubs)
            .flat_map(|c| c.teams.iter())
        {
            team.id.hash(&mut h);
            for s in &team.staffs.staffs {
                s.id.hash(&mut h);
                s.contract.is_some().hash(&mut h);
            }
        }
        for s in &data.free_agent_staff {
            s.id.hash(&mut h);
        }
        h.finish()
    }

    fn markets(data: &SimulatorData) -> u64 {
        let mut h = DefaultHasher::new();
        for country in data.continents.iter().flat_map(|c| &c.countries) {
            let market = &country.transfer_market;
            country.id.hash(&mut h);
            market.listings.len().hash(&mut h);
            market.negotiations.len().hash(&mut h);
            market.transfer_history.len().hash(&mut h);
            if let Some(last) = market.transfer_history.last() {
                last.player_id.hash(&mut h);
                last.from_club_id.hash(&mut h);
                last.to_club_id.hash(&mut h);
                last.transfer_date.hash(&mut h);
                last.fee.amount.to_bits().hash(&mut h);
            }
        }
        h.finish()
    }

    fn free_agents(data: &SimulatorData) -> u64 {
        let mut h = DefaultHasher::new();
        data.free_agents.len().hash(&mut h);
        for p in &data.free_agents {
            p.id.hash(&mut h);
            p.player_attributes.current_ability.hash(&mut h);
            p.is_retired().hash(&mut h);
        }
        h.finish()
    }

    fn match_store(data: &SimulatorData) -> u64 {
        let mut h = DefaultHasher::new();
        data.match_store.len().hash(&mut h);
        h.finish()
    }
}
