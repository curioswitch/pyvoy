use std::sync::Arc;

use envoy_proxy_dynamic_modules_rust_sdk::*;
use pyo3::prelude::*;
use pyo3::types::PyList;
use yaml_rust2::YamlLoader;

mod asgi;
/// Helpers for working with Envoy's SDK.
mod envoy;
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
        eprintln!("Python runtime initialization failed: {}", e);
        return false;
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
            eprintln!("pyvoy: failed to parse filter config YAML: {}", e);
            return None;
        }
    };
    let filter_config = &filter_config[0];

    let Some(app) = filter_config["app"].as_str() else {
        eprintln!("pyvoy: config missing required 'app' field");
        return None;
    };
    let interface = filter_config["interface"].as_str().unwrap_or("asgi");

    let constants = Arc::new(Python::attach(types::Constants::new));

    match interface {
        "asgi" => asgi::filter::Config::new(app, constants)
            .map(|cfg| Box::new(cfg) as Box<dyn HttpFilterConfig<EHF>>),
        "wsgi" => wsgi::filter::Config::new(app, constants)
            .map(|cfg| Box::new(cfg) as Box<dyn HttpFilterConfig<EHF>>),
        _ => {
            eprintln!("pyvoy: unsupported interface: {}", interface);
            None
        }
    }
}
