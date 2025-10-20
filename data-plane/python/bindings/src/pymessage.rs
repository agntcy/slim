// Copyright AGNTCY Contributors
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;

use pyo3::prelude::*;
use pyo3_stub_gen::derive::{gen_stub_pyclass, gen_stub_pymethods};
use slim_datapath::api::ProtoMessage;
use slim_datapath::api::ProtoPublishType;
use slim_datapath::messages::Name;
use slim_service::ServiceError;

use crate::utils::PyName;

/// Python-visible context accompanying every received message.
///
/// Provides routing and descriptive metadata needed for replying,
/// auditing, and instrumentation.
///
/// Fields:
/// * `source_name`: Fully-qualified sender identity.
/// * `destination_name`: Fully-qualified destination identity (may be an empty placeholder
///   when not explicitly set, e.g. broadcast/group scenarios).
/// * `payload_type`: Logical/semantic type (defaults to "msg" if unspecified).
/// * `metadata`: Arbitrary key/value pairs supplied by the sender (e.g. tracing IDs).
/// * `input_connection`: Numeric identifier of the inbound connection carrying the message.
#[gen_stub_pyclass]
#[pyclass]
#[derive(Clone)]
pub struct PyMessageContext {
    #[pyo3(get)]
    pub source_name: PyName,
    #[pyo3(get)]
    pub destination_name: PyName,
    #[pyo3(get)]
    pub payload_type: String,
    #[pyo3(get)]
    pub metadata: HashMap<String, String>,
    #[pyo3(get)]
    pub input_connection: u64,
}

impl PyMessageContext {
    /// Internal constructor used by helper conversion functions. Not exposed
    /// to Python directly; Python code receives already-constructed instances
    /// when consuming messages.
    pub fn new(
        source: Name,
        destination: Option<Name>,
        payload_type: String,
        metadata: HashMap<String, String>,
        input_connection: u64,
    ) -> Self {
        PyMessageContext {
            source_name: PyName::from(source),
            destination_name: PyName::from(
                destination.unwrap_or_else(|| Name::from_strings(["", "", ""])),
            ),
            payload_type,
            metadata,
            input_connection,
        }
    }

    /// Build a `PyMessageContext` plus the raw payload bytes from a low-level
    /// `ProtoMessage`. Returns an error if the message type is unsupported
    /// (i.e. not a publish payload).
    ///
    /// On success:
    /// * The context captures source/destination identities
    /// * `payload_type` defaults to "msg" if unset
    /// * `metadata` is copied from the underlying protocol envelope
    /// * The returned `Vec<u8>` is the raw application payload
    pub fn from_proto_message(msg: ProtoMessage) -> Result<(Self, Vec<u8>), ServiceError> {
        if let Some(ProtoPublishType(publish)) = msg.message_type.as_ref() {
            let source = msg.get_source();
            let destination = Some(msg.get_dst());
            let input_connection = msg.get_incoming_conn();
            let payload_bytes = publish
                .msg
                .as_ref()
                .map(|c| c.as_application_payload().blob.clone())
                .unwrap_or_default();
            let payload_type = publish
                .msg
                .as_ref()
                .map(|c| c.as_application_payload().payload_type.clone())
                .unwrap_or_else(|| "msg".to_string());
            let metadata = msg.get_metadata_map();
            let ctx = PyMessageContext::new(
                source,
                destination,
                payload_type,
                metadata,
                input_connection,
            );
            Ok((ctx, payload_bytes))
        } else {
            Err(ServiceError::ReceiveError(
                "unsupported message type".to_string(),
            ))
        }
    }
}

#[gen_stub_pymethods]
#[pymethods]
impl PyMessageContext {
    /// Prevent direct construction from Python. `PyMessageContext` instances
    /// are created internally when messages are received from the service.
    #[new]
    pub fn new_py() -> PyResult<Self> {
        Err(pyo3::exceptions::PyException::new_err(
            "Cannot construct PyMessageContext directly",
        ))
    }
}
