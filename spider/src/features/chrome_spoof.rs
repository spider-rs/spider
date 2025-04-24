use case_insensitive_string::compact_str;

/// Represents a browser brand and its version, used for spoofing `userAgentData.fullVersionList`.
pub struct BrandEntry {
    /// The name of the browser brand (e.g., "Chromium", "Not-A.Brand").
    pub brand: String,
    /// The full version string of the brand (e.g., "122.0.0.0").
    pub version: String,
}

/// Represents the high-entropy values returned by `navigator.userAgentData.getHighEntropyValues()`.
pub struct HighEntropyUaData {
    /// The CPU architecture of the device (e.g., "x86", "arm").
    pub architecture: String,
    /// The device model (mostly non-empty for Android devices).
    pub model: String,
    /// The bitness property.
    pub bitness: String,
    /// The platform being used.
    pub platform: String,
    /// The OS platform version (e.g., "10.0" for Windows 10, "13" for Android 13).
    pub platform_version: String,
    /// A list of brand/version pairs representing the full user agent fingerprint.
    pub full_version_list: Vec<BrandEntry>,
}

/// Build the entropy data.
pub fn build_high_entropy_data(
    user_agent: &Option<Box<compact_str::CompactString>>,
) -> HighEntropyUaData {
    let user_agent: &str = user_agent.as_deref().map_or("", |v| v);

    let full_version = user_agent
        .split_whitespace()
        .find(|s| s.starts_with("Chrome/"))
        .and_then(|s| s.strip_prefix("Chrome/"))
        .unwrap_or("135.0.0.0");

    let (architecture, model, platform, platform_version, bitness): (
        &str,
        String,
        &str,
        String,
        &str,
    ) = if user_agent.contains("Android") {
        let version = user_agent
            .split(';')
            .find_map(|s| s.trim().strip_prefix("Android "))
            .unwrap_or("13");

        let model = user_agent
            .split(';')
            .nth(2)
            .map(|s| s.trim().to_string())
            .unwrap_or_default();

        let bitness = if user_agent.contains("arm64") || user_agent.contains("aarch64") {
            "64"
        } else {
            "32"
        };

        ("arm", model, "Android", version.to_string(), bitness)
    } else if user_agent.contains("Windows NT") {
        let version = user_agent
            .split("Windows NT ")
            .nth(1)
            .and_then(|s| s.split(';').next())
            .unwrap_or("10.0");

        let bitness = if user_agent.contains("Win64")
            || user_agent.contains("x64")
            || user_agent.contains("WOW64")
        {
            "64"
        } else {
            "32"
        };

        (
            "x86",
            "".to_string(),
            "Windows",
            version.to_string(),
            bitness,
        )
    } else if user_agent.contains("Mac OS X") {
        let chrome_major = full_version
            .split('.')
            .next()
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(135);

        let base_chrome = 135;
        let base_mac = 14.6;
        let delta = if chrome_major > base_chrome {
            ((chrome_major - base_chrome) as f32 * 0.1).round()
        } else {
            0.0
        };

        let mac_major = base_mac + delta;
        let platform_version = format!("{:.1}.1", mac_major);

        ("arm", "".to_string(), "macOS", platform_version, "64")
    } else if user_agent.contains("Linux") {
        let platform_version = full_version
            .split('.')
            .take(3)
            .collect::<Vec<_>>()
            .join(".");

        let bitness = if user_agent.contains("x86_64")
            || user_agent.contains("amd64")
            || user_agent.contains("arm64")
        {
            "64"
        } else {
            "32"
        };

        ("x86", "".to_string(), "Linux", platform_version, bitness)
    } else {
        ("x86", "".to_string(), "Unknown", "1.0.0".to_string(), "64")
    };

    HighEntropyUaData {
        architecture: architecture.to_string(),
        bitness: bitness.to_string(),
        model,
        platform: platform.to_string(),
        platform_version,
        full_version_list: vec![
            BrandEntry {
                brand: "Google Chrome".into(),
                version: full_version.into(),
            },
            BrandEntry {
                brand: "Not-A.Brand".into(),
                version: "8.0.0.0".into(),
            },
            BrandEntry {
                brand: "Chromium".into(),
                version: full_version.into(),
            },
        ],
    }
}

/// Spoof to a js snippet.
pub fn spoof_user_agent_data_high_entropy_values(data: &HighEntropyUaData) -> String {
    let brands = data
        .full_version_list
        .iter()
        .map(|b| {
            let major_version = b.version.split('.').next().unwrap_or("99");
            format!("{{brand:'{}',version:'{}'}}", b.brand, major_version)
        })
        .collect::<Vec<_>>()
        .join(",");

    // `fullVersionList`: full versions preserved
    let full_versions = data
        .full_version_list
        .iter()
        .map(|b| format!("{{brand:'{}',version:'{}'}}", b.brand, b.version))
        .collect::<Vec<_>>()
        .join(",");

    format!(
        "(()=>{{const v={{brands:[{brands}],mobile:!1,platform:'{platform}',getHighEntropyValues:h=>Promise.resolve(Object.fromEntries(h.map(k=>[k,{{architecture:'{arch}',model:'{model}',bitness:'{bitness}',platformVersion:'{plat}',fullVersionList:[{full_versions}]}}[k]??null])))}};const f=function(){{return v}};Object.defineProperty(f,'toString',{{value:()=>\"function get userAgentData() {{ [native code] }}\"}});Object.defineProperty(Navigator.prototype,'userAgentData',{{get:f,configurable:!0}});}})();",
        platform = data.platform,
        arch = data.architecture,
        bitness = data.bitness,
        model = data.model,
        plat = data.platform_version,
        brands = brands,
        full_versions = full_versions
    )
}
