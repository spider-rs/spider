//! `cargo run --bin profile`
extern crate spider;

use eframe::egui;
use spider::tokio;
use spider::website::Website;
use tokio::runtime::Runtime;

#[derive(Default)]
pub struct SpiderApp {
    frame_counter: u64,
}

fn main() {
    puffin::set_scopes_on(true);
    let _ = eframe::run_native(
        "puffin egui eframe",
        Default::default(),
        Box::new(|_cc| Box::<SpiderApp>::default()),
    );
}

impl eframe::App for SpiderApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        puffin::profile_function!();
        puffin::GlobalProfiler::lock().new_frame();
        puffin_egui::profiler_window(ctx);

        let rt = Runtime::new().unwrap();
        let mut website: Website = Website::new("https://jeffmendez.com");

        std::thread::sleep(std::time::Duration::from_millis(14));

        // prevent dos
        if self.frame_counter % 100 == 0 {
            rt.block_on(async {
                puffin::profile_scope!("Concurrent");
                website.crawl().await;
            });

            rt.block_on(async {
                puffin::profile_scope!("Sync");
                website.crawl_sync().await;
            });
        }

        self.frame_counter += 1;
    }
}
