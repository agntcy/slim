// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use display_error_chain::ErrorChainExt;
use opentelemetry::propagation::{Extractor, Injector};
use opentelemetry::trace::TraceContextExt;
use tracing::{Span, error};
use tracing_opentelemetry::OpenTelemetrySpanExt;

use crate::api::ProtoPublishType as PublishType;
use crate::api::proto::dataplane::v1::Message;

pub(crate) enum SpanTarget {
    Connection(u64),
    Fanout { subscribers: u32 },
}

struct MetadataExtractor<'a>(&'a std::collections::HashMap<String, String>);

impl Extractor for MetadataExtractor<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).map(|s| s.as_str())
    }

    fn keys(&self) -> Vec<&str> {
        self.0.keys().map(|s| s.as_str()).collect()
    }
}

struct MetadataInjector<'a>(&'a mut std::collections::HashMap<String, String>);

impl Injector for MetadataInjector<'_> {
    fn set(&mut self, key: &str, value: String) {
        self.0.insert(key.to_string(), value);
    }
}

fn extract_parent_context(msg: &Message) -> Option<opentelemetry::Context> {
    let extractor = MetadataExtractor(&msg.metadata);
    let parent_context =
        opentelemetry::global::get_text_map_propagator(|propagator| propagator.extract(&extractor));

    if parent_context.span().span_context().is_valid() {
        Some(parent_context)
    } else {
        None
    }
}

fn inject_current_context(msg: &mut Message) {
    let cx = tracing::Span::current().context();
    let mut injector = MetadataInjector(&mut msg.metadata);
    opentelemetry::global::get_text_map_propagator(|propagator| {
        propagator.inject_context(&cx, &mut injector)
    });
}

fn create_span(function: &str, service_id: &str, msg: &Message, target: SpanTarget) -> Span {
    let span = match target {
        SpanTarget::Connection(connection_id) => tracing::span!(
            tracing::Level::INFO,
            "slim_process_message",
            function = function,
            service_id = %service_id,
            source = %msg.get_source(),
            destination = %msg.get_dst(),
            connection_id = connection_id,
            message_type = %msg.get_type(),
            telemetry = true
        ),
        SpanTarget::Fanout { subscribers } => tracing::span!(
            tracing::Level::INFO,
            "slim_process_message",
            function = function,
            service_id = %service_id,
            source = %msg.get_source(),
            destination = %msg.get_dst(),
            fanout_subscribers = subscribers,
            connection_id = 0u64,
            message_type = %msg.get_type(),
            telemetry = true
        ),
    };

    if let PublishType(_) = msg.get_type() {
        span.set_attribute("session_type", msg.get_session_message_type().as_str_name());
        span.set_attribute(
            "session_id",
            msg.get_session_header().get_session_id() as i64,
        );
        span.set_attribute(
            "message_id",
            msg.get_session_header().get_message_id() as i64,
        );
    }

    span
}

fn attach_trace(msg: &mut Message, span: Span, parent: Option<opentelemetry::Context>) {
    if let Some(ctx) = parent
        && let Err(e) = span.set_parent(ctx)
    {
        error!(error = %e.chain(), "error setting parent context");
    }
    let _guard = span.enter();
    inject_current_context(msg);
}

/// Prepare outbound OTEL context for a single-destination send.
pub(crate) fn prepare_outbound_msg(
    msg: &mut Message,
    function: &str,
    service_id: &str,
    target: SpanTarget,
) {
    if !msg.is_traceable() {
        return;
    }

    let parent = extract_parent_context(msg);
    let span = create_span(function, service_id, msg, target);
    attach_trace(msg, span, parent);
}

/// Prepare OTEL context for a fan-out send.
pub(crate) fn prepare_fanout_msg(
    msg: &mut Message,
    function: &str,
    service_id: &str,
    subscriber_count: u32,
) {
    if !msg.is_traceable() {
        return;
    }

    let parent = extract_parent_context(msg);
    let span = create_span(
        function,
        service_id,
        msg,
        SpanTarget::Fanout {
            subscribers: subscriber_count,
        },
    );
    attach_trace(msg, span, parent);
}

/// Prepare OTEL context on an inbound message (receive path).
pub(crate) fn prepare_inbound_msg(
    msg: &mut Message,
    function: &str,
    service_id: &str,
    conn_index: u64,
    is_local: bool,
) {
    let parent = if is_local {
        None
    } else {
        extract_parent_context(msg)
    };
    let span = create_span(
        function,
        service_id,
        msg,
        SpanTarget::Connection(conn_index),
    );
    attach_trace(msg, span, parent);
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::api::ProtoName;
    use opentelemetry::trace::TracerProvider as _;
    use opentelemetry_sdk::propagation::TraceContextPropagator;
    use opentelemetry_sdk::trace::{Sampler, SdkTracerProvider};
    use tracing_subscriber::Registry;
    use tracing_subscriber::layer::SubscriberExt;

    const ZERO_TRACE_ID: &str = "00000000000000000000000000000000";

    fn build_publish() -> Message {
        let source = ProtoName::from_strings(["org", "ns", "src"]).with_id(1);
        let destination = ProtoName::from_strings(["org", "ns", "dst"]).with_id(2);
        Message::builder()
            .source(source)
            .destination(destination)
            .build_publish()
            .expect("failed to build publish message")
    }

    /// Build a scoped subscriber wired to an always-sampling OpenTelemetry
    /// tracer so spans created under it carry a valid trace context.
    fn otel_subscriber() -> (impl tracing::Subscriber + Send + Sync, SdkTracerProvider) {
        let provider = SdkTracerProvider::builder()
            .with_sampler(Sampler::AlwaysOn)
            .build();
        let tracer = provider.tracer("otel-tracing-test");
        let subscriber =
            Registry::default().with(tracing_opentelemetry::OpenTelemetryLayer::new(tracer));
        (subscriber, provider)
    }

    fn trace_id_from_traceparent(traceparent: &str) -> String {
        // W3C traceparent: "<version>-<trace-id>-<parent-id>-<flags>".
        traceparent
            .split('-')
            .nth(1)
            .expect("traceparent missing trace-id field")
            .to_string()
    }

    #[test]
    fn outbound_injects_traceparent_that_inbound_extracts() {
        opentelemetry::global::set_text_map_propagator(TraceContextPropagator::new());
        let (subscriber, _provider) = otel_subscriber();

        let mut msg = build_publish();
        assert!(
            !msg.metadata.contains_key("traceparent"),
            "fresh message should not carry a traceparent"
        );

        tracing::subscriber::with_default(subscriber, || {
            let root = tracing::info_span!("root");
            let _guard = root.enter();
            prepare_outbound_msg(&mut msg, "send", "svc", SpanTarget::Connection(1));
        });

        let traceparent = msg
            .metadata
            .get("traceparent")
            .expect("outbound path should inject a traceparent");
        let injected_trace_id = trace_id_from_traceparent(traceparent);
        assert_ne!(
            injected_trace_id, ZERO_TRACE_ID,
            "injected trace id should be valid"
        );

        // Receive side: the parent context is recovered from the metadata.
        let parent = extract_parent_context(&msg).expect("inbound path should recover a parent");
        let extracted_trace_id = parent.span().span_context().trace_id().to_string();
        assert_eq!(
            extracted_trace_id, injected_trace_id,
            "round-tripped trace id must match the injected one"
        );
    }

    #[test]
    fn non_traceable_message_is_not_annotated() {
        opentelemetry::global::set_text_map_propagator(TraceContextPropagator::new());
        let (subscriber, _provider) = otel_subscriber();

        let source = ProtoName::from_strings(["org", "ns", "src"]).with_id(1);
        let destination = ProtoName::from_strings(["org", "ns", "dst"]).with_id(2);
        let mut ack = Message::builder()
            .source(source)
            .destination(destination)
            .build_subscription_ack(1, true, "");
        assert!(!ack.is_traceable());

        tracing::subscriber::with_default(subscriber, || {
            let root = tracing::info_span!("root");
            let _guard = root.enter();
            prepare_outbound_msg(&mut ack, "send", "svc", SpanTarget::Connection(1));
        });

        assert!(
            !ack.metadata.contains_key("traceparent"),
            "non-traceable messages must not be annotated with trace context"
        );
    }

    #[test]
    fn extract_returns_none_without_context() {
        opentelemetry::global::set_text_map_propagator(TraceContextPropagator::new());
        let msg = build_publish();
        assert!(
            extract_parent_context(&msg).is_none(),
            "a message with no trace metadata has no parent context"
        );
    }
}
