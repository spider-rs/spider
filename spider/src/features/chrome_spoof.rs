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
        .unwrap_or("122.0.0.0");

    let (architecture, model, platform_version) = if user_agent.contains("Android") {
        // Example: Android 13; Pixel 7 Pro
        let version = user_agent
            .split(';')
            .find(|s| s.trim().starts_with("Android "))
            .map(|s| s.trim().strip_prefix("Android ").unwrap_or("13"))
            .unwrap_or("13");

        let model = user_agent
            .split(';')
            .nth(2)
            .map(|s| s.trim().to_string())
            .unwrap_or_default();

        ("arm", model, version.to_string())
    } else if user_agent.contains("Windows NT") {
        // Example: Windows NT 10.0; Win64; x64
        let version = user_agent
            .split("Windows NT ")
            .nth(1)
            .and_then(|s| s.split(';').next())
            .unwrap_or("10.0");

        ("x86", "".to_string(), version.to_string())
    } else if user_agent.contains("Macintosh") {
        // Example: Mac OS X 10_15_7
        let version = user_agent
            .split("Mac OS X ")
            .nth(1)
            .and_then(|s| s.split(')').next())
            .map(|s| s.replace('_', "."))
            .unwrap_or("13.6.0".to_string());

        ("x86", "".to_string(), version)
    } else if user_agent.contains("Linux") {
        (
            "x86",
            "".to_string(),
            full_version
                .split('.')
                .take(3)
                .collect::<Vec<_>>()
                .join("."),
        )
    } else {
        ("x86", "".to_string(), "1.0.0".to_string())
    };

    HighEntropyUaData {
        architecture: architecture.to_string(),
        model,
        platform_version,
        full_version_list: vec![
            BrandEntry {
                brand: "Chromium".into(),
                version: full_version.into(),
            },
            BrandEntry {
                brand: "Not-A.Brand".into(),
                version: "99.0.0.0".into(),
            },
        ],
    }
}

/// Spoof to a js snippet.
pub fn spoof_user_agent_data_high_entropy_values(data: &HighEntropyUaData) -> String {
    let brands = data
        .full_version_list
        .iter()
        .map(|b| format!("{{brand:'{}',version:'{}'}}", b.brand, b.version))
        .collect::<Vec<_>>()
        .join(",");

    let script = format!(
        "Object.defineProperty(navigator.userAgentData,'getHighEntropyValues',{{configurable:!0,enumerable:!1,writable:!1,value:function(h){{const v={{architecture:'{}',model:'{}',platformVersion:'{}',fullVersionList:[{}]}};return Promise.resolve(Object.fromEntries(h.map(k=>[k,v[k]??null])))}}}});",
        data.architecture,
        data.model,
        data.platform_version,
        brands
    );

    script
}
