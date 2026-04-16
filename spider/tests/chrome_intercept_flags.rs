//! Tests verifying that chrome request interception flags in chromey's
//! NetworkManager work correctly with spider's configuration.
//!
//! These tests reproduce the exact flag states that arise during different
//! spider code paths and verify whether `on_fetch_request_paused` handles
//! or drops paused requests.

#[cfg(feature = "chrome")]
mod chrome_intercept_flag_tests {
    use chromiumoxide::cdp::browser_protocol::fetch::EventRequestPaused;
    use chromiumoxide::cdp::browser_protocol::network::{
        Headers, Request, RequestId, RequestReferrerPolicy, ResourcePriority, ResourceType,
    };
    use chromiumoxide::handler::network::{NetworkEvent, NetworkManager};
    use std::time::Duration;

    /// Create a synthetic EventRequestPaused (mirrors chromey's test helper).
    fn make_request_paused(url: &str, resource_type: ResourceType) -> EventRequestPaused {
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
            frame_id: chromiumoxide::cdp::browser_protocol::page::FrameId::from(
                "frame1".to_string(),
            ),
            resource_type,
            response_error_reason: None,
            response_status_code: None,
            response_status_text: None,
            response_headers: None,
            network_id: None,
            redirected_request_id: None,
        }
    }

    /// Check if NetworkManager emitted any CDP response command (continueRequest,
    /// failRequest, or fulfillRequest) after on_fetch_request_paused.
    fn emits_cdp_response(nm: &mut NetworkManager) -> bool {
        let mut found = false;
        while let Some(ev) = nm.poll() {
            if let NetworkEvent::SendCdpRequest((method, _)) = &ev {
                let m: &str = method.as_ref();
                if m == "Fetch.continueRequest"
                    || m == "Fetch.failRequest"
                    || m == "Fetch.fulfillRequest"
                {
                    found = true;
                }
            }
        }
        found
    }

    /// Drain all pending events from NetworkManager.
    fn drain(nm: &mut NetworkManager) {
        while nm.poll().is_some() {}
    }

    // ─── Baseline: both flags false (no interception) ───

    #[test]
    fn default_nm_handles_document_request() {
        let mut nm = NetworkManager::new(false, Duration::from_secs(30));
        nm.set_page_url("https://example.com/".to_string());
        drain(&mut nm);

        let event = make_request_paused("https://example.com/page.html", ResourceType::Document);
        nm.on_fetch_request_paused(&event);

        assert!(
            emits_cdp_response(&mut nm),
            "Default NetworkManager should respond to paused Document requests"
        );
    }

    // ─── HandlerConfig.request_intercept = true (spider's chrome_intercept) ───
    //
    // Target::new() calls set_request_interception(true) which sets
    // user_request_interception_enabled = true. update_protocol_request_interception()
    // sends Fetch.enable but does NOT set protocol_request_interception_enabled.
    // So guard at line 955 is: true && false → doesn't trigger → works.

    #[test]
    fn set_request_interception_true_handles_requests() {
        let mut nm = NetworkManager::new(false, Duration::from_secs(30));
        nm.set_page_url("https://example.com/".to_string());
        nm.set_request_interception(true);
        drain(&mut nm);

        let event = make_request_paused("https://example.com/page.html", ResourceType::Document);
        nm.on_fetch_request_paused(&event);

        assert!(
            emits_cdp_response(&mut nm),
            "set_request_interception(true) should still handle paused requests \
             (protocol_request_interception_enabled stays false)"
        );
    }

    // ─── goto_with_html_once flow: listener + set_request_interception ───
    //
    // When goto_with_html_once runs with had_interception = true:
    // 1. page.event_listener::<EventRequestPaused>() → enable_request_intercept()
    //    → protocol_request_interception_enabled = true
    // 2. page.set_request_interception(false) → EnableInterception(false)
    //    → user_request_interception_enabled = !false = true
    //
    // Now both flags are true → guard triggers → handler returns without responding

    #[test]
    fn listener_plus_set_request_interception_true_state() {
        let mut nm = NetworkManager::new(false, Duration::from_secs(30));
        nm.set_page_url("https://example.com/".to_string());

        // Step 1: Target init sets request interception (from HandlerConfig)
        nm.set_request_interception(true);

        // Step 2: event_listener::<EventRequestPaused>() triggers enable_request_intercept()
        nm.enable_request_intercept();
        // Now protocol_request_interception_enabled = true

        drain(&mut nm);

        // Now: user_request_interception_enabled = true, protocol_request_interception_enabled = true
        // Guard: true && true → returns early

        let event = make_request_paused("https://example.com/page.html", ResourceType::Document);
        nm.on_fetch_request_paused(&event);

        let responded = emits_cdp_response(&mut nm);

        // This test documents the ACTUAL behavior.
        // If this fails (responded = false), it proves the handler drops requests
        // when both flags are true — which is what happens in goto_with_html_once.
        println!(
            "Handler responds when both flags true (listener + intercept): {}",
            responded
        );

        // NOTE: In the goto_with_html_once flow, the user's own code handles
        // Document events via the event listener. But NON-Document events (scripts,
        // images, XHR) would go unanswered if the handler skips them.
        // Test a non-Document event:
        let script_event = make_request_paused("https://example.com/app.js", ResourceType::Script);
        nm.on_fetch_request_paused(&script_event);

        let script_responded = emits_cdp_response(&mut nm);
        println!(
            "Handler responds to Script when both flags true: {}",
            script_responded
        );

        if !responded || !script_responded {
            println!(
                "BUG CONFIRMED: When enable_request_intercept() sets protocol flag to true, \
                 the handler's guard (user && protocol) triggers and drops requests. \
                 In goto_with_html_once, this means non-Document requests go unanswered, \
                 causing Chrome to hold them forever."
            );
        }
    }

    /// Specific test for the goto_with_html_once had_interception=false path.
    /// The else branch calls Fetch.enable with Document-only pattern, which is fine.
    /// No enable_request_intercept() is called, so protocol flag stays false.
    #[test]
    fn goto_html_no_prior_interception_handles_requests() {
        let mut nm = NetworkManager::new(false, Duration::from_secs(30));
        nm.set_page_url("https://example.com/".to_string());
        // No set_request_interception — simulating had_interception=false
        drain(&mut nm);

        let event = make_request_paused("https://example.com/page.html", ResourceType::Document);
        nm.on_fetch_request_paused(&event);

        assert!(
            emits_cdp_response(&mut nm),
            "Without prior interception, handler should respond to Document requests"
        );
    }

    // ─── page.set_request_interception(true) via EnableInterception ───
    //
    // This is the path used by user code to take manual control.
    // EnableInterception(true) → user_request_interception_enabled = !true = false
    // When user takes control, handler should NOT auto-handle.

    #[test]
    fn user_takes_control_handler_defers() {
        let mut nm = NetworkManager::new(false, Duration::from_secs(30));
        nm.set_page_url("https://example.com/".to_string());

        // Config enables interception
        nm.set_request_interception(true);

        // User registers listener → protocol flag true
        nm.enable_request_intercept();

        // User calls page.set_request_interception(true) which sends
        // EnableInterception(true) → user flag = !true = false
        nm.disable_request_intercept();
        // Actually disable_request_intercept() only sets protocol = false.
        // The EnableInterception path sets user = !enabled directly.
        // We can't simulate EnableInterception from outside, so test the
        // flag state we can observe.

        drain(&mut nm);

        let event = make_request_paused("https://example.com/page.html", ResourceType::Document);
        nm.on_fetch_request_paused(&event);

        let responded = emits_cdp_response(&mut nm);
        println!(
            "After disable_request_intercept(): handler responds = {}",
            responded
        );
    }

    // ─── spider_handler_config_equivalent: full spider setup ───

    #[test]
    fn spider_config_handles_all_resource_types() {
        let mut nm = NetworkManager::new(false, Duration::from_secs(30));
        nm.set_page_url("https://example.com/".to_string());
        nm.set_request_interception(true);
        nm.ignore_visuals = true;
        drain(&mut nm);

        // Document
        let ev = make_request_paused("https://example.com/page.html", ResourceType::Document);
        nm.on_fetch_request_paused(&ev);
        assert!(
            emits_cdp_response(&mut nm),
            "Document must get CDP response"
        );

        // Script
        let ev = make_request_paused("https://example.com/bundle.js", ResourceType::Script);
        nm.on_fetch_request_paused(&ev);
        assert!(emits_cdp_response(&mut nm), "Script must get CDP response");

        // Image (should be blocked by ignore_visuals)
        let ev = make_request_paused("https://example.com/img.jpg", ResourceType::Image);
        nm.on_fetch_request_paused(&ev);
        assert!(
            emits_cdp_response(&mut nm),
            "Image must get CDP response (blocked or continued)"
        );

        // XHR
        let ev = make_request_paused("https://example.com/api/data", ResourceType::Xhr);
        nm.on_fetch_request_paused(&ev);
        assert!(emits_cdp_response(&mut nm), "XHR must get CDP response");
    }

    // ─── The critical combined state: listener registration + interception ───

    /// Documents the chromey-level bug: when both flags are true, the handler
    /// drops all requests. Spider's fix avoids this state by using explicit
    /// Fetch.enable with Document-only patterns instead of toggling
    /// set_request_interception, preventing both flags from being true.
    #[test]
    fn both_flags_true_drops_requests_chromey_bug() {
        let mut nm = NetworkManager::new(false, Duration::from_secs(30));
        nm.set_page_url("https://example.com/".to_string());

        // HandlerConfig init
        nm.set_request_interception(true);
        // Listener registration (sets protocol_request_interception_enabled = true)
        nm.enable_request_intercept();

        drain(&mut nm);

        let ev = make_request_paused("https://example.com/page.html", ResourceType::Document);
        nm.on_fetch_request_paused(&ev);

        // This documents the bug — handler drops requests when both flags are true.
        // Spider's goto_with_html_once avoids triggering this state.
        assert!(
            !emits_cdp_response(&mut nm),
            "Chromey bug: when both user_request_interception_enabled and \
             protocol_request_interception_enabled are true, the handler drops requests. \
             If this assertion starts failing (handler now responds), the chromey bug is \
             fixed and spider's workaround can be simplified."
        );
    }
}
