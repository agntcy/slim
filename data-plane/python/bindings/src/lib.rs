// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

mod build_info;
mod pyapp;
mod pyidentity;
mod pymessage;
mod pysession;
mod utils;

use pyo3::prelude::*;
use pyo3_stub_gen::define_stub_info_gatherer;

use slim_config::tls::provider;

#[pymodule]
mod _slim_bindings {
    use super::*;

    #[pymodule_export]
    use pyapp::PyApp;

    #[pymodule_export]
    use pysession::{PyCompletionHandle, PySessionConfiguration, PySessionContext, PySessionType};

    #[pymodule_export]
    use pymessage::PyMessageContext;

    #[pymodule_export]
    use utils::{PyName, init_tracing};

    #[pymodule_export]
    use pyidentity::{
        PyAlgorithm, PyIdentityProvider, PyIdentityVerifier, PyKey, PyKeyData, PyKeyFormat,
    };

    #[pymodule_init]
    fn module_init(m: &Bound<'_, PyModule>) -> PyResult<()> {
        // initialize crypto provider
        provider::initialize_crypto_provider();

        m.add("__version__", build_info::BUILD_INFO.version)?;
        m.add("build_profile", build_info::BUILD_INFO.profile)?;
        m.add("build_info", build_info::BUILD_INFO.to_string())?;
        m.add("SESSION_UNSPECIFIED", pysession::SESSION_UNSPECIFIED)?;
        Ok(())
    }
}

// Define a function to gather stub information.
define_stub_info_gatherer!(stub_info);
