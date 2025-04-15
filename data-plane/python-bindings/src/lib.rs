// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

mod build_info;
mod pyservice;
mod pysession;
mod utils;

use pyo3::prelude::*;
use pyo3_stub_gen::define_stub_info_gatherer;

#[pymodule]
mod _agp_bindings {
    use super::*;

    #[pymodule_export]
    use pyservice::{
        PyService, connect, create_ff_session, create_pyservice, create_rr_session,
        create_streaming_session, disconnect, publish, receive, remove_route, run_server,
        set_route, stop_server, subscribe, unsubscribe,
    };

    #[pymodule_export]
    use pysession::{
        PyFireAndForgetConfiguration, PyRequestResponseConfiguration, PySessionDirection,
        PySessionInfo, PyStreamingConfiguration,
    };

    #[pymodule_export]
    use utils::{PyAgentType, init_tracing};

    #[pymodule_init]
    fn module_init(m: &Bound<'_, PyModule>) -> PyResult<()> {
        m.add("__version__", build_info::BUILD_INFO.version)?;
        m.add("build_profile", build_info::BUILD_INFO.profile)?;
        m.add("build_info", build_info::BUILD_INFO.to_string())?;
        m.add("SESSION_UNSPECIFIED", pysession::SESSION_UNSPECIFIED)?;
        Ok(())
    }
}

// Define a function to gather stub information.
define_stub_info_gatherer!(stub_info);
