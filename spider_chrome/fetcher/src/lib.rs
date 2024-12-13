pub use self::browser::{BrowserFetcher, BrowserFetcherOptions, BrowserFetcherRevisionInfo};
pub use self::error::FetcherError;
pub use self::platform::Platform;
pub use self::revision::Revision;

// The chromium revision is hard to get right and the relation to the CDP revision
// even more so, so here are some guidances.
//
// We used to use the revision of Puppeteer, but they switched to chrome-for-testing.
// This means we have to check things ourself. The chromium revision should at least
// as great as the CDP revision otherwise they won't be compatible.
// Not all revisions of chromium have builds for all platforms.
//
// This is essentially a bruteforce process. You can use the test `find_revision_available`
// to find a revision that is available for all platforms. We recommend setting the `min`
// to the current CDP revision and the max to max revision of stable chromium.
// See https://chromiumdash.appspot.com/releases for the latest stable revision.
//
// In general, we should also try to ship as close as a stable version of chromium if possible.
// The CDP should also be a bit older than that stable version.
// To map a revision to a chromium version you can use the site https://chromiumdash.appspot.com/commits.

/// Currently downloaded chromium revision
pub const CURRENT_REVISION: Revision = Revision(1355984);

mod browser;
mod error;
mod platform;
mod revision;
