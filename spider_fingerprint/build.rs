use std::collections::BTreeMap;
use std::fs::{copy, rename, File};
use std::io::{BufWriter, Write};
use std::path::Path;

fn main() {
    let out_path = std::env::var("OUT_DIR").unwrap();
    let generated_path = format!("{}/chrome_versions.rs", out_path);
    let tmp_path = format!("{}/chrome_versions.rs.tmp", out_path);
    let fallback_path = "chrome_versions.rs.fallback"; // repo root

    let result =
        (|| -> Option<(BTreeMap<String, Vec<String>>, String)> {
            let known_json: serde_json::Value = reqwest::blocking::get(
                "https://googlechromelabs.github.io/chrome-for-testing/known-good-versions.json",
            )
            .ok()?
            .json()
            .ok()?;
            let mut versions_by_major: BTreeMap<String, Vec<String>> = BTreeMap::new();
            for entry in known_json["versions"].as_array()? {
                let ver = entry["version"].as_str()?;
                let major = ver.split('.').next()?;
                versions_by_major
                    .entry(major.to_string())
                    .or_default()
                    .push(ver.to_string());
            }
            let last_json: serde_json::Value = reqwest::blocking::get(
            "https://googlechromelabs.github.io/chrome-for-testing/last-known-good-versions.json"
        ).ok()?.json().ok()?;
            let latest_full = last_json["channels"]["Stable"]["version"]
                .as_str()?
                .to_string();
            Some((versions_by_major, latest_full))
        })();

    match result {
        Some((versions_by_major, latest_full)) => {
            // Write to temporary file first
            {
                let mut file = BufWriter::new(File::create(&tmp_path).unwrap());
                writeln!(file, "use phf::{{phf_map, Map}};").unwrap();
                writeln!(file, "/// Map of Chrome major version to all known good full versions. Generated at build time.").unwrap();
                writeln!(
                    file,
                    "/// The \"latest\" key points to the current stable Chrome full version."
                )
                .unwrap();
                writeln!(file, "pub static CHROME_VERSIONS_BY_MAJOR: Map<&'static str, &'static [&'static str]> = phf_map! {{").unwrap();
                writeln!(file, "    \"latest\" => &[\"{}\"],", latest_full).unwrap();
                for (major, versions) in &versions_by_major {
                    let quoted_versions: Vec<String> =
                        versions.iter().map(|v| format!("\"{}\"", v)).collect();
                    writeln!(
                        file,
                        "    \"{}\" => &[{}],",
                        major,
                        quoted_versions.join(", ")
                    )
                    .unwrap();
                }
                writeln!(file, "}};").unwrap();
                file.flush().unwrap();
            }
            // Atomically move to output and fallback only after a successful write
            if let Err(e) = rename(&tmp_path, &generated_path) {
                eprintln!("{:?}", e)
            }
            // Only now overwrite fallback file
            if let Err(e) = copy(&generated_path, fallback_path) {
                eprintln!("{:?}", e)
            }
        }
        None => {
            // Download or parse failed: use fallback file
            eprintln!(
                "WARNING: Failed to fetch or parse Chrome version lists; using fallback file."
            );
            // Copy fallback to output
            if Path::new(fallback_path).exists() {
                if let Err(e) = copy(fallback_path, &generated_path) {
                    eprintln!("{:?}", e)
                }
            } else {
                panic!("No fallback file found and failed to download new data!");
            }
        }
    }
}
