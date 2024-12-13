extern crate phf_codegen;
use std::env;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::Path;

fn main() {
    let out_dir = env::var("OUT_DIR").unwrap();
    let domain_map_path = Path::new(&out_dir).join("domain_map.rs");
    let url_trie_path = Path::new(&out_dir).join("url_ignore_trie.rs");
    let blockers_dir = Path::new(&out_dir).join("blockers");
    fs::create_dir_all(&blockers_dir).unwrap();

    let pattern_dir = "url_patterns/domains";

    generate_domain_map(&domain_map_path, pattern_dir);
    generate_url_ignore_tries(&url_trie_path, pattern_dir);
    generate_blockers(&blockers_dir, pattern_dir);
    generate_blockers_mod(&blockers_dir, pattern_dir);
}

fn generate_domain_map(domain_map_path: &Path, pattern_dir: &str) {
    let mut file = BufWriter::new(File::create(&domain_map_path).unwrap());
    let mut map = phf_codegen::Map::new();

    writeln!(file, "mod blockers;\nmod url_ignore_trie;").unwrap();
    writeln!(
        &mut file,
        "#[derive(Default, Debug, Clone, Copy, PartialEq)]"
    )
    .unwrap();
    writeln!(
        &mut file,
        r#"#[derive(serde::Serialize, serde::Deserialize)]"#
    )
    .unwrap();
    writeln!(&mut file, "pub enum NetworkInterceptManager {{").unwrap();

    let mut domain_variants = vec![];

    for entry in fs::read_dir(pattern_dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();

        if let Some(domain_name) = path.file_stem().unwrap().to_str() {
            let enum_name = format_ident(domain_name);
            writeln!(&mut file, "    {},", enum_name).unwrap();
            domain_variants.push((domain_name.to_string(), enum_name.clone()));
            map.entry(
                format!("{}", domain_name),
                &format!("NetworkInterceptManager::{}", enum_name),
            );
        }
    }

    writeln!(&mut file, "    #[default]\n    UNKNOWN,").unwrap(); // Default case
    writeln!(&mut file, "}}\n").unwrap();

    write!(
        file,
        "static DOMAIN_MAP: phf::Map<&'static str, NetworkInterceptManager> = {};\n",
        map.build()
    )
    .unwrap();

    writeln!(file, "impl NetworkInterceptManager {{").unwrap();
    writeln!(file, "    pub fn intercept_detection(&self, url: &str, ignore_visuals: bool, is_xhr: bool) -> bool {{").unwrap();
    writeln!(file, "        let mut should_block = false;").unwrap();
    writeln!(file, "        match self {{").unwrap();

    for (domain_name, enum_name) in domain_variants {
        let clean_name = domain_name.split('.').next().unwrap().to_lowercase();
        writeln!(
            file,
            "            NetworkInterceptManager::{} => {{",
            enum_name
        )
        .unwrap();
        writeln!(file, "                if is_xhr {{").unwrap();
        writeln!(
            file,
            "                    should_block = blockers::{}_blockers::block_xhr(url);",
            clean_name
        )
        .unwrap();
        writeln!(file, "                }} else {{").unwrap();
        writeln!(
            file,
            "                    should_block = blockers::{}_blockers::block_scripts(url);",
            clean_name
        )
        .unwrap();
        writeln!(
            file,
            "                    if !should_block && ignore_visuals {{"
        )
        .unwrap();
        writeln!(
            file,
            "                        should_block = blockers::{}_blockers::block_styles(url);",
            clean_name
        )
        .unwrap();
        writeln!(file, "                    }}").unwrap();
        writeln!(file, "                }}").unwrap();
        writeln!(file, "            }},").unwrap();
    }

    writeln!(file, "            NetworkInterceptManager::UNKNOWN => (),").unwrap();

    writeln!(file, "        }}").unwrap();
    writeln!(file, "        should_block").unwrap();
    writeln!(file, "    }}").unwrap();
    writeln!(file, "}}").unwrap();
}

fn generate_url_ignore_tries(url_trie_path: &Path, pattern_dir: &str) {
    let mut file = BufWriter::new(File::create(url_trie_path).unwrap());

    writeln!(file, "use crate::handler::blockers::Trie;").unwrap();
    writeln!(file, "lazy_static::lazy_static! {{").unwrap();

    for category in &["scripts", "xhr", "styles"] {
        if let Ok(domain_entries) = fs::read_dir(pattern_dir) {
            for domain_entry in domain_entries {
                let domain_entry = domain_entry.unwrap();
                let domain_path = domain_entry.path();

                if domain_path.is_dir() {
                    let domain_name = domain_path.file_name().unwrap().to_str().unwrap();
                    let category_domain_path = domain_path.join(category);

                    if let Ok(category_entries) = fs::read_dir(&category_domain_path) {
                        let trie_name = format_ident(&format!("{}_{}", domain_name, category));
                        writeln!(
                            file,
                            "pub static ref {}_TRIE: Trie = {{",
                            trie_name.to_uppercase()
                        )
                        .unwrap();
                        writeln!(file, "let mut trie = Trie::new();").unwrap();

                        for entry in category_entries {
                            let entry = entry.unwrap();
                            let path = entry.path();

                            if path.is_file() {
                                let contents = fs::read_to_string(path).unwrap();
                                for pattern in contents.lines() {
                                    writeln!(file, "trie.insert({:?});", pattern.trim()).unwrap();
                                }
                            }
                        }

                        writeln!(file, "trie").unwrap();
                        writeln!(file, "}};").unwrap();
                    }
                }
            }
        }
    }

    writeln!(file, "}}").unwrap();
}

fn generate_blockers(blockers_dir: &Path, pattern_dir: &str) {
    if let Ok(domain_entries) = fs::read_dir(pattern_dir) {
        for domain_entry in domain_entries {
            let domain_entry = domain_entry.unwrap();
            let domain_path = domain_entry.path();

            if domain_path.is_dir() {
                let domain_name = domain_path.file_name().unwrap().to_str().unwrap();
                let file_name = format!("{}_blockers.rs", domain_name.split('.').next().unwrap());
                let file_path = blockers_dir.join(file_name);
                let mut file = BufWriter::new(File::create(file_path).unwrap());

                // Generate block_scripts
                let scripts_trie_name = format_ident(&format!("{}_scripts", domain_name));
                writeln!(file, "pub fn block_scripts(url: &str) -> bool {{").unwrap();
                writeln!(file, "    crate::handler::blockers::intercept_manager::url_ignore_trie::{}_TRIE.contains_prefix(url)", scripts_trie_name.to_uppercase()).unwrap();
                writeln!(file, "}}\n").unwrap();

                // Generate block_styles
                let styles_trie_name = format_ident(&format!("{}_styles", domain_name));
                writeln!(file, "pub fn block_styles(url: &str) -> bool {{").unwrap();
                writeln!(file, "    crate::handler::blockers::intercept_manager::url_ignore_trie::{}_TRIE.contains_prefix(url)", styles_trie_name.to_uppercase()).unwrap();
                writeln!(file, "}}\n").unwrap();

                // Generate block_xhr
                let xhr_trie_name = format_ident(&format!("{}_xhr", domain_name));
                writeln!(file, "pub fn block_xhr(url: &str) -> bool {{").unwrap();
                writeln!(file, "    crate::handler::blockers::intercept_manager::url_ignore_trie::{}_TRIE.contains_prefix(url)", xhr_trie_name.to_uppercase()).unwrap();
                writeln!(file, "}}\n").unwrap();
            }
        }
    }
}

fn generate_blockers_mod(blockers_dir: &Path, pattern_dir: &str) {
    let mod_file_path = blockers_dir.join("mod.rs");
    let mut mod_file = BufWriter::new(File::create(mod_file_path).unwrap());

    if let Ok(domain_entries) = fs::read_dir(pattern_dir) {
        for domain_entry in domain_entries {
            let domain_entry = domain_entry.unwrap();
            let clean_name = domain_entry
                .file_name()
                .to_str()
                .unwrap_or_default()
                .split('.')
                .next()
                .unwrap()
                .to_lowercase();

            writeln!(mod_file, "pub mod {}_blockers;", clean_name).unwrap();
        }
    }
}

fn format_ident(name: &str) -> String {
    name.replace('.', "_").replace('-', "_").to_uppercase()
}
