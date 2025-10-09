use executors::crossbeam_channel_pool::ThreadPool;
use pyo3::prelude::*;

use super::types::*;
use crate::types::*;
use envoy_proxy_dynamic_modules_rust_sdk::EnvoyHttpFilterScheduler;
use flume;
use flume::Sender;

#[derive(Clone)]
pub(crate) struct Executor {
    pool: ThreadPool,
    app_module: String,
    app_attr: String,
}

impl Executor {
    pub(crate) fn new(app_module: &str, app_attr: &str, num_threads: usize) -> PyResult<Self> {
        let pool = ThreadPool::new(num_threads);

        Ok(Self {
            pool,
            app_module: app_module.to_string(),
            app_attr: app_attr.to_string(),
        })
    }

    pub(crate) fn execute_app(
        &self,
        scope: Scope,
        trailers_accepted: bool,
        request_future_tx: Sender<Py<PyAny>>,
        response_tx: Sender<ResponseEvent>,
        recv_scheduler: Box<dyn EnvoyHttpFilterScheduler>,
        send_scheduler: Box<dyn EnvoyHttpFilterScheduler>,
        end_scheduler: Box<dyn EnvoyHttpFilterScheduler>,
    ) {
    }
}

#[pyclass]
struct StartResponseCallable {
    headers: Option<Vec<(String, Box<[u8]>)>>,
}

#[pymethods]
impl StartResponseCallable {
    fn __call__(&self) -> PyResult<Py<PyAny>> {}
}
