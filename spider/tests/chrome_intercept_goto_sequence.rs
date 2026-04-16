//! Tests verifying the exact flag state sequence in goto_with_html_once.
//! This simulates the handler-side effects of each CDP operation in order.

#[cfg(feature = "chrome")]
mod goto_sequence_tests {
    use chromiumoxide::cdp::browser_protocol::fetch::EventRequestPaused;
    use chromiumoxide::cdp::browser_protocol::network::{
        Headers, Request, RequestId, RequestReferrerPolicy, ResourcePriority, ResourceType,
    };
    use chromiumoxide::handler::network::{NetworkEvent, NetworkManager};
    use std::time::Duration;

    fn make_request_paused(url: &str, rt: ResourceType) -> EventRequestPaused {
        EventRequestPaused {
            request_id: RequestId::from("test-req".to_string()).into(),
            request: Request {
                url: url.to_string(),
                method: "GET".to_string(),
                headers: Headers::new(serde_json::Value::Object(Default::default())),
                initial_priority: ResourcePriority::Medium,
                referrer_policy: RequestReferrerPolicy::NoReferrer,
                url_fragment: None,
                has_post_data: None,
                post_data_entries: None,
                mixed_content_type: None,
                is_link_preload: None,
                trust_token_params: None,
                is_same_site: Some(true),
                is_ad_related: None,
            },
            frame_id: chromiumoxide::cdp::browser_protocol::page::FrameId::from("f".to_string()),
            resource_type: rt,
            response_error_reason: None,
            response_status_code: None,
            response_status_text: None,
            response_headers: None,
            network_id: None,
            redirected_request_id: None,
        }
    }

    fn drain(nm: &mut NetworkManager) {
        while nm.poll().is_some() {}
    }

    fn handler_emits_response(nm: &mut NetworkManager) -> bool {
        let mut found = false;
        while let Some(ev) = nm.poll() {
            if let NetworkEvent::SendCdpRequest((method, _)) = &ev {
                let m: &str = method.as_ref();
                if m.starts_with("Fetch.") {
                    found = true;
                }
            }
        }
        found
    }

    /// Simulate the full goto_with_html_once sequence for the COMMON case:
    /// chrome_intercept is NOT enabled (had_interception = false).
    ///
    /// Initial state: user=false, protocol=false (no set_request_interception in target init)
    #[test]
    fn sequence_no_chrome_intercept() {
        let mut nm = NetworkManager::new(false, Duration::from_secs(30));
        nm.set_page_url("https://example.com/".to_string());
        drain(&mut nm);

        // Step 1: page.event_listener::<EventRequestPaused>()
        // → enable_request_intercept() → protocol = true
        nm.enable_request_intercept();

        // Step 2: page.execute(Fetch.enable) — Document-only pattern
        // (This is a CDP command, doesn't affect NetworkManager flags directly)

        // Step 3: page.set_request_interception(false)
        // → EnableInterception(false) → user = !false = true
        // We can't call page.set_request_interception, so simulate directly:
        // In the handler: TargetMessage::EnableInterception(false) → user = !false = true
        // We need a way to set this... but user_request_interception_enabled is pub(crate)
        // Let me use set_request_interception which also calls update_protocol...

        // Actually, set_request_interception on NM sets user directly.
        // But page.set_request_interception sends EnableInterception which only sets user = !enabled.
        // These are different code paths! NM::set_request_interception also calls update_protocol.

        // For this test, we need to simulate the EnableInterception message.
        // Since we can't access the field directly, let's test what we CAN observe:
        // After the sequence, when a Document event arrives, does the handler respond?

        // Simulate: user was false, now we call set_request_interception(true) which
        // sets user = true via NM path. This is different from EnableInterception.
        // The NM path: set user, then call update_protocol.
        // If user = true and protocol = true, update_protocol sees enabled = true,
        // protocol was already true → no change. No side effect.
        nm.set_request_interception(true); // Sets user=true + update_protocol (no-op)
        drain(&mut nm);

        // Now state: user=true, protocol=true → handler defers
        let ev = make_request_paused("https://example.com/page.html", ResourceType::Document);
        nm.on_fetch_request_paused(&ev);
        assert!(
            !handler_emits_response(&mut nm),
            "During goto_with_html_once: handler should defer to listener (no response)"
        );

        // Step 4 (cleanup): page.execute(Fetch.disable) — no NM flag change
        // Step 5 (cleanup): page.set_request_interception(true) → EnableInterception(true) → user = false
        // Simulate via NM: set_request_interception(false) sets user=false + update_protocol
        nm.set_request_interception(false); // Sets user=false + update_protocol
        drain(&mut nm);

        // Now state: user=false, protocol=true → handler processes
        let ev = make_request_paused("https://example.com/page2.html", ResourceType::Document);
        nm.on_fetch_request_paused(&ev);
        assert!(
            handler_emits_response(&mut nm),
            "After cleanup: handler should process events again"
        );
    }

    /// Simulate the full sequence for chrome_intercept ENABLED case.
    ///
    /// Initial state: user=true, protocol=false (set_request_interception(true) in target init)
    #[test]
    fn sequence_with_chrome_intercept() {
        let mut nm = NetworkManager::new(false, Duration::from_secs(30));
        nm.set_page_url("https://example.com/".to_string());

        // Target init: HandlerConfig.request_intercept = true
        nm.set_request_interception(true); // user=true, sends Fetch.enable, protocol stays false
        drain(&mut nm);

        // Verify: handler processes events (user=true, protocol=false → false)
        let ev = make_request_paused("https://example.com/before.html", ResourceType::Document);
        nm.on_fetch_request_paused(&ev);
        assert!(
            handler_emits_response(&mut nm),
            "Before goto: handler should process events normally"
        );

        // Step 1: page.event_listener → enable_request_intercept() → protocol = true
        nm.enable_request_intercept();

        // Step 2: Fetch.enable (Document-only) — CDP only, no NM change

        // Step 3: set_request_interception(false) → user stays true (via EnableInterception)
        // Actually via NM path: set_request_interception(true) would set user=true (already true)
        // The page.set_request_interception(false) → EnableInterception(false) → user = !false = true
        // user is already true, so no change. Both flags true → handler defers.

        // Now state: user=true, protocol=true → handler defers
        let ev = make_request_paused("https://example.com/during.html", ResourceType::Document);
        nm.on_fetch_request_paused(&ev);
        assert!(
            !handler_emits_response(&mut nm),
            "During goto: handler should defer (both flags true)"
        );

        // Cleanup: Fetch.disable + set_request_interception(true) → EnableInterception(true) → user=false
        // Via NM: set_request_interception(false) sets user=false
        nm.set_request_interception(false);
        drain(&mut nm);

        // After cleanup: user=false, protocol=true → handler processes
        let ev = make_request_paused("https://example.com/after.html", ResourceType::Document);
        nm.on_fetch_request_paused(&ev);
        assert!(
            handler_emits_response(&mut nm),
            "After cleanup: handler should process events again"
        );
    }

    /// Verify that the normal Chrome crawl path (no cached content) is unaffected.
    /// goto_with_html_once is NOT called when content=false.
    #[test]
    fn normal_chrome_crawl_unaffected() {
        let mut nm = NetworkManager::new(false, Duration::from_secs(30));
        nm.set_page_url("https://example.com/".to_string());

        // With chrome_intercept
        nm.set_request_interception(true);
        drain(&mut nm);

        // Normal crawl: handler processes all events
        for (url, rt) in [
            ("https://example.com/page.html", ResourceType::Document),
            ("https://example.com/app.js", ResourceType::Script),
            ("https://example.com/img.jpg", ResourceType::Image),
            ("https://example.com/api", ResourceType::Xhr),
        ] {
            let ev = make_request_paused(url, rt);
            nm.on_fetch_request_paused(&ev);
            assert!(
                handler_emits_response(&mut nm),
                "Normal crawl: {} request should be handled",
                url
            );
        }
    }
}
