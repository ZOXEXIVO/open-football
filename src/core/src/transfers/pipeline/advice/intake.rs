//! What a club does with the names its staff brought in.
//!
//! The pass that follows [`super::scan`]: every recommendation filed in the last
//! week is re-checked against the world as it is now — the player may have moved,
//! the seller's balance may have turned, the recruitment meeting may have thrown
//! him out — and then either joins the shortlist of a brief the club already has
//! open, seeds a shortlist for a brief that had none, or opens a brief of its own.
//!
//! Split from one 394-line body along the two passes its comments already named.
//! The scan cannot write (it walks `&country.clubs` while deciding), so it stages
//! actions and [`IntakeCommit`] does every write.

use crate::transfers::squad::SquadReviewPass;
use chrono::Duration;
use chrono::NaiveDate;

use crate::transfers::MarketMap;
use crate::transfers::gate::build::Suitor;
use crate::transfers::gate::{MoveTerms, PursuitAction, TransferMovePlausibility};
use crate::transfers::pipeline::{
    ClubTransferPlan, FileReading, KnownPlayerMemory, ShortlistCandidate, ShortlistCandidateStatus,
    StaffRecommendation, TransferNeedPriority, TransferNeedReason, TransferRequest,
    TransferRequestStatus,
};
use crate::transfers::view::player::CountryPlayerLookup;
use crate::transfers::view::world::WorldPool;
use crate::{Club, Country, PositionCoverage};

use super::BuyerNeedPicture;

struct RecommendationProcessAction {
    club_id: u32,
    /// The gate's reading the action was decided on.
    reading: FileReading,
    kind: RecommendationProcessKind,
}

enum RecommendationProcessKind {
    AddToShortlist {
        shortlist_request_id: u32,
        candidate: ShortlistCandidate,
    },
    /// The recommendation fits an OPEN request that has no
    /// shortlist yet — open one for it and put him on it.
    ///
    /// Without this branch the recommendation fell down a hole. A
    /// matching request with no shortlist satisfied the `if let
    /// Some(req)` arm, found nothing to add to, and returned — while
    /// the create-a-request arm below is an `else`, so it never ran
    /// either. In other words the club's own recruitment department
    /// silently discarded a name precisely when its coach had ALSO
    /// asked for that position: the more a club wanted a player
    /// there, the less likely it was to act on the one it had
    /// found. Only a shortlist that a scouting assignment happened
    /// to have produced could ever receive him.
    SeedShortlist {
        request_id: u32,
        /// The REQUEST's allocation, not the candidate's fee — the
        /// shortlist's budget is what the club set aside for the
        /// position, and every downstream affordability check
        /// reads it.
        allocation: f64,
        candidate: ShortlistCandidate,
    },
    CreateRequest {
        request: TransferRequest,
        candidate: ShortlistCandidate,
    },
}

/// One club's intake. Read-only: the scan walks every club in the country
/// while it decides, so it cannot hold a `&mut` to any of them.
struct IntakeScan<'a> {
    country: &'a Country,
    club: &'a Club,
    plan: &'a ClubTransferPlan,
    lookup: &'a CountryPlayerLookup,
    suitor: Suitor<'a>,
    world: WorldPool<'a>,
    market_map: &'a MarketMap,
    date: NaiveDate,
}

impl IntakeScan<'_> {
    /// Every recommendation filed in the last seven days, gated and routed.
    fn stage(&self, actions: &mut Vec<RecommendationProcessAction>) {
        let country = self.country;
        let plan = self.plan;
        let date = self.date;
        let player_lookup = self.lookup;
        let seven_days_ago = date - Duration::days(7);

        let recent_recs: Vec<&StaffRecommendation> = plan
            .staff_recommendations
            .iter()
            .filter(|r| r.date_recommended >= seven_days_ago)
            .collect();

        for rec in &recent_recs {
            // Meeting rejections blocklist the player for 6 months —
            // the consumption chokepoint gate covers every
            // recommendation source (scout network, listed-star sweep,
            // bargain hunts) at once.
            if plan.is_rejected(rec.player_id, date) {
                continue;
            }
            // Determine the player's position group — and every group he
            // can actually play in, so a recommendation can answer the
            // brief the club is genuinely short in rather than the one his
            // primary label happens to name.
            let memory = plan.known_player(rec.player_id);
            let (player_pos_group, player_coverage) =
                if let Some(player) = player_lookup.find_player(country, rec.player_id) {
                    (
                        player.position().position_group(),
                        PositionCoverage::of(&player.positions),
                    )
                } else if let Some(memory) = memory {
                    (
                        memory.position_group,
                        PositionCoverage::single(memory.position),
                    )
                } else {
                    continue;
                };

            // Re-check plausibility before promoting a stale
            // recommendation. Player status and seller balance may
            // have shifted since the recommendation was filed.
            // Read again before a stale recommendation becomes a request:
            // his status and his club's position may have shifted since it
            // was filed, and a man who cannot be read is not pursued.
            let Some(summary) = player_lookup
                .find_summary(country, rec.player_id, date)
                .or_else(|| self.world.find(rec.player_id))
            else {
                continue;
            };
            let assessment = TransferMovePlausibility::read(
                &self.suitor,
                summary,
                MoveTerms::purchase(true),
                self.market_map,
            );
            if !assessment.reaches(PursuitAction::Shortlist.needs()) {
                continue;
            }
            let reading = FileReading::of(&assessment, date);
            let player_age = Some(summary.age);

            // Check if an existing unfulfilled request covers the same position group.
            // Emergency free-agent depth requests don't count — attaching a paid
            // recommendation to their shortlist would route a zero-budget request
            // into the paid negotiation path.
            // The request's age band must also fit (min strict, max + 3,
            // matching the loan-market relaxation) — otherwise a
            // recommended veteran lands on a DevelopmentSigning
            // shortlist and the deal executes under a "young prospect"
            // motive. A player who fits no open request falls through
            // to the create-request branch below, whose
            // StaffRecommendation band (18-32) is the honest label.
            //
            // Matched on what he can PLAY, and the most urgent such brief
            // wins. Keying it on his primary label alone sent every
            // versatile attacker onto whichever request his record's first
            // entry named — so a club with an unfilled centre-forward brief
            // and an open midfield tip kept adding forwards to the midfield
            // list, and bought a sixth attacking midfielder while the shirt
            // it was actually short in went unaddressed.
            let matching_request = plan
                .transfer_requests
                .iter()
                .filter(|r| {
                    player_coverage.covers_group(r.position.position_group())
                        && r.status != TransferRequestStatus::Fulfilled
                        && r.status != TransferRequestStatus::Abandoned
                        && !r.is_emergency_free_agent_depth()
                        && player_age.is_none_or(|age| {
                            age >= r.preferred_age_min
                                && age <= r.preferred_age_max.saturating_add(3)
                        })
                        // …and near the level the request asks for. A
                        // tip twenty points under the bar is not that
                        // request's answer; it opens its own, smaller
                        // request below (or nothing).
                        && rec
                            .assessed_ability
                            .saturating_add(BuyerNeedPicture::STAFF_TIP_ABILITY_TOLERANCE)
                            >= r.min_ability
                })
                .min_by_key(|r| {
                    (
                        r.priority.dashboard_sort_bucket(),
                        // Stable among equals: his own label first, so a
                        // single-group player is routed exactly as before.
                        u8::from(r.position.position_group() != player_pos_group),
                        r.id,
                    )
                });

            if let Some(req) = matching_request {
                self.attach_to_request(rec, req, reading, actions);
            } else if rec.confidence >= 0.6 && rec.assessed_ability >= 50 {
                self.open_request(rec, memory, reading, actions);
            }
        }
    }

    /// The recommendation answers a brief the club already has open: join its
    /// shortlist, or seed one when the brief has never had a shortlist built.
    ///
    /// That second case used to fall down a hole — a matching request with no
    /// shortlist satisfied the `if let Some(req)` arm, found nothing to add to,
    /// and returned, while the create-a-request arm is an `else`, so it never
    /// ran either.
    fn attach_to_request(
        &self,
        rec: &StaffRecommendation,
        req: &TransferRequest,
        reading: FileReading,
        actions: &mut Vec<RecommendationProcessAction>,
    ) {
        let club = self.club;
        let plan = self.plan;

        // Find the shortlist for this request
        let has_shortlist = plan
            .shortlists
            .iter()
            .any(|s| s.transfer_request_id == req.id);

        if has_shortlist {
            // Add as candidate to existing shortlist
            let already_in = plan.shortlists.iter().any(|s| {
                s.transfer_request_id == req.id
                    && s.candidates.iter().any(|c| c.player_id == rec.player_id)
            });

            if !already_in {
                actions.push(RecommendationProcessAction {
                    club_id: club.id,
                    reading,
                    kind: RecommendationProcessKind::AddToShortlist {
                        shortlist_request_id: req.id,
                        candidate: ShortlistCandidate {
                            player_id: rec.player_id,
                            // Same /200 ability scale as the
                            // scouting and market shortlist
                            // paths — the old /100 doubled the
                            // per-point weight and let a bare
                            // staff rec leapfrog fully vetted
                            // candidates on scale alone.
                            score: rec.assessed_ability as f32 / 200.0 + rec.confidence * 0.1,
                            estimated_fee: rec.estimated_fee,
                            status: ShortlistCandidateStatus::Available,
                        },
                    },
                });
            }
        } else if rec.confidence >= 0.6 && rec.assessed_ability >= 50 && req.budget_allocation > 0.0
        {
            // The request is open, funded and empty. Open its
            // shortlist with the name the department has
            // actually found — see `SeedShortlist`. A
            // zero-allocation request is deliberately excluded:
            // those are the free-agent matcher's territory, and
            // a shortlist would route them into the paid
            // negotiation path they carry no money for.
            actions.push(RecommendationProcessAction {
                club_id: club.id,
                reading,
                kind: RecommendationProcessKind::SeedShortlist {
                    request_id: req.id,
                    allocation: req.budget_allocation,
                    candidate: ShortlistCandidate {
                        player_id: rec.player_id,
                        score: rec.assessed_ability as f32 / 200.0 + rec.confidence * 0.1,
                        estimated_fee: rec.estimated_fee,
                        status: ShortlistCandidateStatus::Available,
                    },
                },
            });
        }
    }

    /// No open brief fits him, but the club's own staff rate him highly enough
    /// to open one. Returns early wherever the old body used `continue` — the
    /// branch was the last thing in the loop, so it means the same thing.
    fn open_request(
        &self,
        rec: &StaffRecommendation,
        memory: Option<&KnownPlayerMemory>,
        reading: FileReading,
        actions: &mut Vec<RecommendationProcessAction>,
    ) {
        let country = self.country;
        let club = self.club;
        let plan = self.plan;
        let player_lookup = self.lookup;

        // No existing request — create a new one
        let player_position =
            if let Some(player) = player_lookup.find_player(country, rec.player_id) {
                player.position()
            } else if let Some(memory) = memory {
                memory.position
            } else {
                return;
            };

        // Check we don't already have too many requests
        let active_requests = plan
            .transfer_requests
            .iter()
            .filter(|r| {
                r.status != TransferRequestStatus::Fulfilled
                    && r.status != TransferRequestStatus::Abandoned
            })
            .count();

        if active_requests >= 8 {
            return;
        }

        // Fund the PLAYER, not a flat slice of the pot.
        //
        // A marquee recommendation used to be allocated a flat
        // 15% of the available budget, which for a giant is a
        // fraction of what its own scouts said the target
        // costs. The request then arrived at the board with a
        // fee three or four times its allocation — read as
        // gross financial indiscipline — and at the shortlist
        // with a ceiling no candidate worth recommending could
        // fit under. The recommendation is a named player with
        // an estimated fee attached, so the honest allocation
        // is that fee, bounded by the share of the budget one
        // signing may consume.
        let available_budget = plan.available_budget();
        let alloc = rec
            .estimated_fee
            .max(available_budget * 0.15)
            .min(available_budget * SquadReviewPass::MAX_INVESTMENT_SHARE);

        if alloc <= 0.0 {
            return;
        }

        let next_id = plan.next_request_id
            + actions
                .iter()
                .filter(|a| {
                    a.club_id == club.id
                        && matches!(a.kind, RecommendationProcessKind::CreateRequest { .. })
                })
                .count() as u32;

        actions.push(RecommendationProcessAction {
            club_id: club.id,
            reading,
            kind: RecommendationProcessKind::CreateRequest {
                candidate: ShortlistCandidate {
                    player_id: rec.player_id,
                    // Same /200 scale as every other insertion
                    // path (see the AddToShortlist twin above).
                    score: rec.assessed_ability as f32 / 200.0 + rec.confidence * 0.1,
                    estimated_fee: rec.estimated_fee,
                    status: ShortlistCandidateStatus::Available,
                },
                request: TransferRequest::new(
                    next_id,
                    player_position,
                    TransferNeedPriority::Optional,
                    TransferNeedReason::StaffRecommendation,
                    rec.assessed_ability.saturating_sub(5),
                    rec.assessed_ability,
                    alloc,
                ),
            },
        });
    }
}

/// The single writer. Applies what the scan staged, club by club.
struct IntakeCommit;

impl IntakeCommit {
    fn apply(country: &mut Country, actions: Vec<RecommendationProcessAction>) {
        for action in actions {
            if let Some(club) = country.clubs.iter_mut().find(|c| c.id == action.club_id) {
                let plan = &mut club.transfer_plan;
                let reading = action.reading;

                match action.kind {
                    RecommendationProcessKind::AddToShortlist {
                        shortlist_request_id,
                        candidate,
                    } => {
                        plan.shortlist(reading, shortlist_request_id, 0.0, candidate, None);
                    }
                    RecommendationProcessKind::SeedShortlist {
                        request_id,
                        allocation,
                        candidate,
                    } => {
                        // Two recommendations seeding the same request in one
                        // pass share one list: the second joins the first's.
                        if !plan.shortlist(reading, request_id, allocation, candidate, None) {
                            continue;
                        }
                        // Downstream (the board review, the negotiation
                        // pass) keys on the request being Shortlisted.
                        if let Some(req) = plan
                            .transfer_requests
                            .iter_mut()
                            .find(|r| r.id == request_id)
                            && matches!(
                                req.status,
                                TransferRequestStatus::Pending
                                    | TransferRequestStatus::ScoutingActive
                            )
                        {
                            req.status = TransferRequestStatus::Shortlisted;
                        }
                    }
                    RecommendationProcessKind::CreateRequest {
                        mut request,
                        candidate,
                    } => {
                        let req_id = request.id;
                        let allocation = candidate.estimated_fee;
                        if !plan.shortlist(reading, req_id, allocation, candidate, None) {
                            continue;
                        }
                        if req_id >= plan.next_request_id {
                            plan.next_request_id = req_id + 1;
                        }
                        // The shortlist is attached here and now, so the
                        // request is Shortlisted — not Pending. That is what
                        // puts the marquee path in front of the board:
                        // `review_shortlist_proposals` only reviews requests
                        // at `Shortlisted`.
                        request.status = TransferRequestStatus::Shortlisted;
                        plan.transfer_requests.push(request);
                    }
                }
            }
        }
    }
}

/// The pass itself: scan every club, then commit.
pub(in crate::transfers::pipeline) struct RecommendationIntake;

impl RecommendationIntake {
    pub(in crate::transfers::pipeline) fn run(
        country: &mut Country,
        world: WorldPool<'_>,
        market_map: &MarketMap,
        date: NaiveDate,
    ) {
        let mut actions: Vec<RecommendationProcessAction> = Vec::new();

        // Indexed player/summary resolution for the per-recommendation
        // re-checks below (actions apply after the loop, so the index
        // stays valid for the whole pass).
        let player_lookup = CountryPlayerLookup::build(country);

        for club in &country.clubs {
            let plan = &club.transfer_plan;
            if !plan.initialized {
                continue;
            }
            IntakeScan {
                country,
                club,
                plan,
                lookup: &player_lookup,
                suitor: Suitor::of(country, club, date),
                world,
                market_map,
                date,
            }
            .stage(&mut actions);
        }

        IntakeCommit::apply(country, actions);
    }
}

#[cfg(test)]
mod readable_intake_tests {
    use super::*;
    use crate::transfers::pipeline::{
        KnownPlayerMemory, RecommendationSource, RecommendationType, StaffRecommendation,
    };
    use crate::transfers::tests::kit::{TestClub, TestCountry, TestDate, TestSummary};
    use crate::transfers::view::world::WorldPoolIndex;
    use crate::{PlayerFieldPositionGroup, PlayerPositionType};

    struct Fx;

    impl Fx {
        fn recommendation(player_id: u32) -> StaffRecommendation {
            StaffRecommendation {
                player_id,
                recommender_staff_id: 1,
                source: RecommendationSource::ScoutNetwork,
                recommendation_type: RecommendationType::ReadyForStepUp,
                assessed_ability: 110,
                assessed_potential: 120,
                confidence: 0.8,
                estimated_fee: 500_000.0,
                date_recommended: TestDate::today(),
            }
        }

        fn memory(player_id: u32) -> KnownPlayerMemory {
            KnownPlayerMemory {
                player_id,
                last_known_club_id: 70,
                last_known_country_id: 2,
                position: PlayerPositionType::MidfielderCenter,
                position_group: PlayerFieldPositionGroup::Midfielder,
                age: 25,
                assessed_ability: 110,
                assessed_potential: 120,
                confidence: 0.8,
                estimated_fee: 500_000.0,
                last_seen: TestDate::today(),
                official_appearances_seen: 2,
                friendly_appearances_seen: 0,
            }
        }
    }

    /// A recommendation the gate reads as "may be shortlisted" opens a
    /// request; one naming a man who no longer resolves opens nothing.
    #[test]
    fn intake_acts_only_on_a_player_the_gate_has_read() {
        let mut club = TestClub::new(1).reputation(4_000).build();
        club.transfer_plan.initialized = true;
        club.transfer_plan.total_budget = 5_000_000.0;
        for player_id in [81, 82] {
            club.transfer_plan
                .remember_known_player(Fx::memory(player_id));
            club.transfer_plan
                .staff_recommendations
                .push(Fx::recommendation(player_id));
        }
        let country = TestCountry::new(1).clubs(vec![club]).build();
        let readable = [TestSummary::new(81)
            .club_id(70)
            .country(2, "es")
            .league(4_000, 0.4)
            .salary(100_000)
            .value(500_000.0)
            .build()];
        let index = WorldPoolIndex::build(&readable);
        let lookup = CountryPlayerLookup::build(&country);
        let club = &country.clubs[0];
        let mut actions = Vec::new();
        IntakeScan {
            country: &country,
            club,
            plan: &club.transfer_plan,
            lookup: &lookup,
            suitor: Suitor::of(&country, club, TestDate::today()),
            world: WorldPool::new(&readable, &index),
            market_map: &MarketMap::default(),
            date: TestDate::today(),
        }
        .stage(&mut actions);
        let named: Vec<u32> = actions
            .iter()
            .filter_map(|a| match &a.kind {
                RecommendationProcessKind::CreateRequest { candidate, .. }
                | RecommendationProcessKind::SeedShortlist { candidate, .. }
                | RecommendationProcessKind::AddToShortlist { candidate, .. } => {
                    Some(candidate.player_id)
                }
            })
            .collect();
        assert_eq!(named, vec![81]);
    }
}
