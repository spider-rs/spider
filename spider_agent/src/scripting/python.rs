//! Python execution via `rustpython-vm` (pure Rust, no CPython FFI).
//!
//! Each call:
//! 1. Spawns a fresh `Interpreter` (no cross-call state leakage).
//! 2. Injects an `agent` module with `log`/`fetch`/`read_file`/`write_file`/`check_interrupted`
//!    plus the read-only context attributes (`url`, `title`, `html`, `memory`, `tmpdir`).
//! 3. Overrides the builtin `print` so the script's natural output is captured.
//! 4. Compiles + runs the code; converts any `PyException` into a stderr traceback.
//!
//! ## Cooperative cancellation
//! On timeout the async caller flips `job.interrupt`. Scripts that call
//! `agent.check_interrupted()` in tight loops bail with `KeyboardInterrupt`.
//! Scripts that ignore it are bounded by the worker-pool cap.

use std::cell::RefCell;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use rustpython_vm::convert::ToPyObject;
use rustpython_vm::function::FuncArgs;
use rustpython_vm::{Interpreter, PyObjectRef, PyResult, VirtualMachine};

use super::sandbox::{agent_fetch_blocking, FetchRequest, OutputBuffer, SandboxedDir};
use super::{Job, ScriptLanguage, ScriptResult};

const SCRIPT_NAME: &str = "<spider_script>";

thread_local! {
    /// Per-worker-thread interpreter cache. The first call on a worker pays the
    /// frozen-stdlib registration cost (~200 ms on a cold cache); subsequent
    /// calls reuse the interpreter and run in ~5-10 ms.
    ///
    /// Each call still uses a fresh `vm.new_scope_with_builtins()` so user
    /// globals don't leak between scripts. Module-level mutations (e.g. a
    /// script monkey-patching `json.dumps`) DO persist — accepted tradeoff
    /// for warm-call speed in an LLM-tool context.
    ///
    /// Cleaned up explicitly by `worker_loop` via `cleanup_thread_local()`
    /// BEFORE the worker thread exits; dropping `Interpreter` inside the
    /// pthread destructor phase has surfaced SIGTRAPs in stress tests.
    static PY_INTERPRETER: RefCell<Option<Interpreter>> = const { RefCell::new(None) };
}

/// Drop the cached interpreter while the worker thread is still in normal
/// execution. Called by `worker_loop` after `rx.recv()` returns Disconnected.
pub(crate) fn cleanup_thread_local() {
    PY_INTERPRETER.with(|cell| {
        // Replace with None; the prior `Some(Interpreter)` drops here, while
        // we're still on a normally-scheduled thread.
        *cell.borrow_mut() = None;
    });
}

pub(crate) fn run(job: &Job) -> Result<ScriptResult, String> {
    // OutputBuffer is now thread-local-backed; `new()` clears the buffer for this
    // worker thread so leftover bytes from a prior job can't leak in.
    let stdout = OutputBuffer::new();
    let sandbox = if job.config.allow_filesystem {
        Some(Arc::new(
            SandboxedDir::new().map_err(|e| format!("tmpdir: {e}"))?,
        ))
    } else {
        None
    };
    let job_runtime = job.runtime.clone();
    let job_interrupt = job.interrupt.clone();
    let job_client = job.client.clone();
    let job_usage = job.usage.clone();
    let allow_network = job.config.allow_network;
    let inject_html = job.config.inject_page_html;
    let html_max = job.config.html_max_bytes;

    let exec_result = PY_INTERPRETER.with(|cell| -> PyResult<()> {
        let mut borrow = cell.borrow_mut();
        let interp = borrow.get_or_insert_with(|| {
            // First call on this worker — pay the frozen-stdlib registration cost once.
            Interpreter::with_init(Default::default(), |vm| {
                vm.add_frozen(rustpython_pylib::FROZEN_STDLIB);
            })
        });
        interp.enter(|vm| -> PyResult<()> {
            // We need an object that supports attribute writes (`agent.url = ...`)
            // so the script can later do `agent.url` / `agent.log(...)`.
            // `types.SimpleNamespace` would be ideal, but loading any frozen-stdlib
            // module requires the importer to be initialized, which isn't free.
            // Instead we compile a tiny prelude that defines an empty class and
            // instantiates it — zero imports, fully self-contained.
            let prelude_scope = vm.new_scope_with_builtins();
            let prelude_code = vm
                .compile(
                    "class _AgentNS:\n    pass\n_agent = _AgentNS()\n",
                    rustpython_vm::compiler::Mode::Exec,
                    "<spider_prelude>".to_string(),
                )
                .map_err(|err| vm.new_syntax_error(&err, None))?;
            vm.run_code_obj(prelude_code, prelude_scope.clone())?;
            let agent = prelude_scope
                .globals
                .get_item("_agent", vm)
                .map_err(|_| vm.new_runtime_error("prelude did not bind _agent".to_string()))?;

            // `set_attr` wants a Python string for the name, so we materialize one
            // per call. Cheap — RustPython interns short strings.
            let set = |key: &str, value: PyObjectRef| -> PyResult<()> {
                agent.set_attr(&vm.ctx.new_str(key), value, vm)
            };

            // Read-only context
            set(
                "url",
                job.context
                    .url
                    .as_deref()
                    .map(|s| vm.ctx.new_str(s).to_pyobject(vm))
                    .unwrap_or_else(|| vm.ctx.none()),
            )?;
            set(
                "title",
                job.context
                    .title
                    .as_deref()
                    .map(|s| vm.ctx.new_str(s).to_pyobject(vm))
                    .unwrap_or_else(|| vm.ctx.none()),
            )?;
            let html_value = if inject_html {
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
                vm.ctx.new_str(capped).to_pyobject(vm)
            } else {
                vm.ctx.new_str("").to_pyobject(vm)
            };
            set("html", html_value)?;
            set(
                "memory",
                job.context
                    .memory_json
                    .as_deref()
                    .map(|s| vm.ctx.new_str(s).to_pyobject(vm))
                    .unwrap_or_else(|| vm.ctx.none()),
            )?;
            set(
                "tmpdir",
                sandbox
                    .as_ref()
                    .map(|sb| {
                        vm.ctx
                            .new_str(sb.root_path().to_string_lossy().into_owned())
                            .to_pyobject(vm)
                    })
                    .unwrap_or_else(|| vm.ctx.none()),
            )?;

            // agent.log(*args) → output buffer (thread-local; no capture needed)
            {
                let log_fn = vm.new_function(
                    "log",
                    |args: FuncArgs, vm: &VirtualMachine| -> PyResult<PyObjectRef> {
                        let mut parts = Vec::with_capacity(args.args.len());
                        for a in args.args.iter() {
                            parts.push(a.str(vm)?.as_str().to_string());
                        }
                        let buf = OutputBuffer;
                        buf.write_str(&parts.join(" "));
                        buf.write_str("\n");
                        Ok(vm.ctx.none())
                    },
                );
                set("log", log_fn.into())?;
            }

            // agent.check_interrupted() — raise KeyboardInterrupt if flag set
            {
                let interrupt = job_interrupt.clone();
                let fn_ = vm.new_function(
                    "check_interrupted",
                    move |_args: FuncArgs, vm: &VirtualMachine| -> PyResult<PyObjectRef> {
                        if interrupt.load(Ordering::Relaxed) {
                            return Err(vm.new_exception_msg(
                                vm.ctx.exceptions.keyboard_interrupt.to_owned(),
                                "script interrupted".to_string(),
                            ));
                        }
                        Ok(vm.ctx.none())
                    },
                );
                set("check_interrupted", fn_.into())?;
            }

            // agent.fetch(url, opts?) — opts is a dict; response is returned as a dict
            // built directly via the VM context (no `json` module dependency, which
            // would require the frozen stdlib to be loaded).
            //
            // The client carries the engine's proxy/TLS/header configuration
            // (passed in by the chrome dispatcher via `run_python_with_client`).
            if allow_network {
                let runtime = job_runtime.clone();
                let interrupt = job_interrupt.clone();
                let client = job_client.clone();
                let usage = job_usage.clone();
                let fetch_fn = vm.new_function(
                    "fetch",
                    move |args: FuncArgs, vm: &VirtualMachine| -> PyResult<PyObjectRef> {
                        let url_obj = args.args.first().ok_or_else(|| {
                            vm.new_type_error("agent.fetch: missing url".to_string())
                        })?;
                        let url = url_obj.str(vm)?.as_str().to_string();
                        let req = if let Some(opts) = args.args.get(1) {
                            fetch_req_from_pyobj(vm, opts)?
                        } else {
                            FetchRequest::default()
                        };
                        let resp =
                            agent_fetch_blocking(&client, &runtime, &interrupt, &usage, &url, req)
                                .map_err(|e| vm.new_runtime_error(e))?;
                        fetch_resp_to_pydict(vm, &resp)
                    },
                );
                set("fetch", fetch_fn.into())?;
            }

            // agent.read_file / agent.write_file
            if let Some(sb) = sandbox.as_ref() {
                let sb_read = sb.clone();
                let read_fn = vm.new_function(
                    "read_file",
                    move |args: FuncArgs, vm: &VirtualMachine| -> PyResult<PyObjectRef> {
                        let rel = args
                            .args
                            .first()
                            .ok_or_else(|| {
                                vm.new_type_error("read_file: missing path".to_string())
                            })?
                            .str(vm)?
                            .as_str()
                            .to_string();
                        let content = sb_read.read_file(&rel).map_err(|e| vm.new_os_error(e))?;
                        Ok(vm.ctx.new_str(content).to_pyobject(vm))
                    },
                );
                set("read_file", read_fn.into())?;

                let sb_write = sb.clone();
                let write_fn = vm.new_function(
                    "write_file",
                    move |args: FuncArgs, vm: &VirtualMachine| -> PyResult<PyObjectRef> {
                        let rel = args
                            .args
                            .first()
                            .ok_or_else(|| {
                                vm.new_type_error("write_file: missing path".to_string())
                            })?
                            .str(vm)?
                            .as_str()
                            .to_string();
                        let content = args
                            .args
                            .get(1)
                            .ok_or_else(|| {
                                vm.new_type_error("write_file: missing content".to_string())
                            })?
                            .str(vm)?
                            .as_str()
                            .to_string();
                        sb_write
                            .write_file(&rel, &content)
                            .map_err(|e| vm.new_os_error(e))?;
                        Ok(vm.ctx.none())
                    },
                );
                set("write_file", write_fn.into())?;
            }

            // Override print → buffer (thread-local; no capture)
            let print_fn = vm.new_function(
                "print",
                |args: FuncArgs, vm: &VirtualMachine| -> PyResult<PyObjectRef> {
                    let sep = args
                        .kwargs
                        .get("sep")
                        .map(|v| v.str(vm))
                        .transpose()?
                        .map(|s| s.as_str().to_string())
                        .unwrap_or_else(|| " ".to_string());
                    let end = args
                        .kwargs
                        .get("end")
                        .map(|v| v.str(vm))
                        .transpose()?
                        .map(|s| s.as_str().to_string())
                        .unwrap_or_else(|| "\n".to_string());
                    let mut parts = Vec::with_capacity(args.args.len());
                    for a in args.args.iter() {
                        parts.push(a.str(vm)?.as_str().to_string());
                    }
                    let buf = OutputBuffer;
                    buf.write_str(&parts.join(&sep));
                    buf.write_str(&end);
                    Ok(vm.ctx.none())
                },
            );

            // Set up the scope with our injections.
            let scope = vm.new_scope_with_builtins();
            scope.globals.set_item("agent", agent, vm)?;
            scope.globals.set_item("print", print_fn.into(), vm)?;

            // Compile + run.
            let code_obj = vm
                .compile(
                    &job.code,
                    rustpython_vm::compiler::Mode::Exec,
                    SCRIPT_NAME.to_string(),
                )
                .map_err(|err| vm.new_syntax_error(&err, Some(&job.code)))?;
            vm.run_code_obj(code_obj, scope)?;
            Ok(())
        })
    });

    let elapsed_ms = job.started_at.elapsed().as_millis() as u64;
    let timed_out = job.interrupt.load(Ordering::Relaxed);
    let stdout_str = stdout.drain_to_string();
    // Python doesn't write to stderr during eval — it surfaces tracebacks via
    // the returned `PyException`, which we render below.
    let mut stderr_str = String::new();

    match exec_result {
        Ok(()) => Ok(ScriptResult {
            language: ScriptLanguage::Python.as_str().to_string(),
            success: true,
            stdout: stdout_str,
            stderr: stderr_str,
            value: None,
            elapsed_ms,
            timed_out,
        }),
        Err(exc) => {
            // Render the traceback by re-entering the cached interpreter.
            PY_INTERPRETER.with(|cell| {
                if let Some(interp) = cell.borrow().as_ref() {
                    interp.enter(|vm| {
                        let mut tb = String::new();
                        vm.write_exception(&mut tb, &exc).ok();
                        if !tb.is_empty() {
                            if !stderr_str.is_empty() && !stderr_str.ends_with('\n') {
                                stderr_str.push('\n');
                            }
                            stderr_str.push_str(&tb);
                        }
                    });
                }
            });
            Ok(ScriptResult {
                language: ScriptLanguage::Python.as_str().to_string(),
                success: false,
                stdout: stdout_str,
                stderr: stderr_str,
                value: None,
                elapsed_ms,
                timed_out,
            })
        }
    }
}

/// Pull the supported fields out of a Python dict-shaped argument into a `FetchRequest`.
///
/// We don't depend on the `json` stdlib module — instead we walk the dict directly
/// via PyDict's iter API. Unknown keys are silently ignored (consistent with serde
/// default handling on the `FetchRequest` deserializer).
fn fetch_req_from_pyobj(vm: &VirtualMachine, opts: &PyObjectRef) -> PyResult<FetchRequest> {
    use rustpython_vm::builtins::PyDict;
    let dict = opts
        .clone()
        .downcast::<PyDict>()
        .map_err(|_| vm.new_type_error("agent.fetch opts must be a dict".to_string()))?;

    let mut req = FetchRequest::default();
    for (k, v) in dict {
        let key = k.str(vm)?.as_str().to_string();
        match key.as_str() {
            "method" => req.method = Some(v.str(vm)?.as_str().to_string()),
            "body" => req.body = Some(v.str(vm)?.as_str().to_string()),
            "timeout_ms" => {
                let n = v
                    .try_int(vm)
                    .map_err(|_| vm.new_value_error("timeout_ms must be int".to_string()))?;
                req.timeout_ms = n.try_to_primitive::<u64>(vm).ok();
            }
            "headers" => {
                let hdrs = v
                    .clone()
                    .downcast::<PyDict>()
                    .map_err(|_| vm.new_type_error("headers must be a dict".to_string()))?;
                let mut map = std::collections::HashMap::new();
                for (hk, hv) in hdrs {
                    map.insert(
                        hk.str(vm)?.as_str().to_string(),
                        hv.str(vm)?.as_str().to_string(),
                    );
                }
                req.headers = Some(map);
            }
            _ => {} // ignore unknown keys
        }
    }
    Ok(req)
}

/// Build a Python dict mirroring `FetchResponse`, no `json` module needed.
fn fetch_resp_to_pydict(
    vm: &VirtualMachine,
    resp: &super::sandbox::FetchResponse,
) -> PyResult<PyObjectRef> {
    let dict = vm.ctx.new_dict();
    dict.set_item("status", vm.ctx.new_int(resp.status).into(), vm)?;
    dict.set_item("ok", vm.ctx.new_bool(resp.ok).into(), vm)?;
    dict.set_item("url", vm.ctx.new_str(resp.url.clone()).into(), vm)?;
    dict.set_item("body", vm.ctx.new_str(resp.body.clone()).into(), vm)?;
    dict.set_item("truncated", vm.ctx.new_bool(resp.truncated).into(), vm)?;
    let headers = vm.ctx.new_dict();
    for (k, v) in resp.headers.iter() {
        headers.set_item(k.as_str(), vm.ctx.new_str(v.clone()).into(), vm)?;
    }
    dict.set_item("headers", headers.into(), vm)?;
    Ok(dict.into())
}
