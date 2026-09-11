use crate::club::board::manager_market::ManagerMarketTick;
use crate::competitions::simulation::GlobalCompetitionSimulator;
use crate::country::result::transfers::free::audit::FreeAgentMarketAuditor;
use crate::utils::PerformanceProfiler;
use crate::world::SimulatorData;
use crate::world::free_agents::LoanWageSettlement;
use chrono::Datelike;

/// The passes only the world level can run, because both sides of each
/// one can sit in different countries: managers moving between clubs, a
/// parent club's share of a loanee's wages, and the competitions whose
/// entrants are drawn from everywhere.
pub struct WorldPass;

impl WorldPass {
    pub fn run(data: &mut SimulatorData) {
        let today = data.date.date();

        // Order within the manager market is load-bearing — see
        // `ManagerMarketTick::run` for the dependency rationale.
        let phase = PerformanceProfiler::phase_scope("D1_manager_market", 0);
        ManagerMarketTick::run(data, today);
        drop(phase);

        let phase = PerformanceProfiler::phase_scope("D2_monthly_loan_wages_fa", 0);
        if today.day() == 1 {
            Self::monthly(data);
        }
        drop(phase);

        let _phase = PerformanceProfiler::phase_scope("D3_global_competitions", 0);
        GlobalCompetitionSimulator::simulate(data);
    }

    /// First of the month: settle what the borrowers did not cover,
    /// resolve the long sits in the free-agent pool, and log where the
    /// pool's tail came from before zeroing the counters that measure it.
    fn monthly(data: &mut SimulatorData) {
        let today = data.date.date();

        LoanWageSettlement::settle(data);

        // Long-unemployed free agents eventually hang up the boots.
        // Gated internally on `free_since` >= 12mo, with a deterministic
        // hard bound so unlucky rolls can't strand anyone in the pool for
        // multiple seasons.
        data.process_free_agent_retirements(today);

        // One debug line per 12-month-plus free agent explaining why
        // they're unsigned, then the aggregate: pool size, days-free
        // distribution with mean career pressure per cohort, the in/out
        // flow split by route, and the dominant block reasons. Both are
        // no-ops unless debug logging is enabled.
        FreeAgentMarketAuditor::log_long_term(data, today);
        FreeAgentMarketAuditor::log_pool_stats(data, today);
        // Reset after the log so next month measures only its own flow.
        data.free_agent_flow.reset();
    }
}
