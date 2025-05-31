use rand::Rng;

lazy_static::lazy_static! {
    /// A list of websites that are common
    // we may want to move this to a new repo like ua_generator.
    static ref WEBSITES: [&'static str; 351] = [
        "https://google.com/",
        "https://msn.com/",
        "https://search.brave.com/",
        "https://search.yahoo.com/",
        "https://facebook.com/",
        "https://amazon.com/",
        "https://reddit.com/",
        "https://youtube.com/",
        "https://wikipedia.org/",
        "https://twitter.com/",
        "https://instagram.com/",
        "https://linkedin.com/",
        "https://netflix.com/",
        "https://spotify.com/",
        "https://apple.com/",
        "https://microsoft.com/",
        "https://yahoo.com/",
        "https://imgur.com/",
        "https://adobe.com/",
        "https://tumblr.com/",
        "https://pinterest.com/",
        "https://ebay.com/",
        "https://craigslist.org/",
        "https://bing.com/",
        "https://office.com/",
        "https://qq.com/",
        "https://taobao.com/",
        "https://sohu.com/",
        "https://vk.com/",
        "https://gitlab.com/",
        "https://wordpress.org/",
        "https://github.com/",
        "https://aliexpress.com/",
        "https://whatsapp.com/",
        "https://weibo.com/",
        "https://etsy.com/",
        "https://shutterstock.com/",
        "https://dropbox.com/",
        "https://quora.com/",
        "https://cloudflare.com/",
        "https://soundcloud.com/",
        "https://paypal.com/",
        "https://medium.com/",
        "https://alibaba.com/",
        "https://huffpost.com/",
        "https://expedia.com/",
        "https://tripadvisor.com/",
        "https://cnn.com/",
        "https://foxnews.com/",
        "https://bbc.com/",
        "https://nytimes.com/",
        "https://theguardian.com/",
        "https://walmart.com/",
        "https://target.com/",
        "https://sears.com/",
        "https://bestbuy.com/",
        "https://macys.com/",
        "https://lowes.com/",
        "https://homdepot.com/",
        "https://jcpenny.com/",
        "https://kohls.com/",
        "https://starbucks.com/",
        "https://zappos.com/",
        "https://ikea.com/",
        "https://nike.com/",
        "https://adidas.com/",
        "https://underarmour.com/",
        "https://puma.com/",
        "https://sony.com/",
        "https://samsung.com/",
        "https://panasonic.com/",
        "https://lg.com/",
        "https://pepsico.com/",
        "https://cocacola.com/",
        "https://mcdonalds.com/",
        "https://burgerking.com/",
        "https://pizzahut.com/",
        "https://dominos.com/",
        "https://kfc.com/",
        "https://subway.com/",
        "https://reuters.com/",
        "https://time.com/",
        "https://forbes.com/",
        "https://businessinsider.com/",
        "https://bloomberg.com/",
        "https://wsj.com/",
        "https://usatoday.com/",
        "https://newsweek.com/",
        "https://nbcnews.com/",
        "https://dailymail.co.uk/",
        "https://thetimes.co.uk/",
        "https://nationalgeographic.com/",
        "https://npr.org/",
        "https://techcrunch.com/",
        "https://engadget.com/",
        "https://wired.com/",
        "https://gizmodo.com/",
        "https://theverge.com/",
        "https://slashdot.org/",
        "https://fiverr.com/",
        "https://upwork.com/",
        "https://toptal.com/",
        "https://glassdoor.com/",
        "https://indeed.com/",
        "https://monster.com/",
        "https://simplyhired.com/",
        "https://zillow.com/",
        "https://realtor.com/",
        "https://trulia.com/",
        "https://redfin.com/",
        "https://apartments.com/",
        "https://rent.com/",
        "https://cars.com/",
        "https://autotrader.com/",
        "https://kbb.com/",
        "https://carvana.com/",
        "https://truecar.com/",
        "https://edmunds.com/",
        "https://orbitz.com/",
        "https://priceline.com/",
        "https://hotels.com/",
        "https://booking.com/",
        "https://travelocity.com/",
        "https://kayak.com/",
        "https://jetblue.com/",
        "https://southwest.com/",
        "https://united.com/",
        "https://delta.com/",
        "https://americanairlines.com/",
        "https://spirit.com/",
        "https://gamestop.com/",
        "https://ign.com/",
        "https://gamespot.com/",
        "https://twitch.tv/",
        "https://steampowered.com/",
        "https://epicgames.com/",
        "https://ea.com/",
        "https://blizzard.com/",
        "https://rockstargames.com/",
        "https://nintendo.com/",
        "https://playstation.com/",
        "https://xbox.com/",
        "https://sega.com/",
        "https://bethesda.net/",
        "https://riotgames.com/",
        "https://ubisoft.com/",
        "https://activision.com/",
        "https://capcom.com/",
        "https://square-enix.com/",
        "https://bioware.com/",
        "https://zynga.com/",
        "https://supercell.com/",
        "https://king.com/",
        "https://moonton.com/",
        "https://zenithbank.com/",
        "https://cbsnews.com/",
        "https://weather.com/",
        "https://accuweather.com/",
        "https://nationalweather.org/",
        "https://healthline.com/",
        "https://mayoclinic.org/",
        "https://webmd.com/",
        "https://nih.gov/",
        "https://cdc.gov/",
        "https://who.int/",
        "https://medicalnewstoday.com/",
        "https://sciencedaily.com/",
        "https://sciencemag.org/",
        "https://nature.com/",
        "https://arxiv.org/",
        "https://jstor.org/",
        "https://academia.edu/",
        "https://researchgate.net/",
        "https://springer.com/",
        "https://elsevier.com/",
        "https://wiley.com/",
        "https://tandfonline.com/",
        "https://sciencedirect.com/",
        "https://moodle.org/",
        "https://khanacademy.org/",
        "https://edx.org/",
        "https://coursera.org/",
        "https://udemy.com/",
        "https://skillshare.com/",
        "https://lynda.com/",
        "https://linuxfoundation.org/",
        "https://gnu.org/",
        "https://apache.org/",
        "https://opensource.org/",
        "https://mozilla.org/",
        "https://howstuffworks.com/",
        "https://ehow.com/",
        "https://diy.org/",
        "https://thisoldhouse.com/",
        "https://gutenberg.org/",
        "https://archive.org/",
        "https://smithsonianmag.com/",
        "https://duolingo.com/",
        "https://rosettastone.com/",
        "https://babbel.com/",
        "https://memrise.com/",
        "https://busuu.com/",
        "https://livemocha.com/",
        "https://cloud.google.com/",
        "https://developers.google.com/",
        "https://openai.com/",
        "https://stackoverflow.com/",
        "https://stackexchange.com/",
        "https://mathworks.com/",
        "https://oracle.com/",
        "https://ibm.com/",
        "https://nvidia.com/",
        "https://amd.com/",
        "https://intel.com/",
        "https://cisco.com/",
        "https://salesforce.com/",
        "https://zoom.us/",
        "https://slack.com/",
        "https://asana.com/",
        "https://trello.com/",
        "https://notion.so/",
        "https://figma.com/",
        "https://canva.com/",
        "https://dribbble.com/",
        "https://behance.net/",
        "https://unsplash.com/",
        "https://pexels.com/",
        "https://producthunt.com/",
        "https://crunchbase.com/",
        "https://angel.co/",
        "https://glassdoor.ca/",
        "https://indeed.ca/",
        "https://scholastic.com/",
        "https://intuit.com/",
        "https://quickbooks.intuit.com/",
        "https://mint.intuit.com/",
        "https://bankofamerica.com/",
        "https://chase.com/",
        "https://wellsfargo.com/",
        "https://capitalone.com/",
        "https://americanexpress.com/",
        "https://td.com/",
        "https://hsbc.com/",
        "https://barclays.co.uk/",
        "https://bbc.co.uk/",
        "https://ft.com/",
        "https://economist.com/",
        "https://nature.org/",
        "https://nasa.gov/",
        "https://esa.int/",
        "https://noaa.gov/",
        "https://mit.edu/",
        "https://stanford.edu/",
        "https://harvard.edu/",
        "https://berkeley.edu/",
        "https://ox.ac.uk/",
        "https://cam.ac.uk/",
        "https://columbia.edu/",
        "https://princeton.edu/",
        "https://yale.edu/",
        "https://ucla.edu/",
        "https://nyu.edu/",
        "https://usc.edu/",
        "https://duke.edu/",
        "https://northwestern.edu/",
        "https://uchicago.edu/",
        "https://upenn.edu/",
        "https://cornell.edu/",
        "https://brown.edu/",
        "https://dartmouth.edu/",
        "https://caltech.edu/",
        "https://utoronto.ca/",
        "https://mcgill.ca/",
        "https://ualberta.ca/",
        "https://ubc.ca/",
        "https://sfu.ca/",
        "https://utoronto.ca/",
        "https://uottawa.ca/",
        "https://queensu.ca/",
        "https://ucdavis.edu/",
        "https://uci.edu/",
        "https://ucsd.edu/",
        "https://colorado.edu/",
        "https://illinois.edu/",
        "https://utexas.edu/",
        "https://umich.edu/",
        "https://umn.edu/",
        "https://osaka-u.ac.jp/",
        "https://tokyo-u.ac.jp/",
        "https://kyoto-u.ac.jp/",
        "https://kaist.ac.kr/",
        "https://postech.ac.kr/",
        "https://nus.edu.sg/",
        "https://ntu.edu.sg/",
        "https://unimelb.edu.au/",
        "https://uq.edu.au/",
        "https://unisa.edu.au/",
        "https://harveynorman.com.au/",
        "https://bunnings.com.au/",
        "https://woolworths.com.au/",
        "https://coles.com.au/",
        "https://aldi.com.au/",
        "https://flipkart.com/",
        "https://snapdeal.com/",
        "https://paytm.com/",
        "https://zomato.com/",
        "https://swiggy.com/",
        "https://mercadolibre.com/",
        "https://mercadopago.com/",
        "https://bbva.com/",
        "https://santander.com/",
        "https://banamex.com/",
        "https://coppel.com/",
        "https://liverpool.com.mx/",
        "https://linio.com/",
        "https://afip.gob.ar/",
        "https://clarin.com/",
        "https://lanacion.com.ar/",
        "https://petrobras.com.br/",
        "https://uol.com.br/",
        "https://globo.com/",
        "https://gob.mx/",
        "https://bukalapak.com/",
        "https://tokopedia.com/",
        "https://lazada.com/",
        "https://shopee.com/",
        "https://jd.com/",
        "https://baidu.com/",
        "https://douban.com/",
        "https://xiaomi.com/",
        "https://oppo.com/",
        "https://huawei.com/",
        "https://vivo.com/",
        "https://realme.com/",
        "https://lenovo.com/",
        "https://asrock.com/",
        "https://msi.com/",
        "https://acer.com/",
        "https://asus.com/",
        "https://dell.com/",
        "https://hp.com/",
        "https://westernunion.com/",
        "https://moneygram.com/",
        "https://transferwise.com/",
        "https://wise.com/",
        "https://coinbase.com/",
        "https://binance.com/",
        "https://kraken.com/",
        "https://btc.com/",
        "https://ethereum.org/",
        "https://bitcoin.org/",
    ];
}

/// Get a random website from a static precompiled list.
pub fn spoof_referrer() -> &'static str {
    spoof_referrer_rng(&mut rand::rng())
}

/// Get a random website from a static precompiled list.
pub fn spoof_referrer_rng<R: Rng>(rng: &mut R) -> &'static str {
    WEBSITES[rng.random_range(..WEBSITES.len())]
}

/// Takes a URL and returns a convincing Google referer URL using the domain name or IP. Not used in latest chrome versions.
///
/// Handles:
/// - Domain names with or without subdomains
/// - IP addresses (removes periods)
///
/// # Examples
/// ```
/// use spider_fingerprint::spoof_refererer::spoof_referrer_google;
/// use url::Url;
///
/// let url = Url::parse("https://www.example.com/test").unwrap();
/// assert_eq!(spoof_referrer_google(&url), Some("https://www.google.com/search?q=example".to_string()));
///
/// let url = Url::parse("http://192.168.1.1/").unwrap();
/// assert_eq!(spoof_referrer_google(&url), Some("https://www.google.com/search?q=19216811".to_string()));
/// ```
pub fn spoof_referrer_google(parsed: &url::Url) -> Option<String> {
    let host = parsed.host_str()?;

    // Strip www. if present
    let stripped = host.strip_prefix("www.").unwrap_or(host);

    // Handle IPv4
    if stripped.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return Some(format!(
            "https://www.google.com/search?q={}",
            stripped.replace('.', "")
        ));
    }

    // Handle IPv6: remove colons and brackets
    if stripped.contains(':') {
        let cleaned = stripped.replace(['[', ']', ':'], "");
        if !cleaned.is_empty() {
            return Some(format!("https://www.google.com/search?q={}", cleaned));
        } else {
            return None;
        }
    }

    // Handle domain names
    let labels: Vec<&str> = stripped.split('.').collect();
    if labels.len() >= 2 {
        Some(format!("https://www.google.com/search?q={}", labels[0]))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use url::Url;

    #[test]
    fn test_standard_domain() {
        let url = Url::parse("https://www.example.com/test").unwrap();
        let result = spoof_referrer_google(&url);
        assert_eq!(
            result,
            Some("https://www.google.com/search?q=example".to_string())
        );
    }

    #[test]
    fn test_domain_without_www() {
        let url = Url::parse("https://example.com").unwrap();
        let result = spoof_referrer_google(&url);
        assert_eq!(
            result,
            Some("https://www.google.com/search?q=example".to_string())
        );
    }

    #[test]
    fn test_subdomain() {
        let url = Url::parse("https://blog.shop.site.org").unwrap();
        let result = spoof_referrer_google(&url);
        assert_eq!(
            result,
            Some("https://www.google.com/search?q=blog".to_string())
        );
    }

    #[test]
    fn test_ip_address() {
        let url = Url::parse("http://192.168.1.1/").unwrap();
        let result = spoof_referrer_google(&url);
        assert_eq!(
            result,
            Some("https://www.google.com/search?q=19216811".to_string())
        );
    }

    #[test]
    fn test_ipv6_address() {
        let url = Url::parse("http://[::1]/").unwrap();
        let result = spoof_referrer_google(&url);
        assert_eq!(
            result,
            Some("https://www.google.com/search?q=1".to_string())
        );
    }

    #[test]
    fn test_localhost() {
        let url = Url::parse("http://localhost").unwrap();
        let result = spoof_referrer_google(&url);
        assert_eq!(result, None);
    }

    #[test]
    fn test_invalid_url() {
        let url = Url::parse("http:///invalid").unwrap();
        let result = spoof_referrer_google(&url);
        assert_eq!(result, None);
    }
}
