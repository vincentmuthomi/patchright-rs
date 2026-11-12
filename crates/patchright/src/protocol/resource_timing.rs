//! Resource timing for HTTP requests, shared by the browser-side
//! [`Request::timing`](crate::protocol::Request::timing) and the API-side
//! `APIResponse::timing`.
//!
//! Split into its own module so the pure parse/merge logic sits in mutation
//! scope (`.cargo/mutants.toml`); the protocol objects it serves are only
//! testable against a live driver, which mutation testing excludes.

/// Resource timing information for an HTTP request.
///
/// All time values are in milliseconds relative to the navigation start.
/// A value of `-1` indicates the timing phase was not reached.
///
/// See: <https://playwright.dev/docs/api/class-request#request-timing>
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ResourceTiming {
    /// Request start time in milliseconds since epoch.
    pub start_time: f64,
    /// Time immediately before the browser starts the domain name lookup
    /// for the resource. The value is given in milliseconds relative to
    /// `startTime`, -1 if not available.
    pub domain_lookup_start: f64,
    /// Time immediately after the browser starts the domain name lookup
    /// for the resource. The value is given in milliseconds relative to
    /// `startTime`, -1 if not available.
    pub domain_lookup_end: f64,
    /// Time immediately before the user agent starts establishing the
    /// connection to the server to retrieve the resource.
    pub connect_start: f64,
    /// Time immediately after the browser starts the handshake process
    /// to secure the current connection.
    pub secure_connection_start: f64,
    /// Time immediately after the browser finishes establishing the connection
    /// to the server to retrieve the resource.
    pub connect_end: f64,
    /// Time immediately before the browser starts requesting the resource from
    /// the server, cache, or local resource.
    pub request_start: f64,
    /// Time immediately after the browser starts requesting the resource from
    /// the server, cache, or local resource.
    pub response_start: f64,
    /// Time immediately after the browser receives the last byte of the resource
    /// or immediately before the transport connection is closed, whichever comes first.
    pub response_end: f64,
}

impl ResourceTiming {
    /// Folds the separately-reported response-end time into a timing object.
    ///
    /// The driver builds the timing before the body has finished arriving, so
    /// `responseEnd` is absent there and the real value comes alongside as
    /// `responseEndTiming`. Both the browser-side and API-side paths merge it,
    /// through here, so a `ResourceTiming` means the same thing whichever
    /// origin produced it.
    pub(crate) fn merge_response_end(
        timing: &mut serde_json::Value,
        response_end_timing: Option<f64>,
    ) {
        if let (Some(end), Some(obj)) = (response_end_timing, timing.as_object_mut())
            && let Some(n) = serde_json::Number::from_f64(end)
        {
            obj.insert("responseEnd".to_string(), serde_json::Value::Number(n));
        }
    }

    /// Parses the protocol's `ResourceTiming` shape.
    ///
    /// Every phase is optional on the wire and absent means "not reached",
    /// which Playwright represents as `-1` rather than a missing value. Shared
    /// by [`Request::timing`] and `APIResponse::timing` so the two cannot
    /// disagree about that defaulting. Callers that also have a
    /// `responseEndTiming` should run [`Self::merge_response_end`] first.
    ///
    /// Returns `None` if the value is not a timing object at all.
    pub(crate) fn from_protocol(value: &serde_json::Value) -> Option<Self> {
        use serde::Deserialize;

        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct RawTiming {
            start_time: Option<f64>,
            domain_lookup_start: Option<f64>,
            domain_lookup_end: Option<f64>,
            connect_start: Option<f64>,
            connect_end: Option<f64>,
            secure_connection_start: Option<f64>,
            request_start: Option<f64>,
            response_start: Option<f64>,
            response_end: Option<f64>,
        }

        let raw: RawTiming = serde_json::from_value(value.clone()).ok()?;

        Some(Self {
            start_time: raw.start_time.unwrap_or(-1.0),
            domain_lookup_start: raw.domain_lookup_start.unwrap_or(-1.0),
            domain_lookup_end: raw.domain_lookup_end.unwrap_or(-1.0),
            connect_start: raw.connect_start.unwrap_or(-1.0),
            connect_end: raw.connect_end.unwrap_or(-1.0),
            secure_connection_start: raw.secure_connection_start.unwrap_or(-1.0),
            request_start: raw.request_start.unwrap_or(-1.0),
            response_start: raw.response_start.unwrap_or(-1.0),
            response_end: raw.response_end.unwrap_or(-1.0),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::ResourceTiming;
    use serde_json::json;

    #[test]
    fn absent_phases_become_minus_one() {
        // The driver omits phases that were never reached; Playwright's
        // contract is that those read as -1, not as a missing value.
        let timing = ResourceTiming::from_protocol(&json!({ "startTime": 1000.0 }))
            .expect("an object with only startTime is still a timing");

        assert_eq!(timing.start_time, 1000.0);
        assert_eq!(timing.domain_lookup_start, -1.0);
        assert_eq!(timing.domain_lookup_end, -1.0);
        assert_eq!(timing.connect_start, -1.0);
        assert_eq!(timing.connect_end, -1.0);
        assert_eq!(timing.secure_connection_start, -1.0);
        assert_eq!(timing.request_start, -1.0);
        assert_eq!(timing.response_start, -1.0);
        assert_eq!(timing.response_end, -1.0);
    }

    #[test]
    fn every_phase_is_read_from_its_own_wire_name() {
        // Pins the camelCase mapping: a swapped or misspelled rename would
        // otherwise silently read as -1 and look like "phase not reached".
        let timing = ResourceTiming::from_protocol(&json!({
            "startTime": 1.0,
            "domainLookupStart": 2.0,
            "domainLookupEnd": 3.0,
            "connectStart": 4.0,
            "connectEnd": 5.0,
            "secureConnectionStart": 6.0,
            "requestStart": 7.0,
            "responseStart": 8.0,
            "responseEnd": 9.0,
        }))
        .expect("full timing parses");

        assert_eq!(timing.start_time, 1.0);
        assert_eq!(timing.domain_lookup_start, 2.0);
        assert_eq!(timing.domain_lookup_end, 3.0);
        assert_eq!(timing.connect_start, 4.0);
        assert_eq!(timing.connect_end, 5.0);
        assert_eq!(timing.secure_connection_start, 6.0);
        assert_eq!(timing.request_start, 7.0);
        assert_eq!(timing.response_start, 8.0);
        assert_eq!(timing.response_end, 9.0);
    }

    #[test]
    fn an_empty_timing_defaults_every_phase_including_start() {
        // The other absent-phase test always supplies startTime, so this is
        // the only place start_time's own default is exercised.
        let timing = ResourceTiming::from_protocol(&json!({})).expect("empty object parses");
        assert_eq!(timing.start_time, -1.0);
    }

    #[test]
    fn a_non_object_is_not_a_timing() {
        assert!(ResourceTiming::from_protocol(&json!("nope")).is_none());
        assert!(ResourceTiming::from_protocol(&json!(null)).is_none());
    }
}

#[cfg(test)]
mod merge_tests {
    use super::ResourceTiming;
    use serde_json::json;

    #[test]
    fn response_end_is_folded_into_the_timing() {
        // The driver reports the end separately because the timing object is
        // built before the body finishes. Both origins fold it back in here,
        // so a ResourceTiming means the same thing whichever produced it.
        let mut timing = json!({ "startTime": 100.0, "requestStart": 5.0 });
        ResourceTiming::merge_response_end(&mut timing, Some(42.0));

        let parsed = ResourceTiming::from_protocol(&timing).expect("parses");
        assert_eq!(parsed.response_end, 42.0);
    }

    #[test]
    fn without_a_reported_end_the_phase_stays_unreached() {
        let mut timing = json!({ "startTime": 100.0 });
        ResourceTiming::merge_response_end(&mut timing, None);

        let parsed = ResourceTiming::from_protocol(&timing).expect("parses");
        assert_eq!(parsed.response_end, -1.0);
    }

    #[test]
    fn a_non_object_timing_is_left_alone() {
        let mut timing = json!("not a timing");
        ResourceTiming::merge_response_end(&mut timing, Some(42.0));
        assert_eq!(timing, json!("not a timing"));
    }
}
