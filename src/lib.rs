use envoy_proxy_dynamic_modules_rust_sdk::*;
use pyo3::prelude::*;
use pyo3::types::PyList;
use yaml_rust2::YamlLoader;

mod asgi;
mod types;
mod wsgi;

declare_init_functions!(init, new_http_filter_config_fn);

fn init() -> bool {
    Python::initialize();
    // First attachment: Setup and get a GIL-independent reference to the app
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

    let app = match filter_config["app"].as_str() {
        Some(app) => app,
        None => {
            eprintln!("pyvoy: config missing required 'app' field");
            return None;
        }
    };
    let interface = match filter_config["interface"].as_str() {
        Some(interface) => interface,
        None => "asgi",
    };

    match interface {
        "asgi" => match asgi::filter::Config::new(app) {
            Some(cfg) => Some(Box::new(cfg)),
            None => None,
        },
        "wsgi" => match wsgi::filter::Config::new(app) {
            Some(cfg) => Some(Box::new(cfg)),
            None => None,
        },
        _ => {
            eprintln!("pyvoy: unsupported interface: {}", interface);
            None
        }
    }
}
