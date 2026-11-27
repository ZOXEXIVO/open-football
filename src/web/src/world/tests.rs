use super::store::WorldStore;
use super::{ProcessOutcome, World, WorldSession};
use crate::I18nManager;
use chrono::{NaiveDate, NaiveDateTime, TimeDelta};
use core::{GlobalCompetitions, SimulatorData};
use std::sync::mpsc;
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};
use tokio::runtime::Runtime;

fn day(n: u32) -> NaiveDateTime {
    NaiveDate::from_ymd_opt(2026, 8, n)
        .unwrap()
        .and_hms_opt(0, 0, 0)
        .unwrap()
}

fn sim(n: u32) -> SimulatorData {
    SimulatorData::new(day(n), Vec::new(), GlobalCompetitions::new(Vec::new()))
}

fn world(n: u32) -> World {
    World::start(Some(sim(n)), Arc::new(I18nManager::new()))
}

fn runtime() -> Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

fn mark(sim: &mut SimulatorData, by: u32) {
    sim.free_agent_flow.released_to_pool += by;
}

fn marks(sim: &SimulatorData) -> u32 {
    sim.free_agent_flow.released_to_pool
}

fn freed_within(world: &Weak<SimulatorData>, limit: Duration) -> bool {
    let deadline = Instant::now() + limit;
    while Instant::now() < deadline {
        if world.upgrade().is_none() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

/// A run of fake days that each advance the date by one, stopping after
/// every day until the test lets it go on. It takes queued edits at each
/// boundary and once after the last day, as `ProcessingRun` does.
fn stepped_run(
    days: u32,
    stepped: mpsc::Sender<u32>,
    gate: mpsc::Receiver<()>,
) -> impl FnOnce(&mut WorldSession<'_>) + Send + 'static {
    move |session| {
        for n in 1..=days {
            session.catch_up(|| {});
            if session.cancelled() {
                break;
            }
            session.world().date += TimeDelta::days(1);
            stepped.send(n).unwrap();
            gate.recv().unwrap();
        }
        session.catch_up(|| {});
    }
}

#[test]
fn an_empty_store_has_no_snapshot() {
    assert!(WorldStore::new(None).snapshot().is_none());
}

#[test]
fn a_held_snapshot_is_unchanged_by_a_publish() {
    let store = WorldStore::new(Some(sim(1)));
    let held = store.snapshot().unwrap();

    let previous = store.publish(Arc::new(sim(2))).unwrap();

    assert!(Arc::ptr_eq(&held, &previous));
    assert_eq!(held.date, day(1));
}

#[test]
fn a_snapshot_taken_after_a_publish_reads_the_new_world() {
    let store = WorldStore::new(Some(sim(1)));
    store.publish(Arc::new(sim(2)));
    assert_eq!(store.snapshot().unwrap().date, day(2));
}

#[test]
fn two_concurrent_edits_are_both_present_after_both_answers() {
    let world = world(1);
    let first = world.edit(|sim| mark(sim, 1));
    let second = world.edit(|sim| mark(sim, 2));

    let (first, second) = runtime().block_on(async { tokio::join!(first, second) });

    assert!(first.unwrap().is_some() && second.unwrap().is_some());
    assert_eq!(marks(&world.snapshot().unwrap()), 3);
}

#[test]
fn a_snapshot_taken_right_after_an_answer_contains_the_edit() {
    let world = world(1);
    let answer = runtime().block_on(world.edit(|sim| {
        mark(sim, 5);
        marks(sim)
    }));
    assert_eq!(answer.unwrap(), Some(5));
    assert_eq!(marks(&world.snapshot().unwrap()), 5);
}

#[test]
fn an_edit_without_a_world_answers_none() {
    let world = World::start(None, Arc::new(I18nManager::new()));
    let answer = runtime().block_on(world.edit(|sim| mark(sim, 1)));
    assert_eq!(answer.unwrap(), None);
    assert!(world.snapshot().is_none());
}

#[test]
fn a_stalled_reader_does_not_hold_up_an_edit() {
    let world = world(1);
    let stalled = world.snapshot().unwrap();

    let answer = runtime().block_on(async {
        tokio::time::timeout(Duration::from_secs(2), world.edit(|sim| mark(sim, 1))).await
    });

    assert!(answer.expect("the edit is answered").unwrap().is_some());
    assert_eq!(marks(&world.snapshot().unwrap()), 1);
    assert_eq!(marks(&stalled), 0);
}

#[test]
fn a_superseded_world_is_freed_once_its_last_reader_lets_go() {
    let world = world(1);
    let held = world.snapshot().unwrap();
    let superseded = Arc::downgrade(&held);

    runtime().block_on(world.edit(|sim| mark(sim, 1))).unwrap();

    assert!(Arc::strong_count(&held) >= 2, "the writer still holds it");
    drop(held);
    assert!(freed_within(&superseded, Duration::from_secs(2)));
}

#[test]
fn a_panicking_edit_leaves_the_world_and_the_writer_intact() {
    let world = world(1);
    let before = world.snapshot().unwrap();

    let failed = runtime().block_on(world.edit(|sim| {
        mark(sim, 1);
        sim.date = day(9);
        panic!("an editor bug");
    }));

    assert!(failed.is_err());
    let after = world.snapshot().unwrap();
    assert!(Arc::ptr_eq(&before, &after));
    assert_eq!((after.date, marks(&after)), (day(1), 0));

    let next = runtime().block_on(world.edit(|sim| mark(sim, 2)));
    assert!(next.unwrap().is_some());
    assert_eq!(marks(&world.snapshot().unwrap()), 2);
}

#[test]
fn an_edit_during_a_run_is_published_at_the_next_day_boundary() {
    let world = world(1);
    let runtime = runtime();
    let (stepped, steps) = mpsc::channel();
    let (go, gate) = mpsc::channel();
    let run = world.process(stepped_run(3, stepped, gate));
    assert!(world.processing());

    assert_eq!(steps.recv().unwrap(), 1);
    let edit = world.edit(|sim| mark(sim, 7));
    go.send(()).unwrap();
    assert_eq!(steps.recv().unwrap(), 2);

    assert!(runtime.block_on(edit).unwrap().is_some());
    let published = world.snapshot().unwrap();
    assert!(world.processing(), "the run has not ended");
    assert_eq!((published.date, marks(&published)), (day(2), 7));

    go.send(()).unwrap();
    assert_eq!(steps.recv().unwrap(), 3);
    go.send(()).unwrap();
    assert!(matches!(runtime.block_on(run), ProcessOutcome::Finished));
    assert!(!world.processing());
    let last = world.snapshot().unwrap();
    assert_eq!((last.date, marks(&last)), (day(4), 7));
}

#[test]
fn a_cancelled_run_still_publishes_a_waiting_edit() {
    let world = world(1);
    let runtime = runtime();
    let (stepped, steps) = mpsc::channel();
    let (go, gate) = mpsc::channel();
    let run = world.process(stepped_run(5, stepped, gate));

    assert_eq!(steps.recv().unwrap(), 1);
    let edit = world.edit(|sim| mark(sim, 3));
    world.cancel();
    go.send(()).unwrap();

    assert!(matches!(runtime.block_on(run), ProcessOutcome::Finished));
    assert!(runtime.block_on(edit).unwrap().is_some());
    let last = world.snapshot().unwrap();
    assert_eq!((last.date, marks(&last)), (day(2), 3));
}

#[test]
fn a_second_run_is_refused_while_one_is_in_progress() {
    let world = world(1);
    let runtime = runtime();
    let (stepped, steps) = mpsc::channel();
    let (go, gate) = mpsc::channel();
    let run = world.process(stepped_run(1, stepped, gate));
    steps.recv().unwrap();

    let refused = world.process(|session| session.world().date = day(20));
    assert!(matches!(
        runtime.block_on(refused),
        ProcessOutcome::AlreadyRunning
    ));

    go.send(()).unwrap();
    assert!(matches!(runtime.block_on(run), ProcessOutcome::Finished));
    assert_eq!(world.snapshot().unwrap().date, day(2));
}
