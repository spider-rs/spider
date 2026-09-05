//! Each integration test binary has its own global controller, so receiver
//! counts here cannot race unrelated crawl tests.
#![cfg(feature = "control")]

use spider::{utils::CONTROLLER, website::Website};

#[tokio::test(flavor = "current_thread")]
async fn configure_setup_releases_unused_control_handlers() {
    let baseline = CONTROLLER.0.receiver_count();
    let mut website = Website::new("https://example.com");
    website.configuration.respect_robots_txt = false;

    for no_control_thread in [false, true] {
        website.configuration.no_control_thread = no_control_thread;
        for _ in 0..32 {
            website.configure_setup().await;
            // Give spawned tasks time to subscribe, or observe their abort.
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            assert_eq!(CONTROLLER.0.receiver_count(), baseline);

            website.configure_setup_norobots();
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            assert_eq!(CONTROLLER.0.receiver_count(), baseline);
        }
    }

    drop(website);
    tokio::time::sleep(std::time::Duration::from_millis(1)).await;
    assert_eq!(CONTROLLER.0.receiver_count(), baseline);
}
