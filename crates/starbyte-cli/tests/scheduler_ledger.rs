// The JSON is a machine-readable mirror, not an alternative acceptance
// authority. Keep each checked task synchronized with the Markdown source.
use std::collections::HashSet;

#[test]
fn scheduler_ledger_matches_checked_roadmap_and_declared_percentages() {
    let roadmap = include_str!("../../../docs/scheduler-migration.md");
    let ledger: serde_json::Value =
        serde_json::from_str(include_str!("../../../docs/scheduler-progress.json"))
            .expect("scheduler-progress.json must be valid JSON");
    assert_eq!(ledger["schema_version"], 1);
    assert_eq!(ledger["source"], "docs/scheduler-migration.md");
    let milestones = ledger["milestones"]
        .as_array()
        .expect("milestones must be an array");
    assert_eq!(milestones.len(), 5);
    let mut ids = HashSet::new();
    let (mut completed, mut total) = (0_u64, 0_u64);
    for milestone in milestones {
        let id = milestone["id"].as_str().expect("milestone ID must be text");
        assert!(ids.insert(id.to_owned()), "duplicate milestone {id}");
        assert!(["A", "B", "P", "C", "D"].contains(&id));
        let heading = format!("## Milestone {id}:");
        let section = roadmap
            .split_once(&heading)
            .unwrap_or_else(|| panic!("missing source heading for {id}"))
            .1
            .split("\n## ")
            .next()
            .expect("milestone source section");
        let checkboxes: Vec<(bool, &str)> = section
            .lines()
            .filter_map(|line| {
                line.strip_prefix("- [x] ")
                    .map(|text| (true, text))
                    .or_else(|| line.strip_prefix("- [ ] ").map(|text| (false, text)))
            })
            .collect();
        let tasks = milestone["tasks"]
            .as_array()
            .expect("tasks must be an array");
        assert_eq!(tasks.len(), checkboxes.len(), "{id}: task count mismatch");
        let mut accepted = 0_u64;
        for (index, (task, (checked, source_text))) in
            tasks.iter().zip(checkboxes.iter()).enumerate()
        {
            assert_eq!(
                task["id"].as_str(),
                Some(format!("{id}-{:02}", index + 1).as_str()),
                "{id}: unstable task identifier"
            );
            assert_eq!(
                task["text"].as_str(),
                Some(*source_text),
                "{id}: text or task ordering differs from roadmap"
            );
            let is_accepted = task["status"] == "accepted";
            assert!(
                task["status"] == "accepted" || task["status"] == "pending",
                "{id}: unknown acceptance state"
            );
            assert_eq!(is_accepted, *checked, "{id}: acceptance checkbox drift");
            accepted += u64::from(is_accepted);
        }
        let n = tasks.len() as u64;
        let hundredths = (10_000 * accepted + n / 2) / n;
        let expected_percent = hundredths as f64 / 100.0;
        assert_eq!(milestone["accepted"].as_u64(), Some(accepted), "{id}");
        assert_eq!(milestone["total"].as_u64(), Some(n), "{id}");
        let recorded_percent = milestone["percent"].as_f64().expect("numeric milestone percent");
        assert!((recorded_percent - expected_percent).abs() < 1e-9, "{id}: percentage drift");
        assert!(
            section
                .lines()
                .next()
                .unwrap_or_default()
                .contains(&format!("— {accepted}/{n}")),
            "{id}: checklist heading fraction drift"
        );
        total += n;
        completed += accepted;
    }
    assert_eq!(total, 77, "source task count unexpectedly changed");
    let rounded_percent = (100 * completed + total / 2) / total;
    let exact_hundredths = (10_000 * completed + total / 2) / total;
    let expected_percent = exact_hundredths as f64 / 100.0;
    assert_eq!(ledger["overall"]["accepted"].as_u64(), Some(completed));
    assert_eq!(ledger["overall"]["total"].as_u64(), Some(total));
    let recorded_percent = ledger["overall"]["percent"].as_f64().expect("numeric overall percent");
    assert!((recorded_percent - expected_percent).abs() < 1e-9, "overall percentage drift");
    assert!(
        roadmap.contains(&format!("{completed}/{total} ({rounded_percent}%)")),
        "top-level Markdown progress differs from JSON"
    );
    // 2026-10-24 physical audit: local Codex ran the real Hyprland harness at
    // b1448b3. A-08 was supported by 142 off-workspace frames at 59.81 FPS;
    // D-14 by two away frames, one suspension/resume and ten resumed frames
    // over 166 ms. Focused clock, audible output, controller and production
    // cutover remain independently pending.
    assert_eq!(completed, 71, "accepted tasks changed without audit");
    for (group, task) in [("A", "A-08"), ("D", "D-14")] {
        let milestone = milestones
            .iter()
            .find(|m| m["id"] == group)
            .expect("audited milestone");
        assert_eq!(
            milestone["tasks"]
                .as_array()
                .expect("audited tasks")
                .iter()
                .find(|item| item["id"] == task)
                .expect("audited task")["status"],
            "accepted",
            "{task} must match actual physical evidence"
        );
    }
    for (group, task) in [
        ("P", "P-18"),
        ("P", "P-19"),
        ("C", "C-16"),
        ("D", "D-12"),
        ("D", "D-13"),
        ("D", "D-15"),
    ] {
        let milestone = milestones
            .iter()
            .find(|m| m["id"] == group)
            .expect("pending milestone");
        assert_eq!(
            milestone["tasks"]
                .as_array()
                .expect("pending tasks")
                .iter()
                .find(|item| item["id"] == task)
                .expect("pending task")["status"],
            "pending",
            "{task} requires separate acceptance"
        );
    }
    assert!(
        roadmap.contains("first actual desktop Hyprland acceptance — 11/12 physical probes"),
        "physical acceptance audit must remain documented"
    );
}
