//! Optional comparison against the source database, before hydration modifies
//! any attributes. Run with OPEN_FOOTBALL_DATA pointing at its data directory:
//! cargo test -p database audit_source_ability -- --ignored --nocapture
//! ABILITY_AUDIT_OUTPUT optionally receives one JSON line per complete profile.

use super::*;
use std::collections::{BTreeMap, HashSet};
use std::io::Write;
use std::path::Path;

fn player_files(dir: &Path, files: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("read source database") {
        let path = entry.expect("source entry").path();
        if path.is_dir() {
            player_files(&path, files);
        } else if path.extension().is_some_and(|ext| ext == "json")
            && path
                .parent()
                .and_then(Path::file_name)
                .is_some_and(|name| name == "players" || name == "free_agents")
        {
            files.push(path);
        }
    }
}

#[test]
#[ignore = "requires the external source database via OPEN_FOOTBALL_DATA"]
fn audit_source_ability() {
    let root = std::env::var("OPEN_FOOTBALL_DATA").expect("set OPEN_FOOTBALL_DATA");
    let mut files = Vec::new();
    player_files(Path::new(&root), &mut files);
    files.sort();
    let mut output = std::env::var("ABILITY_AUDIT_OUTPUT").ok().map(|path| {
        std::io::BufWriter::new(std::fs::File::create(path).expect("create audit output"))
    });
    let mut seen = HashSet::new();
    let mut errors: BTreeMap<String, Vec<i32>> = BTreeMap::new();
    let (mut partial, mut invalid) = (0, 0);
    for path in &files {
        let bytes = std::fs::read(path).expect("read player");
        let record: OdbPlayer = match serde_json::from_slice(&bytes) {
            Ok(record) => record,
            Err(err) => panic!("{}: {err}", path.display()),
        };
        if !(1..=200).contains(&record.current_ability) {
            invalid += 1;
            continue;
        }
        let Some(attrs) = &record.attrs else { continue };
        let recorded = RecordedSkills::from_attrs(&attrs.player);
        let primary = positions_from_odb(record.id, &record.positions).positions[0].position;
        // Do not impute missing inputs then present the result as validation.
        // Only score records whose missing slots cannot affect the result.
        let mut lower = recorded.values;
        let mut upper = recorded.values;
        RecordedSkills::each(&mut lower, &recorded.values, |dst, v| *dst = v.max(1.0));
        RecordedSkills::each(&mut upper, &recorded.values, |dst, v| {
            *dst = if v > 0.0 { v } else { 20.0 };
        });
        let derived = lower.calculate_ability_for_position(primary);
        if derived != upper.calculate_ability_for_position(primary) {
            partial += 1;
            continue;
        }
        if !seen.insert(record.id) {
            continue;
        }
        let error = derived as i32 - record.current_ability as i32;
        errors
            .entry(format!("{primary:?}"))
            .or_default()
            .push(error);
        if let Some(out) = &mut output {
            let value = serde_json::json!({
                "id": record.id, "name": format!("{} {}", record.first_name, record.last_name),
                "path": path, "position": format!("{primary:?}"),
                "ca": record.current_ability, "pa": record.potential_ability,
                "derived": derived, "skills": lower,
                "recorded_skills": recorded.values,
                "positions": record.positions.iter().map(|p| (&p.code, p.level)).collect::<Vec<_>>(),
                "left_foot": record.foots.as_ref().map(|f| f.left),
                "right_foot": record.foots.as_ref().map(|f| f.right),
            });
            writeln!(out, "{value}").expect("write audit record");
        }
    }
    let all: Vec<i32> = errors.values().flatten().copied().collect();
    errors.insert("ALL".into(), all);
    eprintln!(
        "{} source files; {partial} partial attribute records excluded; {invalid} invalid/unspecified CA",
        files.len()
    );
    for (position, values) in errors {
        let n = values.len() as f64;
        let bias = values.iter().sum::<i32>() as f64 / n;
        let mae = values.iter().map(|e| e.abs()).sum::<i32>() as f64 / n;
        let within = values.iter().filter(|e| e.abs() <= ODB_CA_DRIFT).count();
        eprintln!(
            "{position}: n={} bias={bias:.2} MAE={mae:.2} within6={within} ({:.1}%)",
            values.len(),
            within as f64 / n * 100.0
        );
    }
    assert!(!seen.is_empty(), "no complete profiles found");
}
