// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

mod pyconfig;
mod pyservice;
mod pysession;
mod utils;

use pyconfig::{PyGrpcClientConfig, PyGrpcServerConfig};
use pyo3::prelude::*;
use pyo3_stub_gen::define_stub_info_gatherer;
use pysession::PyStreamingConfiguration;
use tokio::sync::OnceCell;

use crate::pysession::{PyFireAndForgetConfiguration, PyRequestResponseConfiguration};
use agp_service::session;
use pyservice::PyService;
use pyservice::{
    connect, create_ff_session, create_pyservice, create_rr_session, create_streaming_session,
    disconnect, publish, receive, remove_route, run_server, set_route, stop_server, subscribe,
    unsubscribe,
};
use pysession::{PySessionDirection, PySessionInfo};
use utils::PyAgentType;

static TRACING_GUARD: OnceCell<agp_tracing::OtelGuard> = OnceCell::const_new();

async fn init_tracing_impl(log_level: String, enable_opentelemetry: bool) {
    let _ = TRACING_GUARD
        .get_or_init(|| async {
            let mut config = agp_tracing::TracingConfiguration::default().with_log_level(log_level);

            if enable_opentelemetry {
                config = config.clone().enable_opentelemetry();
            }

            let otel_guard = config.setup_tracing_subscriber();

            otel_guard
        })
        .await;
}

#[pyfunction]
#[pyo3(signature = (log_level="info".to_string(), enable_opentelemetry=false,))]
fn init_tracing(py: Python, log_level: String, enable_opentelemetry: bool) {
    let _ = pyo3_async_runtimes::tokio::future_into_py(py, async move {
        Ok(init_tracing_impl(log_level, enable_opentelemetry).await)
    });
}

// mod _pydantic_core {
//     #[allow(clippy::wildcard_imports)]
//     use super::*;

//     #[pymodule_export]
//     use crate::{
//         from_json, list_all_errors, to_json, to_jsonable_python, validate_core_schema, ArgsKwargs, PyMultiHostUrl,
//         PySome, PyUrl, PydanticCustomError, PydanticKnownError, PydanticOmit, PydanticSerializationError,
//         PydanticSerializationUnexpectedValue, PydanticUndefinedType, PydanticUseDefault, SchemaError, SchemaSerializer,
//         SchemaValidator, TzInfo, ValidationError,
//     };

//     #[pymodule_init]
//     fn module_init(m: &Bound<'_, PyModule>) -> PyResult<()> {
//         m.add("__version__", get_pydantic_core_version())?;
//         m.add("build_profile", env!("PROFILE"))?;
//         m.add("build_info", build_info())?;
//         m.add("_recursion_limit", recursion_guard::RECURSION_GUARD_LIMIT)?;
//         m.add("PydanticUndefined", PydanticUndefinedType::new(m.py()))?;
//         Ok(())
//     }
// }

#[pymodule]
fn _agp_bindings(_py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyService>()?;
    m.add_class::<PyAgentType>()?;
    m.add_class::<PySessionInfo>()?;
    m.add_class::<PyFireAndForgetConfiguration>()?;
    m.add_class::<PyRequestResponseConfiguration>()?;
    m.add_class::<PyStreamingConfiguration>()?;
    m.add_class::<PySessionDirection>()?;

    m.add_function(wrap_pyfunction!(create_pyservice, m)?)?;
    m.add_function(wrap_pyfunction!(create_ff_session, m)?)?;
    m.add_function(wrap_pyfunction!(create_rr_session, m)?)?;
    m.add_function(wrap_pyfunction!(create_streaming_session, m)?)?;
    m.add_function(wrap_pyfunction!(subscribe, m)?)?;
    m.add_function(wrap_pyfunction!(unsubscribe, m)?)?;
    m.add_function(wrap_pyfunction!(set_route, m)?)?;
    m.add_function(wrap_pyfunction!(remove_route, m)?)?;
    m.add_function(wrap_pyfunction!(publish, m)?)?;
    m.add_function(wrap_pyfunction!(run_server, m)?)?;
    m.add_function(wrap_pyfunction!(stop_server, m)?)?;
    m.add_function(wrap_pyfunction!(connect, m)?)?;
    m.add_function(wrap_pyfunction!(disconnect, m)?)?;
    m.add_function(wrap_pyfunction!(receive, m)?)?;
    m.add_function(wrap_pyfunction!(init_tracing, m)?)?;

    m.add("SESSION_UNSPECIFIED", session::SESSION_UNSPECIFIED)?;

    Ok(())
}

// Define a function to gather stub information.
define_stub_info_gatherer!(stub_info);
