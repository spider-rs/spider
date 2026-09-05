#![cfg(all(feature = "control", not(feature = "decentralized")))]

use spider::website::Website;
use std::time::Duration;

async fn wait_for_receivers(expected: usize) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while spider::utils::CONTROLLER.0.receiver_count() != expected {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("control subscriptions must return to their expected count");
}

// Keep the global subscription assertions in one test to avoid racing each other.
#[tokio::test]
async fn cancelled_setup_and_crawl_release_control_handlers() {
    for respect_robots in [true, false] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind local test endpoint");
        let url = format!("http://{}", listener.local_addr().unwrap());
        let mut website = Website::new(&url);
        website.with_respect_robots_txt(respect_robots);
        let baseline = spider::utils::CONTROLLER.0.receiver_count();
        let mut operation = Box::pin(async {
            if respect_robots {
                let (_, handler) = website.setup().await;
                if let Some((_, task)) = handler {
                    task.abort();
                }
            } else {
                website.crawl().await;
            }
        });
        let connection = tokio::time::timeout(Duration::from_secs(5), async {
            tokio::select! {
                result = listener.accept() => result.expect("accept local request"),
                _ = &mut operation => panic!("operation must wait for HTTP response"),
            }
        })
        .await
        .expect("request must reach local endpoint");
        wait_for_receivers(baseline + 1).await;
        drop(operation);
        wait_for_receivers(baseline).await;
        drop(connection);
    }
}
