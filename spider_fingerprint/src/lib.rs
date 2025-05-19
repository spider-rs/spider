include!(concat!(env!("OUT_DIR"), "/chrome_versions.rs"));

/// Builder types.
pub mod configs;
/// Custom static profiles.
pub mod profiles;
/// GPU spoofs.
pub mod spoof_gpu;
/// Spoof mouse-movement.
pub mod spoof_mouse_movement;
/// Referer headers.
pub mod spoof_refererer;
/// User agent.
pub mod spoof_user_agent;
/// Spoof viewport.
pub mod spoof_viewport;
/// WebGL spoofs.
pub mod spoof_webgl;
/// Generic spoofs.
pub mod spoofs;

use profiles::{
    gpu::select_random_gpu_profile,
    gpu_limits::{build_gpu_request_adapter_script_from_limits, GpuLimits},
};
use spoof_gpu::build_gpu_spoof_script_wgsl;

use crate::configs::{AgentOs, Tier};

lazy_static::lazy_static! {
    /// The latest Chrome version, configurable via the `CHROME_VERSION` env variable.
    pub static ref BASE_CHROME_VERSION: u32 = std::env::var("CHROME_VERSION")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(136);

    pub(crate) static ref LATEST_FULL_VERSION_FULL: &'static str = CHROME_VERSIONS_BY_MAJOR
        .get("latest")
        .and_then(|arr| arr.first().copied())
        .unwrap_or("136.0.7103.114");

    /// The latest Chrome not a brand version, configurable via the `CHROME_NOT_A_BRAND_VERSION` env variable.
    pub static ref CHROME_NOT_A_BRAND_VERSION: String = std::env::var("CHROME_NOT_A_BRAND_VERSION")
        .ok()
        .and_then(|v| if v.is_empty() { None } else { Some(v) })
        .unwrap_or("99.0.0.0".into());
}

/// Generate the initial stealth script to send in one command.
pub fn build_stealth_script(tier: Tier, os: AgentOs) -> String {
    use crate::spoofs::{
        spoof_hardware_concurrency, unified_worker_override, HIDE_CHROME, HIDE_CONSOLE,
        HIDE_WEBDRIVER, NAVIGATOR_SCRIPT, PLUGIN_AND_MIMETYPE_SPOOF,
    };

    let gpu_profile = select_random_gpu_profile(os);
    let spoof_gpu = build_gpu_spoof_script_wgsl(gpu_profile.canvas_format);
    let spoof_webgl = unified_worker_override(
        gpu_profile.hardware_concurrency,
        gpu_profile.webgl_vendor,
        gpu_profile.webgl_renderer,
    );
    let spoof_concurrency = spoof_hardware_concurrency(gpu_profile.hardware_concurrency);

    let mut gpu_limit = GpuLimits::for_os(os);

    if gpu_profile.webgl_renderer
        != "ANGLE (Apple, ANGLE Metal Renderer: Apple M1, Unspecified Version)"
    {
        gpu_limit = gpu_limit.with_variation(gpu_profile.hardware_concurrency);
    }

    let spoof_gpu_adapter = build_gpu_request_adapter_script_from_limits(
        gpu_profile.webgpu_vendor,
        gpu_profile.webgpu_architecture,
        "",
        "",
        &gpu_limit,
    );

    if tier == Tier::Basic {
        format!(
            r#"{HIDE_CHROME}{HIDE_CONSOLE}{spoof_webgl}{spoof_gpu_adapter}{NAVIGATOR_SCRIPT}{PLUGIN_AND_MIMETYPE_SPOOF}"#
        )
    } else if tier == Tier::BasicWithConsole {
        format!(
            r#"{HIDE_CHROME}{spoof_webgl}{spoof_gpu_adapter}{NAVIGATOR_SCRIPT}{PLUGIN_AND_MIMETYPE_SPOOF}"#
        )
    } else if tier == Tier::BasicNoWebgl {
        format!(
            r#"{HIDE_CHROME}{HIDE_CONSOLE}{spoof_concurrency}{NAVIGATOR_SCRIPT}{PLUGIN_AND_MIMETYPE_SPOOF}"#
        )
    } else if tier == Tier::Mid {
        format!(
            r#"{HIDE_CHROME}{HIDE_CONSOLE}{spoof_webgl}{spoof_gpu_adapter}{HIDE_WEBDRIVER}{NAVIGATOR_SCRIPT}{PLUGIN_AND_MIMETYPE_SPOOF}"#
        )
    } else if tier == Tier::Full {
        format!("{HIDE_CHROME}{HIDE_CONSOLE}{spoof_webgl}{spoof_gpu_adapter}{HIDE_WEBDRIVER}{NAVIGATOR_SCRIPT}{PLUGIN_AND_MIMETYPE_SPOOF}{spoof_gpu}")
    } else {
        Default::default()
    }
}

/// Generate the hide plugins script.
pub fn generate_hide_plugins() -> String {
    format!(
        "{}{}",
        crate::spoofs::NAVIGATOR_SCRIPT,
        crate::spoofs::PLUGIN_AND_MIMETYPE_SPOOF
    )
}

/// Simple function to wrap the eval script safely.
pub fn wrap_eval_script(source: &str) -> String {
    format!(r#"(()=>{{{}}})();"#, source)
}
