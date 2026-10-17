//! Runs the official JSON-Schema-Test-Suite (draft 2020-12) through the
//! macro's lowering and the runtime. Groups whose schema uses a keyword we
//! do not support are skipped (the macro would reject them at compile
//! time); every other test must give the suite's verdict, whole and with
//! one-byte reads.

mod support;

use serde_json::Value;
use std::path::Path;

#[test]
fn test_json_schema_test_suite() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/json_schema_test_suite");
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect();
    files.sort();

    let (mut run, mut skipped_groups, mut groups) = (0, 0, 0);
    let mut failures = vec![];
    let mut skipped = std::collections::BTreeMap::<String, usize>::new();
    for file in files {
        let name = file.file_name().unwrap().to_string_lossy().into_owned();
        let suite: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        for group in suite.as_array().unwrap() {
            groups += 1;
            let desc = group["description"].as_str().unwrap();
            let cfg = match support::config(&[group["schema"].clone()], false) {
                Ok(cfg) => cfg,
                Err(e) if e.unsupported => {
                    skipped_groups += 1;
                    *skipped
                        .entry(e.keyword.clone().unwrap_or_default())
                        .or_default() += 1;
                    continue;
                }
                Err(e) => {
                    failures.push(format!("{}: {}: lowering failed: {}", name, desc, e));
                    continue;
                }
            };
            for test in group["tests"].as_array().unwrap() {
                let text = serde_json::to_string(&test["data"]).unwrap();
                let expected = test["valid"].as_bool().unwrap();
                for chunk in [usize::MAX, 1] {
                    let got = support::validate_text(cfg, text.as_bytes(), chunk);
                    if got.is_ok() != expected {
                        failures.push(format!(
                            "{}: {}: {}: expected {}, got {:?}",
                            name,
                            desc,
                            test["description"].as_str().unwrap(),
                            if expected { "valid" } else { "invalid" },
                            got
                        ));
                    }
                }
                run += 1;
            }
        }
    }
    eprintln!(
        "{} tests run; {} of {} groups skipped for unsupported keywords: {:?}",
        run, skipped_groups, groups, skipped
    );
    assert!(
        failures.is_empty(),
        "{} failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
