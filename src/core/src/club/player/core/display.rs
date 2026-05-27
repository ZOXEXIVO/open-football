use crate::club::player::contract::{
    AffordabilityInput, ContractStalemate, rejection_reason_token,
};
use crate::club::player::player::Player;
use crate::club::staff::Staff;
use crate::utils::DateUtils;
use chrono::NaiveDate;
use serde::Serialize;
use std::collections::BTreeMap;

// ─── Shared sub-structs ─────────────────────────────────────────────

#[derive(Serialize)]
struct PlayerSeasonStatsLlm {
    played: u16,
    played_subs: u16,
    goals: u16,
    assists: u16,
    yellow_cards: u8,
    average_rating: f32,
}

#[derive(Serialize)]
struct PlayerTrainingTrendLlm {
    technical: f32,
    mental: f32,
    physical: f32,
}

#[derive(Serialize)]
struct PlayerHistoryLlm {
    club_reputation: String,
    season: String,
    apps: u16,
    goals: u16,
    assists: u16,
    average_rating: f32,
}

#[derive(Serialize)]
struct PendingAskLlm {
    desired_salary: u32,
    desired_years: u8,
    /// Snake-case token derived from `RejectionReason` — `low_salary`,
    /// `short_contract`, `status_below_expectation`, etc. Stable string so
    /// the prompt can match on it without leaking enum names.
    rejection_reason: Option<&'static str>,
    /// True when wage budget headroom is known and the ask fits inside it.
    /// `None` when the caller did not supply budget context — in that case
    /// the prompt should treat affordability as unknown rather than
    /// inferring from the salary value alone.
    affordable: Option<bool>,
}

#[derive(Serialize)]
struct ContractStalemateLlm {
    /// Rolling 12-month count of renewal offers extended by the club.
    offers_12m: u32,
    /// Rolling 12-month count of renewal offers rejected by the player.
    rejections_12m: u32,
    /// Days since the most recent rejection. Missing when no rejection
    /// in the 12-month window.
    last_rejection_days_ago: Option<i64>,
    /// One of `none`, `emerging`, `severe`, `exhausted`. Mirrors
    /// `StalemateLevel` so the prompt can branch without re-deriving.
    level: &'static str,
    /// Outstanding ask from the player's side captured after the last
    /// rejection. None when there is no pending ask.
    pending_ask: Option<PendingAskLlm>,
}

// ─── as_llm() struct ────────────────────────────────────────────────

#[derive(Serialize)]
struct PlayerLlm {
    id: u32,
    age: u8,
    height: String,
    weight: String,
    positions: BTreeMap<String, String>,
    preferred_foot: String,
    physical_condition: String,
    /// Single human-readable summary derived from condition / load /
    /// jadedness / readiness — see ConditionLabel.
    condition_label: String,
    match_readiness: String,
    fitness: String,
    jadedness: String,
    morale: String,
    status: String,
    contract_months_left: Option<i64>,
    annual_wage: Option<String>,
    squad_status: Option<String>,
    reputation: PlayerReputationLlm,
    technical: PlayerTechnicalLlm,
    mental: PlayerMentalLlm,
    physical: PlayerPhysicalLlm,
    season_stats: Option<PlayerSeasonStatsLlm>,
    friendly_stats: Option<PlayerSeasonStatsLlm>,
    cup_stats: Option<PlayerSeasonStatsLlm>,
    training_trend: Option<PlayerTrainingTrendLlm>,
    club_history: Vec<PlayerHistoryLlm>,
    staff_opinion: String,
    /// Renewal-stalemate snapshot. Tells the AI whether listing for
    /// "failed renewal talks" is structurally justified — and how
    /// severely. Always present; `level == "none"` means no relevant
    /// rejection history.
    contract_stalemate: ContractStalemateLlm,
}

#[derive(Serialize)]
struct PlayerTechnicalLlm {
    corners: String,
    crossing: String,
    dribbling: String,
    finishing: String,
    first_touch: String,
    free_kicks: String,
    heading: String,
    long_shots: String,
    long_throws: String,
    marking: String,
    passing: String,
    penalty_taking: String,
    tackling: String,
    technique: String,
}

#[derive(Serialize)]
struct PlayerMentalLlm {
    aggression: String,
    anticipation: String,
    bravery: String,
    composure: String,
    concentration: String,
    decisions: String,
    determination: String,
    flair: String,
    leadership: String,
    off_the_ball: String,
    positioning: String,
    teamwork: String,
    vision: String,
    work_rate: String,
}

#[derive(Serialize)]
struct PlayerPhysicalLlm {
    acceleration: String,
    agility: String,
    balance: String,
    jumping: String,
    natural_fitness: String,
    pace: String,
    stamina: String,
    strength: String,
}

#[derive(Serialize)]
struct PlayerReputationLlm {
    current: String,
    home: String,
    world: String,
}

// ─── Helpers ────────────────────────────────────────────────────────

fn pct(val: f32) -> String {
    format!("{}%", (val / 20.0 * 100.0).round() as u32)
}

// ─── Implementation ─────────────────────────────────────────────────

impl Player {
    pub fn as_llm(&self, staff: &Staff, sim_date: NaiveDate) -> String {
        self.as_llm_with_affordability(staff, sim_date, None)
    }

    /// Build the same LLM payload as `as_llm`, but with affordability
    /// context so the `contract_stalemate.pending_ask.affordable` field
    /// can be filled in. Callers with wage-budget info (TransferList AI)
    /// should prefer this overload; callers without it can keep using
    /// `as_llm` and the prompt will treat affordability as unknown.
    pub fn as_llm_with_affordability(
        &self,
        staff: &Staff,
        sim_date: NaiveDate,
        affordability_headroom: Option<u32>,
    ) -> String {
        let age = DateUtils::age(self.birth_date, sim_date);
        let positions: BTreeMap<String, String> = self
            .positions
            .positions
            .iter()
            .filter(|p| p.level >= 5)
            .map(|p| {
                (
                    p.position.get_short_name().to_string(),
                    format!("{}%", p.level as u32 * 5),
                )
            })
            .collect();
        let morale = self.happiness.morale as u8;
        let status = self.status_string();

        let st = &self.statistics;
        let season_stats = if st.played > 0 || st.played_subs > 0 {
            Some(PlayerSeasonStatsLlm {
                played: st.played,
                played_subs: st.played_subs,
                goals: st.goals,
                assists: st.assists,
                yellow_cards: st.yellow_cards,
                average_rating: st.average_rating,
            })
        } else {
            None
        };

        let fs = &self.friendly_statistics;
        let friendly_stats = if fs.played > 0 || fs.played_subs > 0 {
            Some(PlayerSeasonStatsLlm {
                played: fs.played,
                played_subs: fs.played_subs,
                goals: fs.goals,
                assists: fs.assists,
                yellow_cards: fs.yellow_cards,
                average_rating: fs.average_rating,
            })
        } else {
            None
        };

        let cs = &self.cup_statistics;
        let cup_stats = if cs.played > 0 || cs.played_subs > 0 {
            Some(PlayerSeasonStatsLlm {
                played: cs.played,
                played_subs: cs.played_subs,
                goals: cs.goals,
                assists: cs.assists,
                yellow_cards: cs.yellow_cards,
                average_rating: cs.average_rating,
            })
        } else {
            None
        };

        let t = &self.skills.technical;
        let m = &self.skills.mental;
        let p = &self.skills.physical;

        let attr = &self.player_attributes;

        let (contract_months_left, annual_wage, squad_status) = match &self.contract {
            Some(c) => {
                let days = (c.expiration - sim_date).num_days();
                let months = if days <= 0 { Some(0) } else { Some(days / 30) };
                let wage = Some(format!("${}", c.salary));
                let status = Some(format!("{:?}", c.squad_status));
                (months, wage, status)
            }
            None => (None, None, None),
        };

        let current_salary = self.contract.as_ref().map(|c| c.salary).unwrap_or(0);
        let stalemate = ContractStalemate::assess(
            self,
            sim_date,
            AffordabilityInput {
                wage_budget_headroom: affordability_headroom,
                current_salary,
            },
        );
        let contract_stalemate = ContractStalemateLlm {
            offers_12m: stalemate.offers_12m,
            rejections_12m: stalemate.rejections_12m,
            last_rejection_days_ago: stalemate.last_rejection_days_ago,
            level: stalemate.level.as_key(),
            pending_ask: stalemate.pending_ask.as_ref().map(|ask| PendingAskLlm {
                desired_salary: ask.desired_salary,
                desired_years: ask.desired_years,
                rejection_reason: ask.rejection_reason.map(rejection_reason_token),
                affordable: stalemate.ask_affordable,
            }),
        };

        let player = PlayerLlm {
            id: self.id,
            age,
            height: format!("{:.2}m", attr.height as f32 / 100.0),
            weight: format!("{}kg", attr.weight),
            positions,
            preferred_foot: self.preferred_foot_str().to_string(),
            physical_condition: format!("{}%", attr.condition_percentage()),
            condition_label: self.condition_label().as_str().to_string(),
            match_readiness: pct(self.skills.physical.match_readiness),
            fitness: format!(
                "{}%",
                (attr.fitness as f32 / 10000.0 * 100.0).round() as u32
            ),
            jadedness: format!(
                "{}%",
                (attr.jadedness as f32 / 10000.0 * 100.0).round() as u32
            ),
            morale: format!("{}%", morale),
            status,
            contract_months_left,
            annual_wage,
            squad_status,
            reputation: PlayerReputationLlm {
                current: format!(
                    "{}%",
                    (self.player_attributes.current_reputation as f32 / 10000.0 * 100.0).round()
                        as u32
                ),
                home: format!(
                    "{}%",
                    (self.player_attributes.home_reputation as f32 / 10000.0 * 100.0).round()
                        as u32
                ),
                world: format!(
                    "{}%",
                    (self.player_attributes.world_reputation as f32 / 10000.0 * 100.0).round()
                        as u32
                ),
            },
            technical: PlayerTechnicalLlm {
                corners: pct(t.corners),
                crossing: pct(t.crossing),
                dribbling: pct(t.dribbling),
                finishing: pct(t.finishing),
                first_touch: pct(t.first_touch),
                free_kicks: pct(t.free_kicks),
                heading: pct(t.heading),
                long_shots: pct(t.long_shots),
                long_throws: pct(t.long_throws),
                marking: pct(t.marking),
                passing: pct(t.passing),
                penalty_taking: pct(t.penalty_taking),
                tackling: pct(t.tackling),
                technique: pct(t.technique),
            },
            mental: PlayerMentalLlm {
                aggression: pct(m.aggression),
                anticipation: pct(m.anticipation),
                bravery: pct(m.bravery),
                composure: pct(m.composure),
                concentration: pct(m.concentration),
                decisions: pct(m.decisions),
                determination: pct(m.determination),
                flair: pct(m.flair),
                leadership: pct(m.leadership),
                off_the_ball: pct(m.off_the_ball),
                positioning: pct(m.positioning),
                teamwork: pct(m.teamwork),
                vision: pct(m.vision),
                work_rate: pct(m.work_rate),
            },
            physical: PlayerPhysicalLlm {
                acceleration: pct(p.acceleration),
                agility: pct(p.agility),
                balance: pct(p.balance),
                jumping: pct(p.jumping),
                natural_fitness: pct(p.natural_fitness),
                pace: pct(p.pace),
                stamina: pct(p.stamina),
                strength: pct(p.strength),
            },
            season_stats,
            friendly_stats,
            cup_stats,
            training_trend: self.training_trend_llm(),
            club_history: self.club_history_vec(),
            staff_opinion: Self::staff_relationship_llm(staff, self.id),
            contract_stalemate,
        };

        serde_json::to_string(&player).unwrap()
    }

    fn status_string(&self) -> String {
        use crate::club::player::status::PlayerStatusType;

        let mut flags = Vec::new();
        if self.player_attributes.is_injured {
            flags.push(format!(
                "INJ {}d",
                self.player_attributes.injury_days_remaining
            ));
        }
        if self.player_attributes.is_banned {
            flags.push("BAN".to_string());
        }
        if self.player_attributes.is_in_recovery() {
            flags.push(format!(
                "REC {}d",
                self.player_attributes.recovery_days_remaining
            ));
        }

        let statuses = self.statuses.get();
        if statuses.contains(&PlayerStatusType::Lst) {
            flags.push("LST".to_string());
        }
        if statuses.contains(&PlayerStatusType::Loa) {
            flags.push("LOA".to_string());
        }
        if statuses.contains(&PlayerStatusType::Req) {
            flags.push("REQ".to_string());
        }
        if statuses.contains(&PlayerStatusType::Unh) {
            flags.push("UNH".to_string());
        }

        if flags.is_empty() {
            "OK".to_string()
        } else {
            flags.join(" ")
        }
    }

    fn training_trend_llm(&self) -> Option<PlayerTrainingTrendLlm> {
        let records = self.training_history.records();
        if records.is_empty() {
            return None;
        }

        let oldest = &records[0].skills;
        let current = &self.skills;

        Some(PlayerTrainingTrendLlm {
            technical: current.technical.average() - oldest.technical.average(),
            mental: current.mental.average() - oldest.mental.average(),
            physical: current.physical.average() - oldest.physical.average(),
        })
    }

    fn club_history_vec(&self) -> Vec<PlayerHistoryLlm> {
        // LLM context: feed the sample-size-regressed value so prompts
        // about a young player aren't anchored on a small-sample raw
        // average. Position-aware regression uses the player's current
        // position — historical entries don't track per-season role.
        let pos = self.position().position_group();
        self.statistics_history
            .items
            .iter()
            .rev()
            .take(3)
            .map(|h| PlayerHistoryLlm {
                club_reputation: format!("{}/10000", h.team_reputation),
                season: h.season.display.clone(),
                apps: h.statistics.played + h.statistics.played_subs,
                goals: h.statistics.goals,
                assists: h.statistics.assists,
                average_rating: h.statistics.average_rating_realistic(pos),
            })
            .collect()
    }

    fn staff_relationship_llm(staff: &Staff, player_id: u32) -> String {
        if let Some(rel) = staff.relations.get_player(player_id) {
            let opinion = if rel.level > 50.0 {
                "favored"
            } else if rel.level > 20.0 {
                "liked"
            } else if rel.level > -20.0 {
                "neutral"
            } else if rel.level > -50.0 {
                "disliked"
            } else {
                "conflict"
            };
            format!("{} trust:{}", opinion, rel.trust as u8)
        } else {
            "unknown".to_string()
        }
    }
}
