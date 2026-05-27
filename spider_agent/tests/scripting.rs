//! Integration tests for the pure-Rust scripting engine.
//!
//! These don't touch the network unless `RUN_LIVE_TESTS=1` is set.

#![cfg(feature = "scripting")]

use spider_agent::scripting::{ScriptConfig, ScriptContext, ScriptEngine};
use std::time::Duration;

fn enabled_engine() -> ScriptEngine {
    ScriptEngine::new(ScriptConfig {
        enabled: true,
        num_workers: 2,
        queue_capacity: 8,
        max_concurrent: 2,
        default_timeout: Duration::from_secs(5),
        permit_acquire_timeout: Duration::from_secs(10),
        max_output_bytes: 64 * 1024,
        allow_network: false,
        allow_filesystem: true,
        inject_page_html: true,
        html_max_bytes: 4096,
    })
}

#[tokio::test]
async fn python_hello_world() {
    let engine = enabled_engine();
    let result = engine
        .run_python(
            "print('hello from python')".to_string(),
            ScriptContext::default(),
            None,
        )
        .await;
    assert!(result.success, "python script failed: {}", result.stderr);
    assert_eq!(result.stdout.trim(), "hello from python");
    assert_eq!(result.language, "python");
    assert!(!result.timed_out);
}

#[tokio::test]
async fn python_agent_globals() {
    let engine = enabled_engine();
    let result = engine
        .run_python(
            r#"
print(agent.url)
print(agent.title)
print(agent.html[:20])
agent.log("from log")
"#
            .to_string(),
            ScriptContext {
                url: Some("https://example.com".into()),
                title: Some("Example".into()),
                html: Some("<html><body>hi</body></html>".into()),
                memory_json: None,
            },
            None,
        )
        .await;
    assert!(result.success, "stderr: {}", result.stderr);
    let stdout = result.stdout;
    assert!(stdout.contains("https://example.com"), "stdout: {stdout}");
    assert!(stdout.contains("Example"), "stdout: {stdout}");
    assert!(stdout.contains("<html><body>hi"), "stdout: {stdout}");
    assert!(stdout.contains("from log"), "stdout: {stdout}");
}

#[tokio::test]
async fn python_imports_frozen_stdlib() {
    // The frozen stdlib (`rustpython-pylib`) covers pure-Python modules whose
    // dependencies are themselves pure Python. Modules that bottom out in
    // native C/Rust accelerators (`math`, `struct`, `_socket`, ...) live in
    // the separate `rustpython-stdlib` crate and are deliberately omitted to
    // keep the binary small and the sandbox surface minimal. `json` and `re`
    // are the canonical "complex pure-Python" modules and are sufficient to
    // prove the frozen importer is wired up.
    let engine = enabled_engine();
    let result = engine
        .run_python(
            r#"
import json
import re

data = {"x": 1, "y": [2, 3]}
print(json.dumps(data))
print(re.findall(r"\d+", "abc 12 de 34"))
"#
            .to_string(),
            ScriptContext::default(),
            None,
        )
        .await;
    assert!(result.success, "stderr: {}", result.stderr);
    let out = result.stdout;
    assert!(out.contains(r#"{"x": 1, "y": [2, 3]}"#), "json: {out}");
    assert!(out.contains("12"), "re: {out}");
    assert!(out.contains("34"), "re: {out}");
}

#[tokio::test]
async fn python_syntax_error_surfaces() {
    let engine = enabled_engine();
    let result = engine
        .run_python("def broken(:".to_string(), ScriptContext::default(), None)
        .await;
    assert!(!result.success);
    assert!(!result.stderr.is_empty());
}

#[tokio::test]
async fn python_sandboxed_filesystem() {
    let engine = enabled_engine();
    let result = engine
        .run_python(
            r#"
agent.write_file("note.txt", "rust + python")
content = agent.read_file("note.txt")
print(content)
"#
            .to_string(),
            ScriptContext::default(),
            None,
        )
        .await;
    assert!(result.success, "stderr: {}", result.stderr);
    assert_eq!(result.stdout.trim(), "rust + python");
}

#[tokio::test]
async fn python_filesystem_escape_rejected() {
    let engine = enabled_engine();
    let result = engine
        .run_python(
            r#"
try:
    agent.read_file("../../../etc/passwd")
    print("ESCAPED")
except OSError as e:
    print("blocked:", str(e)[:20])
"#
            .to_string(),
            ScriptContext::default(),
            None,
        )
        .await;
    assert!(result.success, "stderr: {}", result.stderr);
    assert!(
        result.stdout.contains("blocked"),
        "expected sandbox to reject escape, got: {}",
        result.stdout
    );
    assert!(!result.stdout.contains("ESCAPED"));
}

#[tokio::test]
async fn javascript_hello_world() {
    let engine = enabled_engine();
    let result = engine
        .run_javascript(
            "console.log('hello from js')".to_string(),
            ScriptContext::default(),
            None,
        )
        .await;
    assert!(result.success, "js script failed: {}", result.stderr);
    assert_eq!(result.stdout.trim(), "hello from js");
    assert_eq!(result.language, "javascript");
    assert!(!result.timed_out);
}

#[tokio::test]
async fn javascript_agent_globals_and_log() {
    let engine = enabled_engine();
    let result = engine
        .run_javascript(
            r#"
agent.log("url:", agent.url);
agent.log("title:", agent.title);
agent.log("html_len:", agent.html.length);
"#
            .to_string(),
            ScriptContext {
                url: Some("https://example.com".into()),
                title: Some("Example".into()),
                html: Some("<html></html>".into()),
                memory_json: None,
            },
            None,
        )
        .await;
    assert!(result.success, "stderr: {}", result.stderr);
    assert!(result.stdout.contains("https://example.com"));
    assert!(result.stdout.contains("Example"));
    assert!(result.stdout.contains("html_len: 13"));
}

#[tokio::test]
async fn javascript_returns_value() {
    let engine = enabled_engine();
    let result = engine
        .run_javascript("1 + 2 + 3".to_string(), ScriptContext::default(), None)
        .await;
    assert!(result.success, "stderr: {}", result.stderr);
    assert_eq!(result.value, Some(serde_json::json!(6)));
}

#[tokio::test]
async fn javascript_sandboxed_filesystem() {
    let engine = enabled_engine();
    let result = engine
        .run_javascript(
            r#"
agent.write_file("note.txt", "rust + js");
const content = agent.read_file("note.txt");
console.log(content);
"#
            .to_string(),
            ScriptContext::default(),
            None,
        )
        .await;
    assert!(result.success, "stderr: {}", result.stderr);
    assert_eq!(result.stdout.trim(), "rust + js");
}

#[tokio::test]
async fn disabled_engine_rejects_calls() {
    let cfg = ScriptConfig {
        enabled: false,
        ..ScriptConfig::default()
    };
    let engine = ScriptEngine::new(cfg);
    let result = engine
        .run_python("print('hi')".to_string(), ScriptContext::default(), None)
        .await;
    assert!(!result.success);
    assert!(result.stderr.contains("disabled"));
}

#[tokio::test]
async fn javascript_runtime_error_surfaces() {
    let engine = enabled_engine();
    let result = engine
        .run_javascript(
            "throw new Error('boom')".to_string(),
            ScriptContext::default(),
            None,
        )
        .await;
    assert!(!result.success);
    assert!(result.stderr.contains("boom"), "stderr: {}", result.stderr);
}

#[tokio::test]
async fn timeout_marks_result() {
    let engine = ScriptEngine::new(ScriptConfig {
        enabled: true,
        num_workers: 1,
        queue_capacity: 4,
        max_concurrent: 1,
        default_timeout: Duration::from_millis(150),
        permit_acquire_timeout: Duration::from_secs(5),
        max_output_bytes: 64 * 1024,
        allow_network: false,
        allow_filesystem: true,
        inject_page_html: false,
        html_max_bytes: 0,
    });
    // A loop that doesn't honor agent.check_interrupted — the caller times out
    // and returns; the worker thread continues but is bounded by `num_workers`.
    let result = engine
        .run_javascript(
            "while (true) {}".to_string(),
            ScriptContext::default(),
            None,
        )
        .await;
    assert!(result.timed_out, "expected timed_out=true, got {result:?}");
}

// ===== End-to-end tests — exercise the engine through the public agent API =======

#[tokio::test]
async fn e2e_engine_accepts_script_engine() {
    // Wire a ScriptEngine into RemoteMultimodalEngine via the builder method,
    // confirming the cfg-gated field is present and stays attached through
    // `clone_with_cfg` (the path used when forking config per URL).
    use spider_agent::{RemoteMultimodalConfig, RemoteMultimodalEngine};

    let script = ScriptEngine::new(ScriptConfig {
        enabled: true,
        ..ScriptConfig::default()
    });
    let engine = RemoteMultimodalEngine::new("https://api.example.com", "test-model", None)
        .with_script_engine(Some(script));
    assert!(
        engine.script_engine.is_some(),
        "with_script_engine should populate the field"
    );
    assert!(engine.script_engine.as_ref().unwrap().is_enabled());

    // clone_with_cfg should propagate the script engine, otherwise scripts
    // dispatched after a config fork would silently no-op.
    let cloned = engine.clone_with_cfg(RemoteMultimodalConfig::default());
    assert!(
        cloned.script_engine.is_some(),
        "clone_with_cfg must propagate script_engine"
    );
}

#[tokio::test]
async fn e2e_python_full_workflow() {
    // Simulates what the LLM-driven dispatcher does end-to-end:
    //   1. Page context (URL/title/HTML) is captured.
    //   2. Script is dispatched with a tight per-call timeout.
    //   3. Script reads agent.* globals, calls agent.write_file/read_file,
    //      uses frozen stdlib (re/json), then prints a structured result.
    //   4. Caller receives ScriptResult with stdout the LLM can read.
    let engine = enabled_engine();
    let result = engine
        .run_python(
            r#"
import json
import re

# Extract digits from the "page" using the frozen stdlib `re`.
numbers = [int(n) for n in re.findall(r"\d+", agent.html)]

# Persist a small artifact to the sandboxed tmpdir.
agent.write_file("digits.json", json.dumps({"url": agent.url, "found": numbers}))

# Read it back to verify round-trip.
record = json.loads(agent.read_file("digits.json"))
print("url:", record["url"])
print("count:", len(record["found"]))
print("sum:", sum(record["found"]))
"#
            .to_string(),
            ScriptContext {
                url: Some("https://shop.example.com/widget".into()),
                title: Some("Widget Listing".into()),
                html: Some("<p>Price 1999, Stock 42, SKU 7</p>".into()),
                memory_json: None,
            },
            Some(Duration::from_secs(10)),
        )
        .await;
    assert!(result.success, "stderr: {}", result.stderr);
    assert!(!result.timed_out);
    let out = result.stdout;
    assert!(
        out.contains("url: https://shop.example.com/widget"),
        "stdout: {out}"
    );
    assert!(out.contains("count: 3"), "stdout: {out}");
    assert!(out.contains("sum: 2048"), "stdout: {out}");
}

#[tokio::test]
async fn e2e_javascript_full_workflow() {
    // Mirror of e2e_python_full_workflow on the JS side: read agent.*, do work,
    // persist to sandbox, read back, return a value the engine surfaces as JSON.
    let engine = enabled_engine();
    let result = engine
        .run_javascript(
            r#"
const html = agent.html;
const matches = html.match(/\d+/g) || [];
const numbers = matches.map(Number);
agent.write_file("digits.json", JSON.stringify({ url: agent.url, found: numbers }));
const record = JSON.parse(agent.read_file("digits.json"));
console.log("url:", record.url);
console.log("count:", record.found.length);
console.log("sum:", record.found.reduce((a, b) => a + b, 0));
record.found.reduce((a, b) => a + b, 0)
"#
            .to_string(),
            ScriptContext {
                url: Some("https://shop.example.com/widget".into()),
                title: Some("Widget Listing".into()),
                html: Some("<p>Price 1999, Stock 42, SKU 7</p>".into()),
                memory_json: None,
            },
            Some(Duration::from_secs(10)),
        )
        .await;
    assert!(result.success, "stderr: {}", result.stderr);
    let out = result.stdout;
    assert!(out.contains("url: https://shop.example.com/widget"));
    assert!(out.contains("count: 3"));
    assert!(out.contains("sum: 2048"));
    // The final expression value is also captured.
    assert_eq!(result.value, Some(serde_json::json!(2048)));
}

#[tokio::test]
async fn e2e_concurrent_scripts_serialized_by_workers() {
    // Two parallel callers dispatch overlapping scripts through the same engine.
    // Worker pool has 1 worker + max_concurrent=1 → the second call waits on the
    // semaphore. Neither call deadlocks; both eventually complete with success.
    let engine = std::sync::Arc::new(ScriptEngine::new(ScriptConfig {
        enabled: true,
        num_workers: 1,
        queue_capacity: 4,
        max_concurrent: 1,
        default_timeout: Duration::from_secs(5),
        permit_acquire_timeout: Duration::from_secs(10),
        max_output_bytes: 64 * 1024,
        allow_network: false,
        allow_filesystem: true,
        inject_page_html: false,
        html_max_bytes: 0,
    }));

    let e1 = engine.clone();
    let e2 = engine.clone();
    let h1 = tokio::spawn(async move {
        e1.run_python(
            "agent.log('one')".to_string(),
            ScriptContext::default(),
            None,
        )
        .await
    });
    let h2 = tokio::spawn(async move {
        e2.run_javascript(
            "agent.log('two')".to_string(),
            ScriptContext::default(),
            None,
        )
        .await
    });
    let (r1, r2) = tokio::join!(h1, h2);
    let r1 = r1.unwrap();
    let r2 = r2.unwrap();
    assert!(r1.success && r1.stdout.contains("one"), "r1: {r1:?}");
    assert!(r2.success && r2.stdout.contains("two"), "r2: {r2:?}");
}

#[tokio::test]
async fn e2e_action_type_round_trips_via_serde() {
    // Confirms ActionType::RunPython and RunJavaScript serialize/deserialize
    // — important because the LLM emits actions as JSON.
    use spider_agent::ActionType;

    let original = ActionType::RunPython {
        code: "print('hi')".to_string(),
        timeout_ms: Some(2000),
    };
    let json = serde_json::to_string(&original).unwrap();
    let back: ActionType = serde_json::from_str(&json).unwrap();
    assert_eq!(original, back);

    let original = ActionType::RunJavaScript {
        code: "1+1".to_string(),
        timeout_ms: None,
    };
    let json = serde_json::to_string(&original).unwrap();
    let back: ActionType = serde_json::from_str(&json).unwrap();
    assert_eq!(original, back);
}

// ===== Live tests — require RUN_LIVE_TESTS=1 + network ============================

fn live() -> bool {
    std::env::var("RUN_LIVE_TESTS").ok().as_deref() == Some("1")
}

#[tokio::test]
async fn python_agent_fetch_live() {
    if !live() {
        eprintln!("skipping live test (set RUN_LIVE_TESTS=1)");
        return;
    }
    let engine = ScriptEngine::new(ScriptConfig {
        enabled: true,
        allow_network: true,
        default_timeout: Duration::from_secs(30),
        ..ScriptConfig::default()
    });
    let result = engine
        .run_python(
            r#"
resp = agent.fetch("https://example.com")
print("status:", resp["status"])
print("ok:", resp["ok"])
print("body_len:", len(resp["body"]))
"#
            .to_string(),
            ScriptContext::default(),
            None,
        )
        .await;
    assert!(result.success, "stderr: {}", result.stderr);
    assert!(result.stdout.contains("status: 200"));
    assert!(result.stdout.contains("ok: True"));
}

#[tokio::test]
async fn javascript_agent_fetch_live() {
    if !live() {
        eprintln!("skipping live test (set RUN_LIVE_TESTS=1)");
        return;
    }
    let engine = ScriptEngine::new(ScriptConfig {
        enabled: true,
        allow_network: true,
        default_timeout: Duration::from_secs(30),
        ..ScriptConfig::default()
    });
    let result = engine
        .run_javascript(
            r#"
const resp = agent.fetch("https://example.com");
console.log("status:", resp.status, "ok:", resp.ok, "body_len:", resp.body.length);
"#
            .to_string(),
            ScriptContext::default(),
            None,
        )
        .await;
    assert!(result.success, "stderr: {}", result.stderr);
    assert!(result.stdout.contains("status: 200"));
    assert!(result.stdout.contains("ok: true"));
}
