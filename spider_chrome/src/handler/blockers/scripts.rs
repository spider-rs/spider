use crate::handler::blockers::Trie;

lazy_static::lazy_static! {
    /// Ignore list of scripts.
    pub (crate) static ref URL_IGNORE_TRIE: Trie = {
        let mut trie = Trie::new();
        let patterns = [
            "https://www.googletagservices.com/tag/",
            "https://js.hs-analytics.net/analytics/",
            "https://www.googletagmanager.com/gtag",
            "https://www.googletagmanager.com/gtm.js",
            "https://js.hsadspixel.net",
            "https://www.google.com/adsense/",
            "https://www.googleadservices.com",
            "https://static.cloudflareinsights.com/",
            "https://adservice.google.com",
            "https://www.gstatic.com/cv/js/sender/",
            "https://googleads.g.doubleclick.net",
            "https://www.google-analytics.com",
            "https://iabusprivacy.pmc.com/geo-info.js",
            "https://cookie-cdn.cookiepro.com/consent",
            "https://load.sumome.com/",
            "https://www.mongoosemetrics.com/",
            "https://geolocation-recommendations.shopifyapps.com/",
            "https://w.usabilla.com/",
            "https://consentcdn.cookiebot.com/",
            "https://plausible.io/api/event",
            "https://sentry.io/api/",
            "https://cdn.onesignal.com",
            "https://cdn.cookielaw.org/",
            "https://static.doubleclick.net",
            "https://tools.luckyorange.com/",
            "https://cdn.piano.io",
            "https://px.ads.linkedin.com",
            "https://connect.facebook.net",
            "https://tags.tiqcdn.com",
            "https://tr.snapchat.com",
            "https://ads.twitter.com",
            "https://cdn.segment.com",
            "https://stats.wp.com",
            "https://analytics.",
            "http://analytics.",
            "https://cdn.cxense.com",
            "https://cdn.tinypass.com",
            "https://cd.connatix.com",
            "https://platform-api.sharethis.com/js/sharethis.js",
            "https://js.hsforms.net/forms/embed/v2.js",
            "https://static.parastorage.com/services/wix-thunderbolt/dist/",
            "https://static.parastorage.com/services/tag-manager-client/",
            "https://static.parastorage.com/services/form-app/",
            "https://www.datadoghq-browser-agent.com/datadog-rum-slim-v4.js",
            "https://cdn.rudderlabs.com",
            "https://script.hotjar.com/",
            "https://static.hotjar.com/",
            "https://cdn.insurads.com/",
            "https://cdn-ukwest.onetrust.com",
            "https://cdn.onetrust.com",
            "https://services.insurads.com/",
            "https://platform.iteratehq.com/loader.js",
            "https://acdn.adnxs.com/ast/ast.js",
            "https://schibsted-cdn.relevant-digital.com/static/tags/",
            "https://bat.bing.net",
            "https://static.addtoany.com/menu/",
            "https://www.b2i.us/b2i/",
            "https://acsbapp.com/apps/app/dist/js/app.js",
            "https://cdn.doofinder.com/livelayer/",
            "https://load.sumo.com/",
            "https://cdn11.bigcommerce.com/",
            "https://na.shgcdn3.com/collector.js",
            "https://microapps.bigcommerce.com/bodl-events/index.js",
            "https://checkout-sdk.bigcommerce.com/v1/loader.js",
            "https://cdn.callrail.com/companies/",
            "https://www.webtraxs.com/trxscript.php",
            "https://diffuser-cdn.app-us1.com/diffuser/diffuser.js",
            "https://try.abtasty.com/",
            "https://imasdk.googleapis.com/js/sdkloader/ima3.js",
            "https://cdn.registerdisney.go.com/v4/responder.js",
            "https://cdn.registerdisney.go.com/v4/OneID.js",
            "https://js-agent.newrelic.com/",
            "https://bat.bing.com/bat.js",
            "https://s1.hdslb.com/bfs/cm/cm-sdk/static/js/track-collect.js",
            "https://hm.baidu.com/hm.js",
            "https://unpkg.zhimg.com/@efe/zhad-tracker",
            ".sharethis.com",
            ".newrelic.com",
            ".googlesyndication.com",
            ".amazon-adsystem.com",
            ".onetrust.com",
            "sc.omtrdc.net",
            "doubleclick.net",
            "hotjar.com",
            "datadome.com",
            "datadog-logs-us.js",
            "tinypass.min.js",
            ".airship.com",
            ".adlightning.com",
            ".lab.amplitude.",
            // explicit ignore tracking.js and ad files
            "privacy-notice.js",
            "tracking.js",
            "plugins/cookie-law-info/legacy/",
            "ads.js",
            "insight.min.js",
            "https://ads.",
            "http://ads.",
            "https://tracking.",
            "http://tracking.",
            "https://static-tracking.",
            // exp testin
            // used for possible location outside
            "https://geo.privacymanager.io/",
            // "https://www.recaptcha.net/recaptcha/",
            // "https://www.google.com/recaptcha/",
            // "https://www.gstatic.com/recaptcha/",
        ];
        for pattern in &patterns {
            trie.insert(pattern);
        }
        trie
    };

    /// Ignore list of scripts embedded or font extra.
    pub(crate) static ref URL_IGNORE_EMBEDED_TRIE: Trie = {
        let mut trie = Trie::new();
        let patterns = [
            "https://www.youtube.com/embed/",      // YouTube video embeds
            "https://www.google.com/maps/embed?",  // Google Maps embeds
            "https://player.vimeo.com/video/",     // Vimeo video embeds
            "https://open.spotify.com/embed/",     // Spotify music embeds
            "https://w.soundcloud.com/player/",    // SoundCloud embeds
            "https://platform.twitter.com/embed/", // Twitter embedded tweets
            "https://www.instagram.com/embed.js",  // Instagram embeds
            "https://www.facebook.com/plugins/",   // Facebook embeds (like posts and videos)
            "https://cdn.embedly.com/widgets/",    // Embedly embeds
            "https://player.twitch.tv/",           // Twitch video player embeds
            "https://maps.googleapis.com/maps/", // Google map embeds
            "https://www.youtube.com/player_api", // Youtube player.
            "https://www.googletagmanager.com/ns.html", // Google tag manager.
            "https://consentcdn.cookiebot.com", // Cookie bot
            "https://www.youtube.com/iframe_api", // Youtube iframes.
            "https://f.vimeocdn.com", // Vimeo EMBEDDINGS
            "https://i.vimeocdn.com/",
            // "https://www.youtube.com/s/player/", // Youtube player not needed usually since iframe_api is used mainly
            // vercel live
            "https://vercel.live/api/",

            // extra CDN scripts
            "https://cdn.readme.io/public/",
            // font awesome
            "https://use.fontawesome.com/",
            // insight tracker
            "https://insight.adsrvr.org/track/",
            "http://www.google-analytics.com/ga.js",
            "cxense.com/",
            // snapchat tracker
            "https://tr.snapchat.com/",
            "https://buy.tinypass.com",
            "https://nimbleplot.com/",
            "https://my.actiondata.co/js/tracker.php",
            "https://ajax.googleapis.com/ajax/libs/webfont/",
            "http://cdn2.editmysite.com/",
            // ignore font extras
            "https://kit.fontawesome.com/",
            "https://use.typekit.net",
            ".amplitude.com",
            ".rudderstack.com",
            // ignore tailwind cdn
            "https://cdn.tailwindcss.com",
            // ignore extra ads
            ".sharethis.com",
            "amazon-adsystem.com",
            ".vimeocdn.com",
            "g.doubleclick.net",
            "https://securepubads.g.doubleclick.net",
            "googlesyndication.com",
            "adsafeprotected.com",
            // more google tracking
            ".googlesyndication.com/safeframe/",
            // repeat consent js
            "/ccpa/user-consent.min.js",
            "/cookiebanner/js/",
            "cookielaw.org",
            // privacy
            "otBannerSdk.js",
            "privacy-notice.js",
            ".ingest.sentry.io/api",
            // ignore amazon scripts for media
            ".ssl-images-amazon.com/images/"
        ];
        for pattern in &patterns {
            trie.insert(pattern);
        }
        trie
    };

    /// Ignore list of path scripts to ignore for tracking and analytics.
    pub(crate) static ref URL_IGNORE_SCRIPT_BASE_PATHS: Trie = {
        let mut trie = Trie::new();
        let patterns = [
            "wp-content/plugins/cookie-law-info",
            "wp-content/js/rlt-proxy.js",
            "wp-admin/rest-proxy/",
            "wp-content/mu-plugins/a8c-analytics/",
            "analytics/",
            "cookie-tracking",
        ];
        for pattern in &patterns {
            trie.insert(pattern);
        }
        trie
    };

    /// Ignore list of path scripts to ignore for themes.
    pub (crate) static ref URL_IGNORE_SCRIPT_STYLES_PATHS: Trie = {
        let mut trie = Trie::new();
        let patterns = [
            "wp-content/themes/",
            "wp-content/plugins/dizo-image-hover/",
            "wp-content/plugins/supreme-modules-pro-for-divi/",
            "wp-content/plugins/page-builder-pmc/",
            "wp-content/plugins/contact-form-7/",
            "wp-content/plugins/responsive-lightbox/",
            "wp-content/cache/breeze-minification/",
            "wp-includes/js/mediaelement",
            "wp-content/plugins/gravityforms/",
            "wp-content/plugins/wp-rocket/assets/js/lazyload/",
            "wp-content/plugins/w3-total-cache/",
            "wp-content/js/bilmur.min.js",
            "npm/bootstrap@"
        ];
        for pattern in &patterns {
            trie.insert(pattern);
        }
        trie
    };

}
