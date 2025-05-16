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

use crate::configs::{AgentOs, Tier};

lazy_static::lazy_static! {
    /// The latest Chrome version, configurable via the `CHROME_VERSION` env variable.
    pub static ref BASE_CHROME_VERSION: u32 = std::env::var("CHROME_VERSION")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(136);
    /// The latest Chrome not a brand version, configurable via the `CHROME_NOT_A_BRAND_VERSION` env variable.
    pub static ref CHROME_NOT_A_BRAND_VERSION: String = std::env::var("CHROME_NOT_A_BRAND_VERSION")
        .ok()
        .and_then(|v| if v.is_empty() { None } else { Some(v) })
        .unwrap_or("99.0.0.0".into());
}

/// Generate the initial stealth script to send in one command.
pub fn build_stealth_script(tier: Tier, os: AgentOs) -> String {
    use crate::spoofs::{
        GPU_REQUEST_ADAPTER, GPU_REQUEST_ADAPTER_MAC, GPU_SPOOF_SCRIPT, GPU_SPOOF_SCRIPT_MAC,
        HIDE_CHROME, HIDE_WEBDRIVER, HIDE_WEBGL, HIDE_WEBGL_MAC, NAVIGATOR_SCRIPT,
        PLUGIN_AND_MIMETYPE_SPOOF,
    };

    let mac_spoof = os == AgentOs::Mac;

    let spoof_gpu = if mac_spoof {
        GPU_SPOOF_SCRIPT_MAC
    } else {
        GPU_SPOOF_SCRIPT
    };

    let spoof_webgl = if mac_spoof {
        HIDE_WEBGL_MAC
    } else {
        HIDE_WEBGL
    };

    let spoof_gpu_adapter = if mac_spoof {
        GPU_REQUEST_ADAPTER_MAC
    } else {
        GPU_REQUEST_ADAPTER
    };

    if tier == Tier::Basic {
        format!(
            r#"{HIDE_CHROME};{spoof_webgl};{spoof_gpu_adapter};{NAVIGATOR_SCRIPT};{PLUGIN_AND_MIMETYPE_SPOOF};"#
        )
    } else if tier == Tier::BasicNoWebgl {
        format!(r#"{HIDE_CHROME};{NAVIGATOR_SCRIPT};{PLUGIN_AND_MIMETYPE_SPOOF};"#)
    } else if tier == Tier::Mid {
        format!(
            r#"{HIDE_CHROME};{spoof_webgl};{spoof_gpu_adapter};{HIDE_WEBDRIVER};{NAVIGATOR_SCRIPT};{PLUGIN_AND_MIMETYPE_SPOOF};"#
        )
    } else if tier == Tier::Full {
        format!("{HIDE_CHROME};{spoof_webgl};{spoof_gpu_adapter};{HIDE_WEBDRIVER};{NAVIGATOR_SCRIPT};{PLUGIN_AND_MIMETYPE_SPOOF};{spoof_gpu};")
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
