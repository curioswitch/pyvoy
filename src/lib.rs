use std::sync::Arc;

use envoy_proxy_dynamic_modules_rust_sdk::*;
use pyo3::prelude::*;
use pyo3::types::PyList;
use yaml_rust2::YamlLoader;

mod asgi;
/// Helpers for working with Envoy's SDK.
mod envoy;
/// An optimized event bridge between Envoy and Python.
mod eventbridge;
mod types;
mod wsgi;

declare_init_functions!(init, new_http_filter_config_fn);

fn init() -> bool {
    Python::initialize();
    let init_result: Result<_, PyErr> = Python::attach(|py| {
        let syspath = py.import("sys")?.getattr("path")?.cast_into::<PyList>()?;
        syspath.insert(0, ".")?;

        Ok(())
    });

    if let Err(e) = init_result {
        // Shouldn't happen in practice.
        envoy_log_error!("Python runtime initialization failed: {}", e);
        return false;
    }

    if let Some(value) = std::env::var_os("PYVOY_PYDEVD_ARGS") {
        let args = value
            .to_string_lossy()
            .split_whitespace()
            .map(|s| s.to_string())
            .collect::<Vec<String>>();
        if let Err(e) = Python::attach(|py| {
            let pyvoy_debug = py.import("pyvoy._debug")?;
            pyvoy_debug.call_method1("enable_pydev", (args,))?;
            Ok::<_, PyErr>(())
        }) {
            envoy_log_warn!("Failed to enable pydevd, proceeding without debug: {}", e);
        }
    }

    true
}

fn new_http_filter_config_fn<EC: EnvoyHttpFilterConfig, EHF: EnvoyHttpFilter>(
    _envoy_filter_config: &mut EC,
    _filter_name: &str,
    filter_config: &[u8],
) -> Option<Box<dyn HttpFilterConfig<EHF>>> {
    let filter_config = match YamlLoader::load_from_str(std::str::from_utf8(filter_config).unwrap())
    {
        Ok(conf) => conf,
        Err(e) => {
            envoy_log_error!("Failed to parse filter config YAML: {}", e);
            return None;
        }
    };
    if filter_config.is_empty() {
        envoy_log_error!("Filter config is empty");
        return None;
    }
    let filter_config = &filter_config[0];

    let Some(app) = filter_config["app"].as_str() else {
        envoy_log_error!("Filter config missing required 'app' field");
        return None;
    };
    let interface = filter_config["interface"].as_str().unwrap_or("asgi");
    let root_path = filter_config["root_path"].as_str().unwrap_or("");
    let worker_threads = match filter_config["worker_threads"].as_i64() {
        Some(v) => v as usize,
        None => {
            if interface == "asgi" {
                1
            } else {
                200
            }
        }
    };

    let constants = Arc::new(Python::attach(|py| types::Constants::new(py, root_path)));

    match interface {
        "asgi" => asgi::filter::Config::new(app, constants)
            .map(|cfg| Box::new(cfg) as Box<dyn HttpFilterConfig<EHF>>),
        "wsgi" => wsgi::filter::Config::new(app, constants, worker_threads)
            .map(|cfg| Box::new(cfg) as Box<dyn HttpFilterConfig<EHF>>),
        _ => {
            envoy_log_error!("Unsupported python interface: {}", interface);
            None
        }
    }
}
