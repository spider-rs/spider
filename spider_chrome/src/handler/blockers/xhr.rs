use case_insensitive_string::CaseInsensitiveString;
use hashbrown::HashSet;

lazy_static::lazy_static! {
     /// Visual assets to ignore for XHR request.
    pub(crate) static ref IGNORE_XHR_ASSETS: HashSet<CaseInsensitiveString> = {
        let mut m: HashSet<CaseInsensitiveString> = HashSet::with_capacity(36);

        m.extend([
            "jpg", "jpeg", "png", "gif", "svg", "webp",       // Image files
            "mp4", "avi", "mov", "wmv", "flv",               // Video files
            "mp3", "wav", "ogg",                             // Audio files
            "woff", "woff2", "ttf", "otf",                   // Font files
            "swf", "xap",                                    // Flash/Silverlight files
            "ico", "eot",                                    // Other resource files

            // Including extensions with extra dot
            ".jpg", ".jpeg", ".png", ".gif", ".svg", ".webp",
            ".mp4", ".avi", ".mov", ".wmv", ".flv",
            ".mp3", ".wav", ".ogg",
            ".woff", ".woff2", ".ttf", ".otf",
            ".swf", ".xap",
            ".ico", ".eot"
        ].map(|s| s.into()));

        m
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use case_insensitive_string::CaseInsensitiveString;

    #[test]
    fn test_ignore_xhr_assets_contains() {
        // Positive tests - these file types (considering case insensitivity) should be contained in the set
        let positive_cases = vec!["jpg", "mp3", "WOFF", ".svg"];

        // Negative tests - these file types should not be contained in the set
        let negative_cases = vec!["randomfiletype", "xyz"];

        for case in positive_cases {
            let case_ci: CaseInsensitiveString = case.into();
            assert!(
                IGNORE_XHR_ASSETS.contains(&case_ci),
                "HashSet should contain: {}",
                case
            );
        }

        for case in negative_cases {
            let case_ci: CaseInsensitiveString = case.into();
            assert!(
                !IGNORE_XHR_ASSETS.contains(&case_ci),
                "HashSet should not contain: {}",
                case
            );
        }
    }
}
