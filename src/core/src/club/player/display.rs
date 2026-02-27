use crate::club::player::player::Player;
use crate::club::staff::staff::Staff;
use crate::utils::DateUtils;
use serde::Serialize;

// ─── Shared sub-structs ─────────────────────────────────────────────

#[derive(Serialize)]
struct PlayerSeasonStatsLlm {
    #[serde(rename = "p")]
    played: u16,
    #[serde(rename = "ps")]
    played_subs: u16,
    #[serde(rename = "g")]
    goals: u16,
    #[serde(rename = "a")]
    assists: u16,
    #[serde(rename = "yc")]
    yellow_cards: u8,
    #[serde(rename = "ar")]
    average_rating: f32,
}

#[derive(Serialize)]
struct PlayerTrainingTrendLlm {
    #[serde(rename = "tec")]
    technical: f32,
    #[serde(rename = "men")]
    mental: f32,
    #[serde(rename = "phy")]
    physical: f32,
}

#[derive(Serialize)]
struct PlayerHistoryLlm {
    #[serde(rename = "rep")]
    club_reputation: String,
    #[serde(rename = "s")]
    season: String,
    #[serde(rename = "ap")]
    apps: u16,
    #[serde(rename = "g")]
    goals: u16,
    #[serde(rename = "a")]
    assists: u16,
    #[serde(rename = "ar")]
    average_rating: f32,
}

// ─── as_llm() struct ────────────────────────────────────────────────

#[derive(Serialize)]
struct PlayerSkillsLlm {
    #[serde(rename = "tec")]
    technical: f32,
    #[serde(rename = "men")]
    mental: f32,
    #[serde(rename = "phy")]
    physical: f32,
}

#[derive(Serialize)]
struct PlayerLlm {
    id: u32,
    #[serde(rename = "age")]
    age: u8,
    #[serde(rename = "pos")]
    positions: String,
    #[serde(rename = "ft")]
    preferred_foot: String,
    #[serde(rename = "sk")]
    skills: PlayerSkillsLlm,
    #[serde(rename = "cond")]
    condition_pct: u32,
    #[serde(rename = "mor")]
    morale_pct: u8,
    #[serde(rename = "st")]
    status: String,
    #[serde(rename = "ss")]
    season_stats: Option<PlayerSeasonStatsLlm>,
    #[serde(rename = "fs")]
    friendly_stats: Option<PlayerSeasonStatsLlm>,
    #[serde(rename = "tt")]
    training_trend: Option<PlayerTrainingTrendLlm>,
    #[serde(rename = "ch")]
    club_history: Vec<PlayerHistoryLlm>,
    #[serde(rename = "op")]
    staff_opinion: String,
}

const PLAYER_LEGEND: &str = r#"{"id":"Player ID","age":"Age","pos":"Positions","ft":"Foot(L/R/B)","sk":"Skills avg 0-20:tec=technical,men=mental,phy=physical","cond":"Condition 0-100%","mor":"Morale 0-100","st":"OK|INJ Nd|BAN|REC Nd|LST|LOA|REQ|UNH","ss":"Season:p=played,ps=subs,g=goals,a=assists,yc=yellows,ar=avg_rating;null if none","fs":"Friendly matches:p=played,ps=subs,g=goals,a=assists,yc=yellows,ar=avg_rating;null if none","tt":"Training trend:tec,men,phy deltas;null if none","ch":"History(last 3):rep=reputation/10000,s=season,ap=apps,g=goals,a=assists,ar=avg_rating","op":"Staff opinion:favored/liked/neutral/disliked/conflict + trust"}"#;

// ─── as_internal_llm() struct ───────────────────────────────────────

#[derive(Serialize)]
struct PlayerInternalLlm {
    id: u32,
    #[serde(rename = "age")]
    age: u8,
    #[serde(rename = "pos")]
    positions: String,
    #[serde(rename = "ft")]
    preferred_foot: String,
    #[serde(rename = "cond")]
    condition_pct: u32,
    #[serde(rename = "mor")]
    morale_pct: u8,
    #[serde(rename = "st")]
    status: String,
    #[serde(rename = "tec")]
    technical: PlayerTechnicalLlm,
    #[serde(rename = "men")]
    mental: PlayerMentalLlm,
    #[serde(rename = "phy")]
    physical: PlayerPhysicalLlm,
    #[serde(rename = "ss")]
    season_stats: Option<PlayerSeasonStatsLlm>,
    #[serde(rename = "tt")]
    training_trend: Option<PlayerTrainingTrendLlm>,
    #[serde(rename = "ch")]
    club_history: Vec<PlayerHistoryLlm>,
}

#[derive(Serialize)]
struct PlayerTechnicalLlm {
    #[serde(rename = "cor")]
    corners: f32,
    #[serde(rename = "cro")]
    crossing: f32,
    #[serde(rename = "dri")]
    dribbling: f32,
    #[serde(rename = "fin")]
    finishing: f32,
    #[serde(rename = "fto")]
    first_touch: f32,
    #[serde(rename = "fk")]
    free_kicks: f32,
    #[serde(rename = "hea")]
    heading: f32,
    #[serde(rename = "lsh")]
    long_shots: f32,
    #[serde(rename = "lth")]
    long_throws: f32,
    #[serde(rename = "mar")]
    marking: f32,
    #[serde(rename = "pas")]
    passing: f32,
    #[serde(rename = "pen")]
    penalty_taking: f32,
    #[serde(rename = "tac")]
    tackling: f32,
    #[serde(rename = "tec")]
    technique: f32,
}

#[derive(Serialize)]
struct PlayerMentalLlm {
    #[serde(rename = "agg")]
    aggression: f32,
    #[serde(rename = "ant")]
    anticipation: f32,
    #[serde(rename = "bra")]
    bravery: f32,
    #[serde(rename = "cmp")]
    composure: f32,
    #[serde(rename = "cnc")]
    concentration: f32,
    #[serde(rename = "dec")]
    decisions: f32,
    #[serde(rename = "det")]
    determination: f32,
    #[serde(rename = "fla")]
    flair: f32,
    #[serde(rename = "lea")]
    leadership: f32,
    #[serde(rename = "otb")]
    off_the_ball: f32,
    #[serde(rename = "pos")]
    positioning: f32,
    #[serde(rename = "tea")]
    teamwork: f32,
    #[serde(rename = "vis")]
    vision: f32,
    #[serde(rename = "wor")]
    work_rate: f32,
}

#[derive(Serialize)]
struct PlayerPhysicalLlm {
    #[serde(rename = "acc")]
    acceleration: f32,
    #[serde(rename = "agi")]
    agility: f32,
    #[serde(rename = "bal")]
    balance: f32,
    #[serde(rename = "jum")]
    jumping: f32,
    #[serde(rename = "nfi")]
    natural_fitness: f32,
    #[serde(rename = "pac")]
    pace: f32,
    #[serde(rename = "sta")]
    stamina: f32,
    #[serde(rename = "str")]
    strength: f32,
    #[serde(rename = "mr")]
    match_readiness: f32,
}

const PLAYER_INTERNAL_LEGEND: &str = r#"{"id":"Player ID","n":"Name","age":"Age","pos":"Positions","ft":"Foot(L/R/B)","cond":"Condition 0-100%","mor":"Morale 0-100","st":"OK|INJ Nd|BAN|REC Nd","tec":"Technical(0-20):cor=corners,cro=crossing,dri=dribbling,fin=finishing,fto=first_touch,fk=free_kicks,hea=heading,lsh=long_shots,lth=long_throws,mar=marking,pas=passing,pen=penalty_taking,tac=tackling,tec=technique","men":"Mental(0-20):agg=aggression,ant=anticipation,bra=bravery,cmp=composure,cnc=concentration,dec=decisions,det=determination,fla=flair,lea=leadership,otb=off_the_ball,pos=positioning,tea=teamwork,vis=vision,wor=work_rate","phy":"Physical(0-20):acc=acceleration,agi=agility,bal=balance,jum=jumping,nfi=natural_fitness,pac=pace,sta=stamina,str=strength,mr=match_readiness","ss":"Season:p=played,ps=subs,g=goals,a=assists,yc=yellows,ar=avg_rating;null if none","tt":"Training trend:tec,men,phy deltas;null if none","ch":"History(last 3):rep=reputation/10000,s=season,ap=apps,g=goals,a=assists,ar=avg_rating"}"#;

// ─── Implementation ─────────────────────────────────────────────────

impl Player {
    pub fn llm_legend() -> &'static str {
        PLAYER_LEGEND
    }

    pub fn internal_llm_legend() -> &'static str {
        PLAYER_INTERNAL_LEGEND
    }

    pub fn as_llm(&self, staff: &Staff) -> String {
        let now = chrono::Local::now().date_naive();
        let age = DateUtils::age(self.birth_date, now);
        let pos = self.positions.display_positions().join(",");
        let cond = self.player_attributes.condition_percentage();
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

        let player = PlayerLlm {
            id: self.id,
            age,
            positions: pos,
            preferred_foot: self.preferred_foot_str().to_string(),
            skills: PlayerSkillsLlm {
                technical: (self.skills.technical.average() * 10.0).round() / 10.0,
                mental: (self.skills.mental.average() * 10.0).round() / 10.0,
                physical: (self.skills.physical.average() * 10.0).round() / 10.0,
            },
            condition_pct: cond,
            morale_pct: morale,
            status,
            season_stats,
            friendly_stats,
            training_trend: self.training_trend_llm(),
            club_history: self.club_history_vec(),
            staff_opinion: Self::staff_relationship_llm(staff, self.id),
        };

        serde_json::to_string(&player).unwrap()
    }

    pub fn as_internal_llm(&self) -> String {
        let now = chrono::Local::now().date_naive();
        let age = DateUtils::age(self.birth_date, now);
        let pos = self.positions.display_positions().join(",");
        let cond = self.player_attributes.condition_percentage();
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

        let t = &self.skills.technical;
        let m = &self.skills.mental;
        let p = &self.skills.physical;

        let player = PlayerInternalLlm {
            id: self.id,
            age,
            positions: pos,
            preferred_foot: self.preferred_foot_str().to_string(),
            condition_pct: cond,
            morale_pct: morale,
            status,
            technical: PlayerTechnicalLlm {
                corners: t.corners,
                crossing: t.crossing,
                dribbling: t.dribbling,
                finishing: t.finishing,
                first_touch: t.first_touch,
                free_kicks: t.free_kicks,
                heading: t.heading,
                long_shots: t.long_shots,
                long_throws: t.long_throws,
                marking: t.marking,
                passing: t.passing,
                penalty_taking: t.penalty_taking,
                tackling: t.tackling,
                technique: t.technique,
            },
            mental: PlayerMentalLlm {
                aggression: m.aggression,
                anticipation: m.anticipation,
                bravery: m.bravery,
                composure: m.composure,
                concentration: m.concentration,
                decisions: m.decisions,
                determination: m.determination,
                flair: m.flair,
                leadership: m.leadership,
                off_the_ball: m.off_the_ball,
                positioning: m.positioning,
                teamwork: m.teamwork,
                vision: m.vision,
                work_rate: m.work_rate,
            },
            physical: PlayerPhysicalLlm {
                acceleration: p.acceleration,
                agility: p.agility,
                balance: p.balance,
                jumping: p.jumping,
                natural_fitness: p.natural_fitness,
                pace: p.pace,
                stamina: p.stamina,
                strength: p.strength,
                match_readiness: p.match_readiness,
            },
            season_stats,
            training_trend: self.training_trend_llm(),
            club_history: self.club_history_vec(),
        };

        serde_json::to_string(&player).unwrap()
    }

    fn status_string(&self) -> String {
        use crate::club::player::status::PlayerStatusType;

        let mut flags = Vec::new();
        if self.player_attributes.is_injured {
            flags.push(format!("INJ {}d", self.player_attributes.injury_days_remaining));
        }
        if self.player_attributes.is_banned {
            flags.push("BAN".to_string());
        }
        if self.player_attributes.is_in_recovery() {
            flags.push(format!("REC {}d", self.player_attributes.recovery_days_remaining));
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

        if flags.is_empty() { "OK".to_string() } else { flags.join(" ") }
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
        self.statistics_history.items.iter().rev().take(3).map(|h| {
            PlayerHistoryLlm {
                club_reputation: format!("{}/10000", h.team_reputation),
                season: h.season.display.clone(),
                apps: h.statistics.played + h.statistics.played_subs,
                goals: h.statistics.goals,
                assists: h.statistics.assists,
                average_rating: h.statistics.average_rating,
            }
        }).collect()
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
