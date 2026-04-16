//! Tests verifying the handler's behavior with EventRequestPaused listener
//! registration and the race between handler auto-handling and user listeners.

#[cfg(feature = "chrome")]
mod handler_race_tests {
    use chromiumoxide::cdp::browser_protocol::fetch::EventRequestPaused;
    use chromiumoxide::cdp::browser_protocol::network::{
        Headers, Request, RequestId, RequestReferrerPolicy, ResourcePriority, ResourceType,
    };
    use chromiumoxide::handler::network::{NetworkEvent, NetworkManager};
    use std::time::Duration;

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

    fn drain(nm: &mut NetworkManager) {
        while nm.poll().is_some() {}
    }

    fn get_cdp_response_method(nm: &mut NetworkManager) -> Option<String> {
        while let Some(ev) = nm.poll() {
            if let NetworkEvent::SendCdpRequest((method, _)) = &ev {
                let m: &str = method.as_ref();
                if m == "Fetch.continueRequest"
                    || m == "Fetch.failRequest"
                    || m == "Fetch.fulfillRequest"
                {
                    return Some(m.to_string());
                }
            }
        }
        None
    }

    /// Simulates old goto_with_html_once had_interception=false path:
    /// - No set_request_interception called (user flag = false, protocol flag = false)
    /// - Listener registered → enable_request_intercept() → protocol = true
    /// - Explicit Fetch.enable sent (not simulated here, only CDP-level)
    /// - Guard: false && true = false → handler processes event
    ///
    /// This proves the handler sends continueRequest BEFORE the listener can
    /// fulfill the request — a race that the listener loses.
    #[test]
    fn no_intercept_with_listener_handler_sends_continue() {
        let mut nm = NetworkManager::new(false, Duration::from_secs(30));
        nm.set_page_url("https://example.com/".to_string());

        // Simulate listener registration (sets protocol flag)
        nm.enable_request_intercept();

        drain(&mut nm);

        let event = make_request_paused("https://example.com/page.html", ResourceType::Document);
        nm.on_fetch_request_paused(&event);

        let response = get_cdp_response_method(&mut nm);
        println!(
            "Handler response for Document (no intercept, with listener): {:?}",
            response
        );

        // The handler sends continueRequest because:
        // user_request_interception_enabled = false (never called set_request_interception)
        // protocol_request_interception_enabled = true (from enable_request_intercept)
        // Guard: false && true = false → handler processes → sends continueRequest
        assert_eq!(
            response.as_deref(),
            Some("Fetch.continueRequest"),
            "Handler should send continueRequest for Document when user flag is false"
        );
    }

    /// Simulates new goto_with_html_once behavior (both paths unified):
    /// - set_request_interception(true) was called during target init (when chrome_intercept enabled)
    /// - Listener registered → enable_request_intercept() → protocol = true
    /// - Guard: true && true = true → handler returns early (no response)
    ///
    /// In this case only the listener can handle the event, which is what we want
    /// for fulfillRequest to work without racing against continueRequest.
    #[test]
    fn intercept_with_listener_handler_defers() {
        let mut nm = NetworkManager::new(false, Duration::from_secs(30));
        nm.set_page_url("https://example.com/".to_string());

        // HandlerConfig init with request_intercept = true
        nm.set_request_interception(true);
        // Listener registration
        nm.enable_request_intercept();

        drain(&mut nm);

        let event = make_request_paused("https://example.com/page.html", ResourceType::Document);
        nm.on_fetch_request_paused(&event);

        let response = get_cdp_response_method(&mut nm);
        println!(
            "Handler response for Document (intercept + listener): {:?}",
            response
        );

        // Handler defers (both flags true) → listener handles exclusively
        assert_eq!(
            response, None,
            "Handler should NOT respond when both flags are true — listener handles it"
        );
    }

    /// When chrome_intercept is enabled but no listener registered,
    /// the handler processes normally (only user flag true, protocol false).
    #[test]
    fn intercept_no_listener_handler_processes() {
        let mut nm = NetworkManager::new(false, Duration::from_secs(30));
        nm.set_page_url("https://example.com/".to_string());

        nm.set_request_interception(true);
        // No listener registration

        drain(&mut nm);

        let event = make_request_paused("https://example.com/page.html", ResourceType::Document);
        nm.on_fetch_request_paused(&event);

        let response = get_cdp_response_method(&mut nm);
        assert_eq!(
            response.as_deref(),
            Some("Fetch.continueRequest"),
            "Without listener, handler should process Document request normally"
        );
    }
}
