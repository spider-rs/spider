//! Pure-Rust embedded scripting — Python + JavaScript without any C/FFI deps.
//!
//! Demonstrates the `scripting` feature on `spider_agent`: the LLM-callable
//! actions `RunPython` and `RunJavaScript` evaluate arbitrary code in
//! embedded `rustpython-vm` / `boa_engine` interpreters. Workers run on
//! dedicated `std::thread`s (no tokio blocking-pool contention, no mutexes,
//! no deadlocks); scripts get a sandboxed tmpdir, optional HTTP via the
//! shared reqwest client, and read-only access to page context.
//!
//! Run with:
//! ```sh
//! cargo run --example scripting --features scripting
//! ```
//!
//! Add `--no-default-features` if you only want the scripting deps and
//! nothing else from `spider_agent`.

use std::time::Duration;

use spider_agent::scripting::{ScriptConfig, ScriptContext, ScriptEngine};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    // Configure the engine. Defaults are conservative — for a real app you'd
    // tune `num_workers` and `max_concurrent` to match your QPS.
    let engine = ScriptEngine::new(ScriptConfig {
        enabled: true,
        num_workers: 2,
        queue_capacity: 16,
        max_concurrent: 2,
        default_timeout: Duration::from_secs(5),
        permit_acquire_timeout: Duration::from_secs(30),
        max_output_bytes: 64 * 1024,
        // `allow_network` enables `agent.fetch(url, opts?)` — set to false to
        // sandbox scripts to local-only computation.
        allow_network: true,
        allow_filesystem: true,
        inject_page_html: true,
        html_max_bytes: 8 * 1024,
    });

    // Simulated page context — in a real run this comes from the chrome page.
    let ctx = ScriptContext {
        url: Some("https://shop.example.com/widget".into()),
        title: Some("Widget Catalog".into()),
        html: Some(
            "<div><span class=price>$1999</span><span class=stock>42 in stock</span></div>".into(),
        ),
        memory_json: None,
    };

    // -----------------------------------------------------------------
    // 1. Python: parse the page with `re` from the frozen stdlib, persist
    //    a small JSON record to the sandboxed tmpdir, read it back.
    // -----------------------------------------------------------------
    println!("== Python ==");
    let py = engine
        .run_python(
            r#"
import json
import re

numbers = [int(n) for n in re.findall(r"\d+", agent.html)]
record = {"url": agent.url, "numbers": numbers}

agent.write_file("record.json", json.dumps(record))
print("wrote:", agent.read_file("record.json"))
print("count:", len(numbers), "sum:", sum(numbers))
"#
            .to_string(),
            ctx.clone(),
            None,
        )
        .await;
    print_result(&py);

    // -----------------------------------------------------------------
    // 2. JavaScript: same workflow, plus return the final expression as the
    //    `value` field on ScriptResult (boa serializes the last expression).
    // -----------------------------------------------------------------
    println!("\n== JavaScript ==");
    let js = engine
        .run_javascript(
            r#"
const numbers = (agent.html.match(/\d+/g) || []).map(Number);
const record = { url: agent.url, numbers };
agent.write_file("record.json", JSON.stringify(record));
console.log("wrote:", agent.read_file("record.json"));
console.log("count:", numbers.length, "sum:", numbers.reduce((a, b) => a + b, 0));
numbers.reduce((a, b) => a + b, 0)  // surfaces as result.value
"#
            .to_string(),
            ctx.clone(),
            None,
        )
        .await;
    print_result(&js);
    if let Some(v) = &js.value {
        println!("result.value = {}", v);
    }

    // -----------------------------------------------------------------
    // 3. Network access via `agent.fetch` — uses the shared reqwest client.
    //    Only fires when `allow_network = true`.
    // -----------------------------------------------------------------
    println!("\n== Python + agent.fetch ==");
    let fetch = engine
        .run_python(
            r#"
resp = agent.fetch("https://example.com")
print("status:", resp["status"], "ok:", resp["ok"], "body_bytes:", len(resp["body"]))
"#
            .to_string(),
            ScriptContext::default(),
            Some(Duration::from_secs(15)),
        )
        .await;
    print_result(&fetch);

    // -----------------------------------------------------------------
    // 4. Cold vs warm: per-worker-thread interpreter cache amortizes init
    //    cost. The second call on the same worker reuses the cached VM and
    //    runs in a small fraction of the cold-start time.
    // -----------------------------------------------------------------
    println!("\n== Cold vs warm Python ==");
    // Use a single-worker engine so both calls hit the same cached interpreter.
    let warm_engine = ScriptEngine::new(ScriptConfig {
        enabled: true,
        num_workers: 1,
        max_concurrent: 1,
        ..ScriptConfig::default()
    });
    let cold = warm_engine
        .run_python("print('cold')".to_string(), ScriptContext::default(), None)
        .await;
    let warm = warm_engine
        .run_python("print('warm')".to_string(), ScriptContext::default(), None)
        .await;
    println!(
        "cold elapsed: {}ms (first call on a fresh worker)",
        cold.elapsed_ms
    );
    if warm.elapsed_ms == 0 {
        println!("warm elapsed: <1ms (subsequent calls reuse the cached interpreter)");
    } else {
        println!(
            "warm elapsed: {}ms  (≈ {}× faster)",
            warm.elapsed_ms,
            cold.elapsed_ms / warm.elapsed_ms
        );
    }

    // -----------------------------------------------------------------
    // 5. Cooperative cancellation: scripts that call `agent.check_interrupted()`
    //    bail with KeyboardInterrupt when the per-call timeout fires.
    // -----------------------------------------------------------------
    println!("\n== Timeout via check_interrupted (engine 1) ==");
    let timed = engine
        .run_python(
            r#"
i = 0
while True:
    i += 1
    if i % 10000 == 0:
        agent.check_interrupted()  # raises KeyboardInterrupt when flagged
"#
            .to_string(),
            ScriptContext::default(),
            Some(Duration::from_millis(150)),
        )
        .await;
    println!(
        "timed_out={} elapsed_ms={} (this is the expected outcome)",
        timed.timed_out, timed.elapsed_ms
    );

    Ok(())
}

fn print_result(r: &spider_agent::scripting::ScriptResult) {
    println!("success: {}  elapsed_ms: {}", r.success, r.elapsed_ms);
    if !r.stdout.is_empty() {
        for line in r.stdout.lines() {
            println!("  stdout │ {line}");
        }
    }
    if !r.stderr.is_empty() {
        for line in r.stderr.lines() {
            println!("  stderr │ {line}");
        }
    }
}
