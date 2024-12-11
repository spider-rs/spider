use crate::handler::blockers::Trie;

lazy_static::lazy_static! {
        /// Ignore list of urls.
        static ref URL_IGNORE_TRIE: Trie = {
            let mut trie = Trie::new();
            let patterns = [
                "https://meta.wikimedia.org/w/index.php?title=MediaWiki:Wikiminiatlas.js&action=raw&ctype=text/javascript",
                "https://login.wikimedia.org/wiki/Special:CentralAutoLogin/checkLoggedIn?useformat=desktop&wikiid=ptwiki&type=script&wikiid=ptwiki&type=script",
                ".wikipedia.org/w/load.php?lang=pt&modules=ext.centralNotice.choiceData%2CgeoIP%2CstartUp%7Cext.centralauth.ForeignApi%2Ccentralautologin%7Cext.checkUser.clientHints%7Cext.cite.ux-enhancements%7Cext.cx.eventlogging.campaigns",
                ".wikipedia.org/w/load.php?lang=pt&modules=startup&only=scripts&raw=1&skin=vector-2022",
                ".eventlogging.campaigns",
                "%2CFeedbackHighlight%2",
                ".quicksurveys.",
                "Special:CentralAutoLogin/start?type=script",
            ];
            for pattern in &patterns {
                trie.insert(pattern);
            }
            trie
        };
}

// Block wikipedia events that are not required
pub fn block_wikipedia(
    event: &chromiumoxide_cdp::cdp::browser_protocol::fetch::EventRequestPaused,
) -> bool {
    URL_IGNORE_TRIE.contains_prefix(&event.request.url)
}
