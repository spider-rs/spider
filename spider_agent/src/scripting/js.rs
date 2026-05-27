//! JavaScript execution via `boa_engine` (pure Rust ECMAScript engine).
//!
//! Each call:
//! 1. Constructs a fresh `Context` (no cross-call state leakage).
//! 2. Stashes per-call state in a thread-local (boa's `NativeFunction::from_copy_closure`
//!    requires `Copy` captures; thread-locals are the idiomatic workaround for sharing
//!    non-`Copy` host state with native functions on a single-threaded worker).
//! 3. Installs `agent.*` host fns + overrides `console.log` to share the output buffer.
//! 4. Parses + evaluates the source; converts any `JsError` into a stderr string.
//!
//! ## Cooperative cancellation
//! Scripts can call `agent.check_interrupted()` in hot loops. Scripts that ignore it
//! are bounded by the worker-pool cap (default 4 workers).

use std::cell::RefCell;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use boa_engine::{
    js_string, object::ObjectInitializer, property::Attribute, Context, JsError, JsNativeError,
    JsResult, JsValue, NativeFunction, Source,
};

use super::sandbox::{agent_fetch_blocking, FetchRequest, OutputBuffer, SandboxedDir};
use super::{Job, ScriptLanguage, ScriptResult};

/// Per-call state shared between the worker thread and the native fns registered with boa.
/// Lives in a thread-local for the duration of one `eval`; cleared on the way out.
///
/// `OutputBuffer` is *not* stored here — it's already thread-local. Storing
/// only the references that vary per call keeps `CallState` lean.
struct CallState {
    interrupt: std::sync::Arc<std::sync::atomic::AtomicBool>,
    runtime: tokio::runtime::Handle,
    sandbox: Option<Arc<SandboxedDir>>,
    allow_network: bool,
}

thread_local! {
    static STATE: RefCell<Option<Arc<CallState>>> = const { RefCell::new(None) };

    /// Per-worker-thread Boa `Context` cache. First call on a worker pays the
    /// Context::default() cost (ECMAScript stdlib registration); subsequent
    /// calls reuse the context for a major warm-call speedup.
    ///
    /// `console` is installed once on Context creation; `agent` is replaced
    /// per call via `global_object().set(...)` so each script sees fresh
    /// context (url/html/etc.) without leaking the prior script's `agent`.
    ///
    /// Module-level mutations to built-in prototypes (e.g. monkey-patching
    /// `Array.prototype.push`) DO persist across calls on the same worker —
    /// accepted tradeoff for warm-call speed in an LLM-tool context.
    ///
    /// Cleaned up explicitly by `worker_loop` via `cleanup_thread_local()`
    /// BEFORE the worker thread exits to avoid thread-destructor-phase issues.
    static JS_CONTEXT: RefCell<Option<Context>> = const { RefCell::new(None) };
}

/// Drop the cached context while the worker thread is still in normal execution.
/// Called by `worker_loop` after `rx.recv()` returns Disconnected.
pub(crate) fn cleanup_thread_local() {
    JS_CONTEXT.with(|cell| {
        *cell.borrow_mut() = None;
    });
}

/// Run `f` with the installed call state. Returns `Err` if state is missing — that
/// would indicate a logic bug (native fn called outside of `run`) but is handled
/// gracefully to honor the no-panic contract.
fn try_with_state<F, R>(f: F) -> Result<R, &'static str>
where
    F: FnOnce(&CallState) -> R,
{
    STATE.with(|cell| {
        let borrow = cell.borrow();
        match borrow.as_ref() {
            Some(state) => Ok(f(state)),
            None => Err("scripting state not installed on worker thread"),
        }
    })
}

fn state_missing_err() -> JsError {
    JsNativeError::error()
        .with_message("scripting state not installed")
        .into()
}

pub(crate) fn run(job: &Job) -> Result<ScriptResult, String> {
    let stdout = OutputBuffer::new();
    let sandbox = if job.config.allow_filesystem {
        Some(Arc::new(
            SandboxedDir::new().map_err(|e| format!("tmpdir: {e}"))?,
        ))
    } else {
        None
    };

    let state = Arc::new(CallState {
        interrupt: job.interrupt.clone(),
        runtime: job.runtime.clone(),
        sandbox: sandbox.clone(),
        allow_network: job.config.allow_network,
    });

    // Install per-call state; ensure it's cleared even on panic/early return.
    STATE.with(|cell| *cell.borrow_mut() = Some(state));

    struct StateGuard;
    impl Drop for StateGuard {
        fn drop(&mut self) {
            STATE.with(|cell| *cell.borrow_mut() = None);
        }
    }
    let _guard = StateGuard;

    // Reuse a per-worker-thread Context to amortize ECMAScript stdlib init.
    // First call on this worker creates the Context + installs `console`;
    // subsequent calls reuse it and refresh the `agent` global per script.
    //
    // All Context-borrowing work (build agent, eval, format error, json-convert)
    // happens inside the `with` closure since the borrow drops on exit.
    let eval_outcome: Result<Option<serde_json::Value>, String> = JS_CONTEXT.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let context: &mut Context = match borrow.as_mut() {
            Some(c) => c,
            None => {
                let mut c = Context::default();
                if let Err(e) = install_console(&mut c) {
                    let msg = format_js_error(&mut c, e);
                    return Err(format!("console setup: {msg}"));
                }
                *borrow = Some(c);
                borrow.as_mut().expect("just set Some above")
            }
        };

        // Build `agent` per call so each script sees fresh url/html/etc.
        // `global_object().set(...)` overwrites cleanly; the prior call's
        // `agent` is dropped here.
        let agent_obj = build_agent_object(context, job, sandbox.as_deref())
            .map_err(|e| format!("agent setup: {}", format_js_error(context, e)))?;
        let global = context.global_object();
        global
            .set(js_string!("agent"), agent_obj, false, context)
            .map_err(|e| format!("install agent: {}", format_js_error(context, e)))?;

        // Parse + evaluate. Convert the result to JSON or render the error
        // BEFORE we release the context borrow.
        match context.eval(Source::from_bytes(&job.code)) {
            Ok(value) => {
                // Boa's `to_json` panics on `undefined` — guard it.
                let value_json = if value.is_undefined() || value.is_null() {
                    None
                } else {
                    value.to_json(context).ok()
                };
                Ok(value_json)
            }
            Err(err) => Err(format_js_error(context, err)),
        }
    });
    let elapsed_ms = job.started_at.elapsed().as_millis() as u64;

    let stdout_str = stdout.drain_to_string();
    let timed_out = job.interrupt.load(Ordering::Relaxed);

    match eval_outcome {
        Ok(value_json) => Ok(ScriptResult {
            language: ScriptLanguage::JavaScript.as_str().to_string(),
            success: true,
            stdout: stdout_str,
            stderr: String::new(),
            value: value_json,
            elapsed_ms,
            timed_out,
        }),
        Err(msg) => Ok(ScriptResult {
            language: ScriptLanguage::JavaScript.as_str().to_string(),
            success: false,
            stdout: stdout_str,
            stderr: msg,
            value: None,
            elapsed_ms,
            timed_out,
        }),
    }
}

fn build_agent_object(
    context: &mut Context,
    job: &Job,
    sandbox: Option<&SandboxedDir>,
) -> JsResult<JsValue> {
    let inject_html = job.config.inject_page_html;
    let html_max = job.config.html_max_bytes;

    let mut init = ObjectInitializer::new(context);

    init.property(
        js_string!("url"),
        job.context
            .url
            .as_deref()
            .map(|s| JsValue::from(js_string!(s)))
            .unwrap_or(JsValue::null()),
        Attribute::READONLY | Attribute::ENUMERABLE,
    );
    init.property(
        js_string!("title"),
        job.context
            .title
            .as_deref()
            .map(|s| JsValue::from(js_string!(s)))
            .unwrap_or(JsValue::null()),
        Attribute::READONLY | Attribute::ENUMERABLE,
    );
    let html_str = if inject_html {
        let raw = job.context.html.as_deref().unwrap_or("");
        let capped = if raw.len() > html_max {
            let mut cut = html_max;
            while cut > 0 && !raw.is_char_boundary(cut) {
                cut -= 1;
            }
            &raw[..cut]
        } else {
            raw
        };
        JsValue::from(js_string!(capped))
    } else {
        JsValue::from(js_string!(""))
    };
    init.property(
        js_string!("html"),
        html_str,
        Attribute::READONLY | Attribute::ENUMERABLE,
    );
    init.property(
        js_string!("memory"),
        job.context
            .memory_json
            .as_deref()
            .map(|s| JsValue::from(js_string!(s)))
            .unwrap_or(JsValue::null()),
        Attribute::READONLY | Attribute::ENUMERABLE,
    );
    init.property(
        js_string!("tmpdir"),
        sandbox
            .map(|sb| JsValue::from(js_string!(sb.root_path().to_string_lossy().as_ref())))
            .unwrap_or(JsValue::null()),
        Attribute::READONLY | Attribute::ENUMERABLE,
    );

    // All native functions are plain `fn` items — Copy, capture-free — and read
    // per-call state from the thread-local stash.
    init.function(NativeFunction::from_fn_ptr(js_log), js_string!("log"), 0);
    init.function(
        NativeFunction::from_fn_ptr(js_check_interrupted),
        js_string!("check_interrupted"),
        0,
    );
    if job.config.allow_network {
        init.function(
            NativeFunction::from_fn_ptr(js_fetch),
            js_string!("fetch"),
            2,
        );
    }
    if sandbox.is_some() {
        init.function(
            NativeFunction::from_fn_ptr(js_read_file),
            js_string!("read_file"),
            1,
        );
        init.function(
            NativeFunction::from_fn_ptr(js_write_file),
            js_string!("write_file"),
            2,
        );
    }

    Ok(init.build().into())
}

fn install_console(context: &mut Context) -> JsResult<()> {
    let console = ObjectInitializer::new(context)
        .function(
            NativeFunction::from_fn_ptr(js_console_log),
            js_string!("log"),
            0,
        )
        .function(
            NativeFunction::from_fn_ptr(js_console_log),
            js_string!("info"),
            0,
        )
        .function(
            NativeFunction::from_fn_ptr(js_console_log),
            js_string!("warn"),
            0,
        )
        .function(
            NativeFunction::from_fn_ptr(js_console_log),
            js_string!("error"),
            0,
        )
        .function(
            NativeFunction::from_fn_ptr(js_console_log),
            js_string!("debug"),
            0,
        )
        .build();
    context.register_global_property(js_string!("console"), console, Attribute::all())?;
    Ok(())
}

// === Native function implementations ===========================================

fn js_log(_this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    write_args_to_stdout(args, ctx)?;
    Ok(JsValue::undefined())
}

fn js_console_log(_this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    write_args_to_stdout(args, ctx)?;
    Ok(JsValue::undefined())
}

fn write_args_to_stdout(args: &[JsValue], ctx: &mut Context) -> JsResult<()> {
    let mut parts = Vec::with_capacity(args.len());
    for a in args {
        parts.push(a.to_string(ctx)?.to_std_string_escaped());
    }
    let buf = OutputBuffer;
    buf.write_str(&parts.join(" "));
    buf.write_str("\n");
    Ok(())
}

fn js_check_interrupted(
    _this: &JsValue,
    _args: &[JsValue],
    _ctx: &mut Context,
) -> JsResult<JsValue> {
    let interrupted =
        try_with_state(|s| s.interrupt.load(Ordering::Relaxed)).map_err(|_| state_missing_err())?;
    if interrupted {
        Err(JsNativeError::error()
            .with_message("script interrupted")
            .into())
    } else {
        Ok(JsValue::undefined())
    }
}

fn js_fetch(_this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let allow_network = try_with_state(|s| s.allow_network).map_err(|_| state_missing_err())?;
    if !allow_network {
        return Err(JsNativeError::error()
            .with_message("agent.fetch is disabled")
            .into());
    }
    let url = args
        .first()
        .ok_or_else(|| JsNativeError::typ().with_message("agent.fetch: missing url"))?
        .to_string(ctx)?
        .to_std_string_escaped();
    let req = if let Some(opts) = args.get(1) {
        let opts_json = opts.to_json(ctx)?;
        serde_json::from_value::<FetchRequest>(opts_json)
            .map_err(|e| JsNativeError::typ().with_message(format!("agent.fetch opts: {e}")))?
    } else {
        FetchRequest::default()
    };
    let resp = try_with_state(|s| agent_fetch_blocking(&s.runtime, &s.interrupt, &url, req))
        .map_err(|_| state_missing_err())?
        .map_err(|e| JsNativeError::error().with_message(e))?;
    let resp_json = serde_json::to_value(&resp)
        .map_err(|e| JsNativeError::error().with_message(format!("serialize: {e}")))?;
    JsValue::from_json(&resp_json, ctx)
}

fn js_read_file(_this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let rel = args
        .first()
        .ok_or_else(|| JsNativeError::typ().with_message("read_file: missing path"))?
        .to_string(ctx)?
        .to_std_string_escaped();
    let content = try_with_state(|s| {
        s.sandbox
            .as_ref()
            .ok_or_else(|| "filesystem disabled".to_string())
            .and_then(|sb| sb.read_file(&rel))
    })
    .map_err(|_| state_missing_err())?
    .map_err(|e| JsNativeError::error().with_message(e))?;
    Ok(JsValue::from(js_string!(content)))
}

fn js_write_file(_this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let rel = args
        .first()
        .ok_or_else(|| JsNativeError::typ().with_message("write_file: missing path"))?
        .to_string(ctx)?
        .to_std_string_escaped();
    let content = args
        .get(1)
        .ok_or_else(|| JsNativeError::typ().with_message("write_file: missing content"))?
        .to_string(ctx)?
        .to_std_string_escaped();
    try_with_state(|s| {
        s.sandbox
            .as_ref()
            .ok_or_else(|| "filesystem disabled".to_string())
            .and_then(|sb| sb.write_file(&rel, &content))
    })
    .map_err(|_| state_missing_err())?
    .map_err(|e| JsNativeError::error().with_message(e))?;
    Ok(JsValue::undefined())
}

fn format_js_error(context: &mut Context, err: JsError) -> String {
    err.to_opaque(context).display().to_string()
}
