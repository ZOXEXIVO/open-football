use crate::world::SimulatorData;
use crate::{NationalSelectionPolicy, NationalTeam};
use rayon::prelude::*;
use std::collections::HashSet;

impl SimulatorData {
    /// World-level national-team call-ups. Runs at the start of each
    /// break/tournament window, before any continent simulates, so
    /// candidate visibility spans the entire world — a Brazilian
    /// playing at a Spanish club is reachable from Brazil's selection
    /// pool without per-continent plumbing.
    pub fn process_world_national_team_callups(&mut self) {
        let date = self.date.date();
        let need_callups =
            NationalTeam::is_break_start(date) || NationalTeam::is_tournament_start(date);
        if !need_callups {
            return;
        }

        // Country IDs across the whole world — used to draw friendly
        // opponents from any nation, not just same-continent.
        let country_ids: Vec<(u32, String)> = self
            .continents
            .iter()
            .flat_map(|c| c.countries.iter())
            .map(|c| (c.id, c.name.clone()))
            .collect();

        // --- Senior selection -------------------------------------------------
        // Build a global senior candidate pool (main teams only) and run
        // the per-country call-up in parallel. Pre-distribute candidates
        // so each rayon worker owns its own slice — no shared HashMap.
        let mut candidates_by_country = NationalTeam::collect_all_candidates_by_country(
            self.continents.iter().flat_map(|c| c.countries.iter()),
            date,
        );
        let senior_work: Vec<_> = self
            .continents
            .iter_mut()
            .flat_map(|c| c.countries.iter_mut())
            .map(|country| {
                let candidates = candidates_by_country
                    .remove(&country.id)
                    .unwrap_or_default();
                (country, candidates)
            })
            .collect();
        senior_work
            .into_par_iter()
            .for_each(|(country, candidates)| {
                country.national_team.country_name = country.name.clone();
                country.national_team.reputation = country.reputation;
                let cid = country.id;
                country
                    .national_team
                    .call_up_squad(candidates, date, cid, &country_ids);
            });

        // --- U21 selection ----------------------------------------------------
        // Collect every player already taken by a senior squad in this
        // window — they're excluded from the U21 pool so the youth side
        // is a genuinely separate set of players, not a senior shadow.
        let senior_selected: HashSet<u32> = self
            .continents
            .iter()
            .flat_map(|c| c.countries.iter())
            .flat_map(|c| c.national_team.squad.iter().map(|sp| sp.player_id))
            .collect();

        let u21_policy = NationalSelectionPolicy::under21();
        let mut u21_candidates_by_country =
            NationalTeam::collect_all_candidates_by_country_with_policy(
                self.continents.iter().flat_map(|c| c.countries.iter()),
                date,
                &u21_policy,
            );
        for candidates in u21_candidates_by_country.values_mut() {
            candidates.retain(|c| !senior_selected.contains(&c.player_id));
        }
        let u21_work: Vec<_> = self
            .continents
            .iter_mut()
            .flat_map(|c| c.countries.iter_mut())
            .map(|country| {
                let candidates = u21_candidates_by_country
                    .remove(&country.id)
                    .unwrap_or_default();
                (country, candidates)
            })
            .collect();
        u21_work.into_par_iter().for_each(|(country, candidates)| {
            country.u21_national_team.country_name = country.name.clone();
            country.u21_national_team.reputation = country.reputation;
            let cid = country.id;
            country.u21_national_team.call_up_squad_with_policy(
                candidates,
                date,
                cid,
                &country_ids,
                &u21_policy,
            );
        });

        // Apply Int / IntU21 statuses across every club in every continent.
        // Senior first, then U21 — the U21 pass only toggles IntU21, so
        // the two never clash on the same player (the pools are disjoint).
        NationalTeam::apply_callup_statuses_across_world(&mut self.continents, date);
        NationalTeam::apply_u21_callup_statuses_across_world(&mut self.continents, date);
    }

    /// World-level Int release. Runs after all matches (continent
    /// matches + global tournament matches) so a tournament final
    /// landing on a release date is played with squad statuses still
    /// attached. Squad data itself is preserved for the squad UI; only
    /// the per-player Int flag is cleared.
    pub fn process_world_national_team_release(&mut self) {
        let date = self.date.date();
        let need_release =
            NationalTeam::is_break_end(date) || NationalTeam::is_tournament_end(date);
        if !need_release {
            return;
        }
        NationalTeam::release_callup_statuses_across_world(&mut self.continents);
        NationalTeam::release_u21_callup_statuses_across_world(&mut self.continents);
    }
}
