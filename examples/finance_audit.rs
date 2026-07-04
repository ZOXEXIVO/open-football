//! Headless multi-year world simulation that audits club finances.
//!
//! Answers "where does the money come from" per reputation tier: runs the
//! real daily simulator (build with `--features match-stub` so matches
//! resolve instantly), then aggregates every club's monthly finance
//! history into per-category annual flows.
//!
//! Usage:
//!   cargo run --release --features match-stub --example finance_audit [years]

use database::{DatabaseGenerator, DatabaseLoader};
use env_logger::Env;
use simulator_core::{
    Club, ClubFinancialBalance, FootballSimulator, ReputationLevel, SimulatorData,
};
use std::collections::HashMap;
use std::time::Instant;

/// Per-club cumulative category flows summed over the whole run
/// (monthly history snapshots + the live in-progress month).
#[derive(Default, Clone)]
struct Flows {
    income: i64,
    outcome: i64,
    tv: i64,
    tv_placement: i64,
    matchday: i64,
    sponsorship: i64,
    merchandising: i64,
    prize: i64,
    loan_fees_in: i64,
    wages: i64,
    staff_wages: i64,
    facilities: i64,
    amortization: i64,
    debt_interest: i64,
    loan_fees_out: i64,
    months: i64,
}

impl Flows {
    fn absorb(&mut self, b: &ClubFinancialBalance) {
        self.income += b.income;
        self.outcome += b.outcome;
        self.tv += b.income_tv;
        self.tv_placement += b.income_tv_placement;
        self.matchday += b.income_matchday;
        self.sponsorship += b.income_sponsorship;
        self.merchandising += b.income_merchandising;
        self.prize += b.income_prize_money;
        self.loan_fees_in += b.income_loan_fees;
        self.wages += b.expense_player_wages;
        self.staff_wages += b.expense_staff_wages;
        self.facilities += b.expense_facilities;
        self.amortization += b.expense_amortization;
        self.debt_interest += b.expense_debt_interest;
        self.loan_fees_out += b.expense_loan_fees;
        self.months += 1;
    }

    /// Income that no category claims: transfer sales, continental match /
    /// participation bonuses, year-end interest, takeover injections.
    fn other_income(&self) -> i64 {
        self.income
            - (self.tv
                + self.tv_placement
                + self.matchday
                + self.sponsorship
                + self.merchandising
                + self.prize
                + self.loan_fees_in)
    }

    /// Outcome that no category claims (year-end debt penalty etc.).
    fn other_outcome(&self) -> i64 {
        self.outcome
            - (self.wages
                + self.staff_wages
                + self.facilities
                + self.amortization
                + self.debt_interest
                + self.loan_fees_out)
    }
}

struct ClubReport {
    name: String,
    country: String,
    tier: ReputationLevel,
    league_tier: u8,
    start_balance: i64,
    end_balance: i64,
    flows: Flows,
}

struct FinanceAudit;

impl FinanceAudit {
    fn m(v: i64) -> f64 {
        v as f64 / 1_000_000.0
    }

    fn tier_name(t: ReputationLevel) -> &'static str {
        match t {
            ReputationLevel::Elite => "Elite",
            ReputationLevel::Continental => "Continental",
            ReputationLevel::National => "National",
            ReputationLevel::Regional => "Regional",
            ReputationLevel::Local => "Local",
            ReputationLevel::Amateur => "Amateur",
        }
    }

    fn tier_rank(t: ReputationLevel) -> usize {
        match t {
            ReputationLevel::Elite => 0,
            ReputationLevel::Continental => 1,
            ReputationLevel::National => 2,
            ReputationLevel::Regional => 3,
            ReputationLevel::Local => 4,
            ReputationLevel::Amateur => 5,
        }
    }

    fn visit_clubs<'a>(data: &'a SimulatorData, mut f: impl FnMut(&'a str, u8, &'a Club)) {
        for continent in &data.continents {
            for country in &continent.countries {
                let league_tiers: HashMap<u32, u8> = country
                    .leagues
                    .leagues
                    .iter()
                    .map(|l| (l.id, l.settings.tier))
                    .collect();
                for club in &country.clubs {
                    let league_tier = club
                        .teams
                        .main()
                        .and_then(|t| t.league_id)
                        .and_then(|id| league_tiers.get(&id).copied())
                        .unwrap_or(0);
                    f(&country.name, league_tier, club);
                }
            }
        }
    }

    fn snapshot(data: &SimulatorData, label: &str) {
        let mut by_tier: [Vec<i64>; 6] = Default::default();
        Self::visit_clubs(data, |_, _, club| {
            let tier = club
                .teams
                .main()
                .map(|t| t.reputation.level())
                .unwrap_or(ReputationLevel::Amateur);
            by_tier[Self::tier_rank(tier)].push(club.finance.balance.balance);
        });

        println!("\n=== Balance distribution: {label} ===");
        println!(
            "{:<12} {:>6} {:>10} {:>10} {:>10} {:>10} {:>10} {:>8} {:>8}",
            "tier", "clubs", "mean $M", "p50 $M", "p90 $M", "max $M", "min $M", ">100M", ">300M"
        );
        let order = [
            ReputationLevel::Elite,
            ReputationLevel::Continental,
            ReputationLevel::National,
            ReputationLevel::Regional,
            ReputationLevel::Local,
            ReputationLevel::Amateur,
        ];
        for tier in order {
            let mut v = by_tier[Self::tier_rank(tier)].clone();
            if v.is_empty() {
                continue;
            }
            v.sort_unstable();
            let n = v.len();
            let mean = v.iter().sum::<i64>() / n as i64;
            let p50 = v[n / 2];
            let p90 = v[(n * 9 / 10).min(n - 1)];
            let over100 = v.iter().filter(|&&b| b > 100_000_000).count();
            let over300 = v.iter().filter(|&&b| b > 300_000_000).count();
            println!(
                "{:<12} {:>6} {:>10.1} {:>10.1} {:>10.1} {:>10.1} {:>10.1} {:>8} {:>8}",
                Self::tier_name(tier),
                n,
                Self::m(mean),
                Self::m(p50),
                Self::m(p90),
                Self::m(v[n - 1]),
                Self::m(v[0]),
                over100,
                over300,
            );
        }
    }

    fn collect_reports(data: &SimulatorData, initial: &HashMap<u32, i64>) -> Vec<ClubReport> {
        let mut reports = Vec::new();
        Self::visit_clubs(data, |country_name, league_tier, club| {
            let tier = club
                .teams
                .main()
                .map(|t| t.reputation.level())
                .unwrap_or(ReputationLevel::Amateur);
            let mut flows = Flows::default();
            for (_, snap) in club.finance.history.iter() {
                flows.absorb(snap);
            }
            // Live in-progress month.
            flows.absorb(&club.finance.balance);
            flows.months -= 1; // live month is partial; don't count it as a full month
            reports.push(ClubReport {
                name: club.name.clone(),
                country: country_name.to_string(),
                tier,
                league_tier,
                start_balance: initial.get(&club.id).copied().unwrap_or(0),
                end_balance: club.finance.balance.balance,
                flows,
            });
        });
        reports
    }

    fn print_flow_header() {
        println!(
            "{:<12} {:>5} | {:>8} {:>8} {:>8} {:>8} {:>8} {:>8} {:>9} | {:>8} {:>8} {:>8} {:>8} {:>8} | {:>9} {:>9}",
            "tier",
            "clubs",
            "tv",
            "gate",
            "sponsor",
            "merch",
            "prize",
            "loans",
            "OTHER-IN",
            "wages",
            "staff",
            "facil",
            "amort",
            "loans",
            "netP&L/y",
            "dCash/y"
        );
    }

    fn print_flow_row(label: &str, reports: &[&ClubReport], years: f64) {
        let n = reports.len().max(1) as f64;
        let sum = |f: fn(&Flows) -> i64| -> f64 {
            reports.iter().map(|r| f(&r.flows)).sum::<i64>() as f64 / n / years / 1_000_000.0
        };
        let net_pl = reports
            .iter()
            .map(|r| r.flows.income - r.flows.outcome)
            .sum::<i64>() as f64
            / n
            / years
            / 1_000_000.0;
        let d_cash = reports
            .iter()
            .map(|r| r.end_balance - r.start_balance)
            .sum::<i64>() as f64
            / n
            / years
            / 1_000_000.0;
        println!(
            "{:<12} {:>5} | {:>8.2} {:>8.2} {:>8.2} {:>8.2} {:>8.2} {:>8.2} {:>9.2} | {:>8.2} {:>8.2} {:>8.2} {:>8.2} {:>8.2} | {:>9.2} {:>9.2}",
            label,
            reports.len(),
            sum(|f| f.tv + f.tv_placement),
            sum(|f| f.matchday),
            sum(|f| f.sponsorship),
            sum(|f| f.merchandising),
            sum(|f| f.prize),
            sum(|f| f.loan_fees_in),
            sum(|f| f.other_income()),
            sum(|f| f.wages),
            sum(|f| f.staff_wages),
            sum(|f| f.facilities),
            sum(|f| f.amortization),
            sum(|f| f.loan_fees_out + f.debt_interest + f.other_outcome()),
            net_pl,
            d_cash,
        );
    }

    fn deep_dive(data: &SimulatorData, initial: &HashMap<u32, i64>, years: f64) {
        let reports = Self::collect_reports(data, initial);

        println!("\n=== Average per-club ANNUAL flows by tier ($M/club/year) ===");
        println!(
            "(OTHER-IN = transfer sales + continental bonuses + interest + takeover injections)"
        );
        Self::print_flow_header();
        let order = [
            ReputationLevel::Elite,
            ReputationLevel::Continental,
            ReputationLevel::National,
            ReputationLevel::Regional,
            ReputationLevel::Local,
            ReputationLevel::Amateur,
        ];
        for tier in order {
            let tier_reports: Vec<&ClubReport> =
                reports.iter().filter(|r| r.tier == tier).collect();
            if tier_reports.is_empty() {
                continue;
            }
            Self::print_flow_row(Self::tier_name(tier), &tier_reports, years);
        }

        println!("\n=== Same, split by league tier (division) ===");
        Self::print_flow_header();
        for div in 1..=4u8 {
            let div_reports: Vec<&ClubReport> =
                reports.iter().filter(|r| r.league_tier == div).collect();
            if div_reports.is_empty() {
                continue;
            }
            Self::print_flow_row(&format!("division-{div}"), &div_reports, years);
        }

        println!("\n=== Top 15 richest clubs BELOW Continental tier ===");
        let mut small: Vec<&ClubReport> = reports
            .iter()
            .filter(|r| r.tier < ReputationLevel::Continental)
            .collect();
        small.sort_by_key(|r| -r.end_balance);
        println!(
            "{:<28} {:<14} {:<10} {:>4} | {:>8} {:>8} | {:>8} {:>8} {:>8} {:>9} | {:>8}",
            "club",
            "country",
            "tier",
            "div",
            "start$M",
            "end$M",
            "tv/y",
            "gate/y",
            "prize/y",
            "OTHER-IN/y",
            "wages/y"
        );
        for r in small.iter().take(15) {
            println!(
                "{:<28} {:<14} {:<10} {:>4} | {:>8.1} {:>8.1} | {:>8.2} {:>8.2} {:>8.2} {:>9.2} | {:>8.2}",
                r.name.chars().take(28).collect::<String>(),
                r.country.chars().take(14).collect::<String>(),
                Self::tier_name(r.tier),
                r.league_tier,
                Self::m(r.start_balance),
                Self::m(r.end_balance),
                Self::m(r.flows.tv + r.flows.tv_placement) / years,
                Self::m(r.flows.matchday) / years,
                Self::m(r.flows.prize) / years,
                Self::m(r.flows.other_income()) / years,
                Self::m(r.flows.wages) / years,
            );
        }
    }
}

#[tokio::main]
async fn main() {
    env_logger::Builder::from_env(Env::default().default_filter_or("warn")).init();

    let years: u32 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(3);

    println!("Loading database...");
    let database = DatabaseLoader::load();
    println!("Generating world...");
    let mut data = DatabaseGenerator::generate(&database);

    let mut initial_balances: HashMap<u32, i64> = HashMap::new();
    FinanceAudit::visit_clubs(&data, |_, _, club| {
        initial_balances.insert(club.id, club.finance.balance.balance);
    });

    FinanceAudit::snapshot(&data, "start");

    let total_days = years * 365;
    let started = Instant::now();
    for day in 0..total_days {
        FootballSimulator::simulate(&mut data).await;
        if (day + 1) % 30 == 0 {
            println!(
                "... simulated {} days ({:.1}%), elapsed {:.0}s, date {}",
                day + 1,
                (day + 1) as f64 / total_days as f64 * 100.0,
                started.elapsed().as_secs_f64(),
                data.date.date()
            );
        }
        if (day + 1) % 365 == 0 {
            FinanceAudit::snapshot(&data, &format!("year {}", (day + 1) / 365));
        }
    }

    FinanceAudit::deep_dive(&data, &initial_balances, years as f64);
}
