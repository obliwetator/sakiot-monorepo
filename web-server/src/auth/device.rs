//! The browser and platform a sign-in came from, as the signed-in devices list
//! names them. Only these names are stored: never the User-Agent string, which
//! identifies a browser more precisely than the list needs, and never an IP
//! address.

use actix_web::HttpRequest;
use actix_web::http::header::USER_AGENT;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Device {
    pub browser: Option<&'static str>,
    pub platform: Option<&'static str>,
}

/// Checked in order. Chromium-based browsers also name Chrome and Safari, and
/// Chrome names Safari, so the more specific tokens come first.
const BROWSERS: [(&str, &str); 10] = [
    ("Edg/", "Edge"),
    ("EdgA/", "Edge"),
    ("EdgiOS/", "Edge"),
    ("OPR/", "Opera"),
    ("OPiOS/", "Opera"),
    ("SamsungBrowser/", "Samsung Internet"),
    ("Firefox/", "Firefox"),
    ("FxiOS/", "Firefox"),
    ("CriOS/", "Chrome"),
    ("Chrome/", "Chrome"),
];

/// Checked in order: Android also names Linux, ChromeOS names X11 but not
/// Linux, and iPhone names "Mac OS X" but not Macintosh.
const PLATFORMS: [(&str, &str); 7] = [
    ("iPhone", "iPhone"),
    ("iPad", "iPad"),
    ("Android", "Android"),
    ("CrOS", "ChromeOS"),
    ("Windows", "Windows"),
    ("Macintosh", "macOS"),
    ("Linux", "Linux"),
];

impl Device {
    pub fn from_request(req: &HttpRequest) -> Self {
        req.headers()
            .get(USER_AGENT)
            .and_then(|value| value.to_str().ok())
            .map(Self::from_user_agent)
            .unwrap_or_default()
    }

    pub fn from_user_agent(user_agent: &str) -> Self {
        let browser = BROWSERS
            .iter()
            .find(|(token, _)| user_agent.contains(token))
            .map(|(_, name)| *name)
            // Safari is the remaining WebKit browser that reports its version.
            .or_else(|| {
                (user_agent.contains("Version/") && user_agent.contains("Safari/"))
                    .then_some("Safari")
            });
        let platform = PLATFORMS
            .iter()
            .find(|(token, _)| user_agent.contains(token))
            .map(|(_, name)| *name);
        Self { browser, platform }
    }
}

#[cfg(test)]
mod tests {
    use super::Device;

    fn device(user_agent: &str) -> (Option<&'static str>, Option<&'static str>) {
        let device = Device::from_user_agent(user_agent);
        (device.browser, device.platform)
    }

    #[test]
    fn names_common_browsers_and_platforms() {
        let cases = [
            (
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36",
                ("Chrome", "Windows"),
            ),
            (
                "Mozilla/5.0 (Linux; Android 10; K) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Mobile Safari/537.36",
                ("Chrome", "Android"),
            ),
            (
                "Mozilla/5.0 (iPhone; CPU iPhone OS 18_5 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) CriOS/141.0.0.0 Mobile/15E148 Safari/604.1",
                ("Chrome", "iPhone"),
            ),
            (
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.5 Safari/605.1.15",
                ("Safari", "macOS"),
            ),
            (
                "Mozilla/5.0 (iPhone; CPU iPhone OS 18_5 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.5 Mobile/15E148 Safari/604.1",
                ("Safari", "iPhone"),
            ),
            (
                "Mozilla/5.0 (iPad; CPU OS 17_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Mobile/15E148 Safari/604.1",
                ("Safari", "iPad"),
            ),
            (
                "Mozilla/5.0 (X11; Linux x86_64; rv:143.0) Gecko/20100101 Firefox/143.0",
                ("Firefox", "Linux"),
            ),
            (
                "Mozilla/5.0 (Android 15; Mobile; rv:143.0) Gecko/143.0 Firefox/143.0",
                ("Firefox", "Android"),
            ),
            (
                "Mozilla/5.0 (iPhone; CPU iPhone OS 18_5 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) FxiOS/143.0 Mobile/15E148 Safari/605.1.15",
                ("Firefox", "iPhone"),
            ),
            (
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36 Edg/141.0.0.0",
                ("Edge", "Windows"),
            ),
            (
                "Mozilla/5.0 (Linux; Android 10; K) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Mobile Safari/537.36 EdgA/141.0.0.0",
                ("Edge", "Android"),
            ),
            (
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36 OPR/124.0.0.0",
                ("Opera", "Windows"),
            ),
            (
                "Mozilla/5.0 (Linux; Android 14; SAMSUNG SM-S918B) AppleWebKit/537.36 (KHTML, like Gecko) SamsungBrowser/28.0 Chrome/130.0.0.0 Mobile Safari/537.36",
                ("Samsung Internet", "Android"),
            ),
            (
                "Mozilla/5.0 (X11; CrOS x86_64 14541.0.0) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36",
                ("Chrome", "ChromeOS"),
            ),
        ];
        for (user_agent, (browser, platform)) in cases {
            assert_eq!(
                device(user_agent),
                (Some(browser), Some(platform)),
                "{user_agent}"
            );
        }
    }

    #[test]
    fn unrecognized_agents_name_nothing() {
        assert_eq!(device("curl/8.5.0"), (None, None));
        assert_eq!(device(""), (None, None));
    }
}
