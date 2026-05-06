// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use display_error_chain::ErrorChainExt;
use opentelemetry::propagation::{Extractor, Injector};
use opentelemetry::trace::TraceContextExt;
use slim_tracing::utils::INSTANCE_ID;
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
            instance_id = %INSTANCE_ID.as_str(),
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
            instance_id = %INSTANCE_ID.as_str(),
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
    msg.clear_slim_header();
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
