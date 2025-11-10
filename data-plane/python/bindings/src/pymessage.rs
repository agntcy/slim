// Copyright AGNTCY Contributors
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;

use pyo3::prelude::*;
use pyo3_stub_gen::derive::{gen_stub_pyclass, gen_stub_pymethods};
use slim_datapath::api::ProtoMessage;
use slim_datapath::messages::Name;
use slim_service::{MessageContext, ServiceError};

use crate::utils::PyName;

/// Python-visible context accompanying every received message.
///
/// Provides routing and descriptive metadata needed for replying,
/// auditing, and instrumentation.
///
/// This type implements `From<MessageContext>` and `Into<MessageContext>`
/// for seamless conversion with the common core message context type.
///
/// Fields:
/// * `source_name`: Fully-qualified sender identity.
/// * `destination_name`: Fully-qualified destination identity (may be an empty placeholder
///   when not explicitly set, e.g. broadcast/group scenarios).
/// * `payload_type`: Logical/semantic type (defaults to "msg" if unspecified).
/// * `metadata`: Arbitrary key/value pairs supplied by the sender (e.g. tracing IDs).
/// * `input_connection`: Numeric identifier of the inbound connection carrying the message.
#[gen_stub_pyclass]
#[pyclass(name = "MessageContext")]
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
    #[pyo3(get)]
    pub identity: String,
}

impl PyMessageContext {
    /// Build a `MessageContext` plus the raw payload bytes from a low-level
    /// `ProtoMessage`. Uses the common MessageContext implementation and
    /// automatic conversion via the `From` trait.
    pub fn from_proto_message(msg: ProtoMessage) -> Result<(Self, Vec<u8>), ServiceError> {
        let (ctx, payload) = MessageContext::from_proto_message(msg)?;
        Ok((ctx.into(), payload))
    }
}

impl From<MessageContext> for PyMessageContext {
    /// Convert a common MessageContext into a Python-specific MessageContext
    fn from(ctx: MessageContext) -> Self {
        PyMessageContext {
            source_name: PyName::from(ctx.source_name),
            destination_name: PyName::from(
                ctx.destination_name
                    .unwrap_or_else(|| Name::from_strings(["", "", ""])),
            ),
            payload_type: ctx.payload_type,
            metadata: ctx.metadata,
            input_connection: ctx.input_connection,
            identity: ctx.identity,
        }
    }
}

impl From<PyMessageContext> for MessageContext {
    /// Convert a Python-specific MessageContext back into a common MessageContext
    fn from(py_ctx: PyMessageContext) -> Self {
        MessageContext::new(
            py_ctx.source_name.into(),
            Some(py_ctx.destination_name.into()),
            py_ctx.payload_type,
            py_ctx.metadata,
            py_ctx.input_connection,
            py_ctx.identity,
        )
    }
}

#[gen_stub_pymethods]
#[pymethods]
impl PyMessageContext {
    /// Prevent direct construction from Python. `MessageContext` instances
    /// are created internally when messages are received from the service.
    #[new]
    pub fn new_py() -> PyResult<Self> {
        Err(pyo3::exceptions::PyException::new_err(
            "Cannot construct PyMessageContext directly",
        ))
    }
}
