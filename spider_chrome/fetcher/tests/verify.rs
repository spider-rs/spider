use chromiumoxide_fetcher::{Platform, Revision, CURRENT_REVISION};

// Check if the chosen revision has a build available for all platforms.
// That not always the case, that is why we need to make sure of it.
#[test]
fn verify_revision_available() {
    for platform in &[
        Platform::Linux,
        Platform::Mac,
        Platform::MacArm,
        Platform::Win32,
        Platform::Win64,
    ] {
        let res =
            ureq::head(&platform.download_url("https://storage.googleapis.com", &CURRENT_REVISION))
                .call();

        if res.is_err() {
            panic!(
                "Revision {} is not available for {:?}",
                CURRENT_REVISION, platform
            );
        }
    }
}

#[ignore]
#[test]
fn find_revision_available() {
    let min = 1355000; // Enter the minimum revision
    let max = 1458586; // Enter the maximum revision 05/11/2025

    'outer: for revision in (min..max).rev() {
        println!("Checking revision {}", revision);

        for platform in &[
            Platform::Linux,
            Platform::Mac,
            Platform::MacArm,
            Platform::Win32,
            Platform::Win64,
        ] {
            let res = ureq::head(
                &platform.download_url("https://storage.googleapis.com", &Revision::from(revision)),
            )
            .call();

            if res.is_err() {
                println!("Revision {} is not available for {:?}", revision, platform);
                continue 'outer;
            }
        }

        println!("Found revision {}", revision);
        break;
    }
}
