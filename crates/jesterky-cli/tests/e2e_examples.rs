use jsonschema::JSONSchema;
use serde_json::json;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

const RUNNABLE_EXAMPLES: &[&str] = &[
    "gelo_trace_annotate.json",
    "gepa_trace_annotate.json",
    "goal_quality_gate.json",
    "obliq_math_verify.json",
    "quality_min.json",
    "quality_scan.json",
    "quality_scan_blogs.json",
    "quality_scan_docs.json",
    "smr_reportbench_trace_evaluate.json",
];

const INVALID_EXAMPLES: &[(&str, &str)] =
    &[("invalid_dangling_entrypoint.json", "error entrypoint[0]")];

const SKIPPED_EXAMPLES: &[SkippedExample] = &[SkippedExample {
    file: "dungeongrid_4p.json",
    reason: "LLM-only workflow; the CLI rejects --actor fake for DungeonGrid hero policy turns",
}];

struct SkippedExample {
    file: &'static str,
    reason: &'static str,
}

struct ExampleFixtures {
    _temp: tempfile::TempDir,
    blog_dir: PathBuf,
    docs_dir: PathBuf,
}

#[test]
fn runnable_examples_fake_run_replay_and_match_manifest_schema() {
    let bin = env!("CARGO_BIN_EXE_jesterky");
    let root = workspace_root();
    let temp = tempfile::tempdir().expect("tempdir");
    let fixtures = example_fixtures();
    let manifest_schema = read_json(&root.join("jesterky.manifest.schema.json"));
    let manifest_schema = JSONSchema::compile(&manifest_schema).expect("manifest schema compiles");

    assert_example_inventory_is_explicit(&root);

    for example in RUNNABLE_EXAMPLES {
        let spec = root.join("examples").join(example);
        let manifest = temp
            .path()
            .join(example.trim_end_matches(".json"))
            .with_extension("manifest.json");
        let mut run = Command::new(bin);
        run.current_dir(&root)
            .arg("run")
            .arg(&spec)
            .arg("--actor")
            .arg("fake")
            .arg("--out")
            .arg(&manifest);
        if let Some(args) = args_for_example(example, &fixtures) {
            run.arg("--args").arg(args);
        }
        let run = run.output().expect("run command executes");
        assert!(
            run.status.success(),
            "{example} fake run failed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&run.stdout),
            String::from_utf8_lossy(&run.stderr)
        );
        let stdout = String::from_utf8_lossy(&run.stdout);
        assert!(
            stdout.contains("status=completed"),
            "{example} should print completed status; got:\n{stdout}"
        );

        let manifest_json = read_json(&manifest);
        assert_eq!(
            manifest_json.get("status").and_then(|value| value.as_str()),
            Some("completed"),
            "{example} manifest status"
        );
        assert_schema_valid(&manifest_schema, &manifest_json, example);
        assert!(
            manifest.with_extension("spec.json").exists(),
            "{example} spec sidecar was written"
        );

        let event_count = json_array_len(&manifest_json, "events", example);
        let recorded_count = json_array_len(&manifest_json, "recorded", example);
        assert!(event_count > 0, "{example} recorded no events");
        assert!(
            recorded_count > 0,
            "{example} recorded no actor/resource calls"
        );

        let replay = Command::new(bin)
            .current_dir(&root)
            .arg("replay")
            .arg(&manifest)
            .arg("--spec")
            .arg(&spec)
            .output()
            .expect("replay command executes");
        assert!(
            replay.status.success(),
            "{example} replay failed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&replay.stdout),
            String::from_utf8_lossy(&replay.stderr)
        );
        let replay_stdout = String::from_utf8_lossy(&replay.stdout);
        let expected = format!("replay ok: events={event_count} recorded={recorded_count}");
        assert!(
            replay_stdout.contains(&expected),
            "{example} replay counts should match manifest; expected `{expected}`, got:\n{replay_stdout}"
        );
    }
}

#[test]
fn invalid_examples_fail_validation_with_error_class() {
    let bin = env!("CARGO_BIN_EXE_jesterky");
    let root = workspace_root();

    for (example, error_class) in INVALID_EXAMPLES {
        let output = Command::new(bin)
            .current_dir(&root)
            .arg("validate")
            .arg(root.join("examples").join(example))
            .output()
            .expect("validate command executes");
        assert!(
            !output.status.success(),
            "{example} should fail validation\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains(error_class),
            "{example} should report error class `{error_class}`; got:\n{stdout}"
        );
    }
}

#[test]
fn skipped_examples_are_explicit_and_still_validate() {
    let bin = env!("CARGO_BIN_EXE_jesterky");
    let root = workspace_root();

    for skipped in SKIPPED_EXAMPLES {
        assert!(
            !skipped.reason.trim().is_empty(),
            "{} skip reason must be explicit",
            skipped.file
        );
        let spec = root.join("examples").join(skipped.file);
        let validate = Command::new(bin)
            .current_dir(&root)
            .arg("validate")
            .arg(&spec)
            .output()
            .expect("validate command executes");
        assert!(
            validate.status.success(),
            "{} should remain a valid skipped workflow\nstdout:\n{}\nstderr:\n{}",
            skipped.file,
            String::from_utf8_lossy(&validate.stdout),
            String::from_utf8_lossy(&validate.stderr)
        );
        let run = Command::new(bin)
            .current_dir(&root)
            .arg("run")
            .arg(&spec)
            .arg("--actor")
            .arg("fake")
            .output()
            .expect("run command executes");
        assert!(
            !run.status.success(),
            "{} skip reason says fake actor is rejected, but fake run succeeded",
            skipped.file
        );
        let stderr = String::from_utf8_lossy(&run.stderr);
        assert!(
            stderr.contains("LLM workflow") && stderr.contains("--actor codex"),
            "{} should fail loudly with the fake-incompatible class; got:\n{stderr}",
            skipped.file
        );
    }
}

fn assert_example_inventory_is_explicit(root: &Path) {
    let discovered = discover_workflow_examples(&root.join("examples"));
    let expected = RUNNABLE_EXAMPLES
        .iter()
        .map(|file| (*file).to_string())
        .chain(INVALID_EXAMPLES.iter().map(|(file, _)| (*file).to_string()))
        .chain(
            SKIPPED_EXAMPLES
                .iter()
                .map(|skipped| skipped.file.to_string()),
        )
        .collect::<BTreeSet<_>>();

    assert_eq!(
        discovered, expected,
        "every workflow JSON in examples/ must be runnable, invalid, or explicitly skipped"
    );
}

fn discover_workflow_examples(examples_dir: &Path) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for entry in std::fs::read_dir(examples_dir).expect("examples dir reads") {
        let path = entry.expect("example entry reads").path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .expect("utf-8 example file name")
            .to_string();
        let value = read_json(&path);
        let is_workflow = value.get("name").is_some()
            && value.get("entrypoint").is_some()
            && value.get("nodes").is_some();
        if is_workflow {
            out.insert(file_name);
        } else {
            assert!(
                file_name.ends_with(".schema.json") || file_name == "budgets_episode_scale.json",
                "non-workflow JSON in examples/ must be a schema or named fragment: {file_name}"
            );
        }
    }
    out
}

fn args_for_example(example: &str, fixtures: &ExampleFixtures) -> Option<String> {
    match example {
        "quality_scan_blogs.json" => Some(
            json!({
                "blog_dir": fixtures.blog_dir.display().to_string(),
            })
            .to_string(),
        ),
        "quality_scan_docs.json" => Some(
            json!({
                "docs_dir": fixtures.docs_dir.display().to_string(),
            })
            .to_string(),
        ),
        "smr_reportbench_trace_evaluate.json" => Some(
            json!({
                "trace_dir": "proof/reportbench_traces",
            })
            .to_string(),
        ),
        _ => None,
    }
}

fn example_fixtures() -> ExampleFixtures {
    let temp = tempfile::tempdir().expect("tempdir");
    let blog_dir = temp.path().join("blog");
    let docs_dir = temp.path().join("docs");
    std::fs::create_dir_all(&blog_dir).expect("blog fixture dir");
    std::fs::create_dir_all(&docs_dir).expect("docs fixture dir");
    std::fs::write(
        blog_dir.join("launch.mdx"),
        "---\ntitle: Launch proof\nstatus: published\n---\n\n# Launch proof\n\nEvidence path is committed.\n",
    )
    .expect("blog fixture writes");
    std::fs::write(
        docs_dir.join("quickstart.mdx"),
        "# Quickstart\n\nRun the fake actor path and replay the manifest.\n",
    )
    .expect("docs page writes");
    std::fs::write(
        docs_dir.join("docs.json"),
        r#"{"navigation":{"groups":[{"group":"Guide","pages":["quickstart"]}]}}"#,
    )
    .expect("docs nav writes");
    ExampleFixtures {
        _temp: temp,
        blog_dir,
        docs_dir,
    }
}

fn assert_schema_valid(schema: &JSONSchema, instance: &serde_json::Value, example: &str) {
    let messages = match schema.validate(instance) {
        Ok(()) => Vec::new(),
        Err(errors) => errors.map(|error| error.to_string()).collect::<Vec<_>>(),
    };
    assert!(
        messages.is_empty(),
        "{example} manifest schema validation failed:\n{}",
        messages.join("\n")
    );
}

fn json_array_len(value: &serde_json::Value, key: &str, example: &str) -> usize {
    value
        .get(key)
        .and_then(|item| item.as_array())
        .unwrap_or_else(|| panic!("{example} manifest `{key}` must be an array"))
        .len()
}

fn read_json(path: &Path) -> serde_json::Value {
    let bytes = std::fs::read(path)
        .unwrap_or_else(|err| panic!("failed to read `{}`: {err}", path.display()));
    serde_json::from_slice(&bytes)
        .unwrap_or_else(|err| panic!("failed to parse `{}` as JSON: {err}", path.display()))
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("cli crate has workspace root ancestor")
        .to_path_buf()
}
