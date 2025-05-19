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

/// The fingerprint type to use.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Fingerprint {
    /// Basic finterprint that includes webgl and gpu attempt spoof.
    Basic,
    /// Basic fingerprint that does not spoof the gpu. Used for real gpu based headless instances.
    /// This will bypass the most advanced anti-bots without the speed reduction of a virtual display.
    NativeGPU,
    /// None - no fingerprint and use the default browser fingerprinting. This may be a good option to use at times.
    #[default]
    None,
}

impl Fingerprint {
    /// Fingerprint should be used.
    pub fn valid(&self) -> bool {
        match &self {
            Self::Basic | Self::NativeGPU => true,
            _ => false,
        }
    }
}
/// Configuration options for browser fingerprinting and automation.
#[derive(Default, Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct EmulationConfiguration {
    /// Enables stealth mode to help avoid detection by anti-bot mechanisms.
    pub tier: configs::Tier,
    /// If enabled, will auto-dismiss browser popups and dialogs.
    pub dismiss_dialogs: bool,
    /// The detailed fingerprint configuration for the browser session.
    pub fingerprint: Fingerprint,
    /// The agent os.
    pub agent_os: AgentOs,
    /// Is this firefox?
    pub firefox_agent: bool,
}

/// Setup the emulation defaults.
impl EmulationConfiguration {
    /// Setup the defaults.
    pub fn setup_defaults(user_agent: &str) -> EmulationConfiguration {
        let mut firefox_agent = false;

        let agent_os = {
            let mut agent_os = AgentOs::Linux;
            if user_agent.contains("Chrome") {
                if user_agent.contains("Linux") {
                    agent_os = AgentOs::Linux;
                } else if user_agent.contains("Mac") {
                    agent_os = AgentOs::Mac;
                } else if user_agent.contains("Windows") {
                    agent_os = AgentOs::Windows;
                } else if user_agent.contains("Android") {
                    agent_os = AgentOs::Android;
                }
            } else {
                firefox_agent = user_agent.contains("Firefox");
            }

            agent_os
        };

        let mut emulation_config = Self::default();

        emulation_config.firefox_agent = firefox_agent;
        emulation_config.agent_os = agent_os;

        emulation_config
    }
}

/// Join the scrips pre-allocated.
fn join_scripts<I: IntoIterator<Item = impl AsRef<str>>>(parts: I) -> String {
    // Heuristically preallocate some capacity (tweak as needed for your use-case).
    let mut script = String::with_capacity(4096);
    for part in parts {
        script.push_str(part.as_ref());
    }
    script
}

/// Emulate a real chrome browser.
pub fn emulate(
    user_agent: &str,
    config: &EmulationConfiguration,
    viewport: &Option<crate::spoof_viewport::Viewport>,
    evaluate_on_new_document: &Option<Box<String>>,
) -> Option<String> {
    use crate::spoof_gpu::{
        FP_JS, FP_JS_GPU_LINUX, FP_JS_GPU_MAC, FP_JS_GPU_WINDOWS, FP_JS_LINUX, FP_JS_MAC,
        FP_JS_WINDOWS,
    };
    use crate::spoofs::{
        resolve_dpr, spoof_history_length_script, spoof_media_codecs_script,
        spoof_media_labels_script, spoof_screen_script_rng, spoof_touch_screen, DISABLE_DIALOGS,
        SPOOF_NOTIFICATIONS, SPOOF_PERMISSIONS_QUERY,
    };
    use rand::Rng;

    let stealth = config.tier.stealth();
    let dismiss_dialogs = config.dismiss_dialogs;
    let agent_os = config.agent_os;
    let firefox_agent = config.firefox_agent;

    let spoof_script = if stealth && !firefox_agent {
        &crate::spoof_user_agent::spoof_user_agent_data_high_entropy_values(
            &crate::spoof_user_agent::build_high_entropy_data(&Some(user_agent)),
        )
    } else {
        &Default::default()
    };

    let linux = agent_os == AgentOs::Linux;

    let mut fingerprint_gpu = false;
    let fingerprint = match config.fingerprint {
        Fingerprint::Basic => true,
        Fingerprint::NativeGPU => {
            fingerprint_gpu = true;
            true
        }
        _ => false,
    };

    let fp_script = if fingerprint {
        let fp_script = if linux {
            if fingerprint_gpu {
                &*FP_JS_GPU_LINUX
            } else {
                &*FP_JS_LINUX
            }
        } else if agent_os == AgentOs::Mac {
            if fingerprint_gpu {
                &*FP_JS_GPU_MAC
            } else {
                &*FP_JS_MAC
            }
        } else if agent_os == AgentOs::Windows {
            if fingerprint_gpu {
                &*FP_JS_GPU_WINDOWS
            } else {
                &*FP_JS_WINDOWS
            }
        } else {
            &*FP_JS
        };
        fp_script
    } else {
        &Default::default()
    };

    let disable_dialogs = if dismiss_dialogs { DISABLE_DIALOGS } else { "" };
    let mut mobile_device = false;

    let screen_spoof = if let Some(viewport) = &viewport {
        mobile_device = viewport.emulating_mobile;
        let dpr = resolve_dpr(
            viewport.emulating_mobile,
            viewport.device_scale_factor,
            agent_os,
        );

        spoof_screen_script_rng(
            viewport.width,
            viewport.height,
            dpr,
            viewport.emulating_mobile,
            &mut rand::rng(),
            agent_os,
        )
    } else {
        Default::default()
    };

    let st = crate::build_stealth_script(config.tier, agent_os);

    // Final combined script to inject
    let merged_script = if let Some(script) = evaluate_on_new_document.as_deref() {
        if fingerprint {
            let mut b = join_scripts([
                &fp_script,
                &spoof_script,
                disable_dialogs,
                &screen_spoof,
                SPOOF_NOTIFICATIONS,
                SPOOF_PERMISSIONS_QUERY,
                &spoof_media_codecs_script(),
                &spoof_touch_screen(mobile_device),
                &spoof_media_labels_script(agent_os),
                &spoof_history_length_script(rand::rng().random_range(1..=6)),
                &st,
                &wrap_eval_script(script),
            ]);

            b.push_str(&wrap_eval_script(script));

            Some(b)
        } else {
            let mut b = join_scripts([
                &spoof_script,
                disable_dialogs,
                &screen_spoof,
                SPOOF_NOTIFICATIONS,
                SPOOF_PERMISSIONS_QUERY,
                &spoof_media_codecs_script(),
                &spoof_touch_screen(mobile_device),
                &spoof_media_labels_script(agent_os),
                &spoof_history_length_script(rand::rng().random_range(1..=6)),
                &st,
                &wrap_eval_script(script),
            ]);
            b.push_str(&wrap_eval_script(script));

            Some(b)
        }
    } else if fingerprint {
        Some(join_scripts([
            &fp_script,
            &spoof_script,
            disable_dialogs,
            &screen_spoof,
            SPOOF_NOTIFICATIONS,
            SPOOF_PERMISSIONS_QUERY,
            &spoof_media_codecs_script(),
            &spoof_touch_screen(mobile_device),
            &spoof_media_labels_script(agent_os),
            &spoof_history_length_script(rand::rng().random_range(1..=6)),
            &st,
        ]))
    } else if stealth {
        Some(join_scripts([
            &spoof_script,
            disable_dialogs,
            &screen_spoof,
            SPOOF_NOTIFICATIONS,
            SPOOF_PERMISSIONS_QUERY,
            &spoof_media_codecs_script(),
            &spoof_touch_screen(mobile_device),
            &spoof_media_labels_script(agent_os),
            &spoof_history_length_script(rand::rng().random_range(1..=6)),
            &st,
        ]))
    } else {
        None
    };

    merged_script
}
