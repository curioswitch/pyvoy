use envoy_proxy_dynamic_modules_rust_sdk::EnvoyHttpFilter;

/// The denominator of the trace ID ratio pyvoy configures Envoy's sampler with.
pub(crate) const RATIO_DENOMINATOR: u64 = 1_000_000;

/// Mirrors the sampler configured for Envoy's OpenTelemetry tracer.
///
/// Envoy does not expose whether the active span is sampled, so the decision is
/// recomputed with the same inputs and arithmetic as Envoy's samplers to
/// propagate the span's trace flags to the application.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TraceSampler {
    /// Whether a sampled flag propagated from a parent is used when present.
    pub(crate) parent_based: bool,
    /// The sampling ratio out of RATIO_DENOMINATOR, or None to always sample.
    pub(crate) ratio: Option<u64>,
}

impl TraceSampler {
    fn sampled(&self, trace_id: &str, parent_sampled: Option<bool>) -> bool {
        if self.parent_based
            && let Some(sampled) = parent_sampled
        {
            return sampled;
        }
        match self.ratio {
            None => true,
            Some(numerator) => trace_id_to_u64(trace_id) % RATIO_DENOMINATOR < numerator,
        }
    }
}

/// Matches Envoy's TraceIdRatioBasedSampler, which reads the first 8 bytes of
/// the trace ID in native byte order.
fn trace_id_to_u64(trace_id: &str) -> u64 {
    let Some(hex) = trace_id.get(..16) else {
        return 0;
    };
    let mut bytes = [0u8; 8];
    for (i, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).unwrap_or(0);
    }
    u64::from_ne_bytes(bytes)
}

/// Sets the request's trace context to Envoy's active span so the application
/// continues the trace from it.
pub(crate) fn propagate_active_span<EHF: EnvoyHttpFilter>(
    envoy_filter: &mut EHF,
    sampler: &TraceSampler,
) {
    let Some(span) = envoy_filter.get_active_span() else {
        return;
    };
    let (Some(trace_id), Some(span_id)) = (span.get_trace_id(), span.get_span_id()) else {
        return;
    };
    let parent_sampled = envoy_filter
        .get_request_header_value("traceparent")
        .and_then(|traceparent| parent_sampled(traceparent.as_slice(), &trace_id));
    if parent_sampled.is_none() {
        // Envoy started a new trace, which a tracestate sent without a valid
        // traceparent does not belong to.
        envoy_filter.remove_request_header("tracestate");
    }
    let flags = if sampler.sampled(&trace_id, parent_sampled) {
        "01"
    } else {
        "00"
    };
    let traceparent = format!("00-{trace_id}-{span_id}-{flags}");
    envoy_filter.set_request_header("traceparent", traceparent.as_bytes());
}

/// Returns the sampled flag of the traceparent header when Envoy's span
/// continues the trace.
fn parent_sampled(traceparent: &[u8], trace_id: &str) -> Option<bool> {
    let mut parts = traceparent.split(|b| *b == b'-');
    let _version = parts.next()?;
    let parent_trace_id = parts.next()?;
    let _parent_id = parts.next()?;
    let flags = parts.next()?;
    if !parent_trace_id.eq_ignore_ascii_case(trace_id.as_bytes()) {
        return None;
    }
    let flags = u8::from_str_radix(std::str::from_utf8(flags).ok()?, 16).ok()?;
    Some(flags & 1 == 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TRACE_ID: &str = "4bf92f3577b34da6a3ce929d0e0e4736";

    #[test]
    fn trace_id_to_u64_native_order() {
        let expected = u64::from_ne_bytes([0x4b, 0xf9, 0x2f, 0x35, 0x77, 0xb3, 0x4d, 0xa6]);
        assert_eq!(trace_id_to_u64(TRACE_ID), expected);
        assert_eq!(trace_id_to_u64("4bf9"), 0);
    }

    #[test]
    fn sampled_ratio_ignores_parent() {
        let bucket = trace_id_to_u64(TRACE_ID) % RATIO_DENOMINATOR;
        let included = TraceSampler {
            parent_based: false,
            ratio: Some(bucket + 1),
        };
        let excluded = TraceSampler {
            parent_based: false,
            ratio: Some(bucket),
        };
        assert!(included.sampled(TRACE_ID, Some(false)));
        assert!(!excluded.sampled(TRACE_ID, Some(true)));
        assert!(included.sampled(TRACE_ID, None));
        assert!(!excluded.sampled(TRACE_ID, None));
    }

    #[test]
    fn sampled_parent_based() {
        let sampler = TraceSampler {
            parent_based: true,
            ratio: Some(0),
        };
        assert!(sampler.sampled(TRACE_ID, Some(true)));
        assert!(!sampler.sampled(TRACE_ID, Some(false)));
        assert!(!sampler.sampled(TRACE_ID, None));
        let always_on = TraceSampler {
            parent_based: true,
            ratio: None,
        };
        assert!(always_on.sampled(TRACE_ID, None));
        assert!(!always_on.sampled(TRACE_ID, Some(false)));
    }

    #[test]
    fn parent_sampled_flags() {
        let sampled = format!("00-{TRACE_ID}-00f067aa0ba902b7-01");
        assert_eq!(parent_sampled(sampled.as_bytes(), TRACE_ID), Some(true));
        let unsampled = format!("00-{TRACE_ID}-00f067aa0ba902b7-02");
        assert_eq!(parent_sampled(unsampled.as_bytes(), TRACE_ID), Some(false));
        let uppercase = format!("00-{}-00f067aa0ba902b7-01", TRACE_ID.to_uppercase());
        assert_eq!(parent_sampled(uppercase.as_bytes(), TRACE_ID), Some(true));
        let other_trace = "00-0af7651916cd43dd8448eb211c80319c-00f067aa0ba902b7-01";
        assert_eq!(parent_sampled(other_trace.as_bytes(), TRACE_ID), None);
        assert_eq!(parent_sampled(b"garbage", TRACE_ID), None);
    }
}
