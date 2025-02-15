use phf_codegen::Set;
use std::env;
use std::fs;
use std::path::PathBuf;

fn main() -> std::io::Result<()> {
    let out_dir = env::var("OUT_DIR").unwrap();
    let response = reqwest::blocking::get(
        "https://raw.githubusercontent.com/spider-rs/bad_websites/main/websites.txt",
    )
    .expect("Failed to fetch file");

    let content = response.text().expect("Failed to read response text");
    let websites: Vec<&str> = content.lines().collect();

    let mut set = Set::new();

    for website in websites {
        set.entry(website);
    }

    let dest_path = PathBuf::from(out_dir).join("bad_websites.rs");

    fs::write(
        &dest_path,
        format!(
            "/// Bad websites that we should not crawl.\n\
        static BAD_WEBSITES: phf::Set<&'static str> = {};",
            set.build()
        ),
    )?;

    Ok(())
}
