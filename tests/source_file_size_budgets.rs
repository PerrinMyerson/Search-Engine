use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

const HUGE_RUST_FILE_LINE_LIMIT: usize = 2_500;

#[derive(Debug)]
struct FileSizeBudget {
    path: &'static str,
    max_lines: usize,
    rationale: &'static str,
}

// These are current large files with explicit headroom while they are split
// into smaller modules. Additions here should explain why growth is temporary
// or why the file owns a naturally large boundary.
const EXPLICIT_LARGE_FILE_BUDGETS: &[FileSizeBudget] = &[
    FileSizeBudget {
        path: "src/browser.rs",
        max_lines: 12_000,
        rationale: "Browser runtime is the current decomposition target; keep extra growth visible while subsystems move behind modules.",
    },
    FileSizeBudget {
        path: "src/bench.rs",
        max_lines: 4_500,
        rationale: "Benchmark harness still centralizes scenarios and reporting; extra growth should trigger extraction into bench modules.",
    },
    FileSizeBudget {
        path: "src/server.rs",
        max_lines: 3_200,
        rationale: "HTTP/API server still owns HTML views, API handlers, status rendering, and in-module tests; temporary headroom until server tests/templates are split after browser runtime milestones stop changing adjacent status surfaces.",
    },
    FileSizeBudget {
        path: "src/browser/tests.rs",
        max_lines: 4_000,
        rationale: "Extracted browser integration-style regression tests from src/browser.rs; split by browser subsystem once the runtime module split settles.",
    },
];

#[test]
fn rust_source_files_stay_within_size_budgets() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut rust_files = Vec::new();

    collect_rust_files(&manifest_dir.join("src"), &mut rust_files);
    collect_rust_files(&manifest_dir.join("tests"), &mut rust_files);
    rust_files.sort();

    let budgets_by_path = EXPLICIT_LARGE_FILE_BUDGETS
        .iter()
        .map(|budget| {
            assert!(
                !budget.rationale.trim().is_empty(),
                "{} has a size budget without a rationale",
                budget.path
            );
            (budget.path, budget)
        })
        .collect::<BTreeMap<_, _>>();

    let mut seen_budget_paths = BTreeSet::new();
    let mut failures = Vec::new();

    for file_path in rust_files {
        let relative_path = relative_unix_path(manifest_dir, &file_path);
        let line_count = physical_line_count(&file_path);

        match budgets_by_path.get(relative_path.as_str()) {
            Some(budget) => {
                seen_budget_paths.insert(budget.path);

                if line_count <= HUGE_RUST_FILE_LINE_LIMIT {
                    failures.push(format!(
                        "{relative_path} is now {line_count} lines, under the shared \
                         {HUGE_RUST_FILE_LINE_LIMIT}-line limit; remove its explicit budget"
                    ));
                } else if line_count > budget.max_lines {
                    failures.push(format!(
                        "{relative_path} is {line_count} lines, exceeding its documented \
                         budget of {} lines. Split/refactor the file or update the budget \
                         with a fresh rationale. Current rationale: {}",
                        budget.max_lines, budget.rationale
                    ));
                }
            }
            None if line_count > HUGE_RUST_FILE_LINE_LIMIT => {
                failures.push(format!(
                    "{relative_path} is {line_count} lines, exceeding the shared \
                     {HUGE_RUST_FILE_LINE_LIMIT}-line limit. Split/refactor the file or \
                     add an explicit budget with a rationale in \
                     EXPLICIT_LARGE_FILE_BUDGETS."
                ));
            }
            None => {}
        }
    }

    for budget in EXPLICIT_LARGE_FILE_BUDGETS {
        if !seen_budget_paths.contains(budget.path) {
            failures.push(format!(
                "{} has a documented size budget but no matching Rust source file",
                budget.path
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "Rust source file size guard failed:\n{}",
        failures.join("\n")
    );
}

fn collect_rust_files(dir: &Path, rust_files: &mut Vec<PathBuf>) {
    let mut entries = fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", dir.display()))
        .map(|entry| {
            entry
                .unwrap_or_else(|error| {
                    panic!("failed to read entry in {}: {error}", dir.display())
                })
                .path()
        })
        .collect::<Vec<_>>();
    entries.sort();

    for entry in entries {
        if entry.is_dir() {
            collect_rust_files(&entry, rust_files);
        } else if entry.extension().and_then(|extension| extension.to_str()) == Some("rs") {
            rust_files.push(entry);
        }
    }
}

fn physical_line_count(path: &Path) -> usize {
    let contents =
        fs::read(path).unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
    let newline_count = contents.iter().filter(|byte| **byte == b'\n').count();

    if contents.is_empty() || contents.ends_with(b"\n") {
        newline_count
    } else {
        newline_count + 1
    }
}

fn relative_unix_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or_else(|error| {
            panic!(
                "failed to strip {} from {}: {error}",
                root.display(),
                path.display()
            )
        })
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}
