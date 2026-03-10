use std::sync::Arc;

use crate::{asgi::shared::eventloop::EventLoops, types::Constants};
use pyo3::{IntoPyObjectExt, prelude::*, types::PyDict};

use pyo3::PyResult;

pub(crate) fn load_app(
    app_module: &str,
    app_attr: &str,
    constants: &Arc<Constants>,
    worker_threads: usize,
    enable_lifespan: Option<bool>,
) -> PyResult<(Py<PyAny>, Py<PyDict>, EventLoops)> {
    // Import threading on this thread because Python records the first thread
    // that imports threading as the main thread. When running the Python interpreter, this
    // happens to work, but not when embedding. For our purposes, we just need the asyncio
    // thread to know it's not the main thread and can use this hack. In practice, since
    // this is still during filter initialization, it may be the correct main thread anyway.
    Python::attach(|py| {
        py.import("threading")?;
        Ok::<_, PyErr>(())
    })?;

    Python::attach(|py| {
        let module = py.import(app_module)?;
        let app = module.getattr(app_attr)?;

        let app = ensure_asgi3_app(py, app)?;

        let asgi = PyDict::new(py);
        asgi.set_item("version", "3.0")?;
        asgi.set_item("spec_version", "2.5")?;

        let loops = EventLoops::new(py, worker_threads, &app, &asgi, enable_lifespan, constants)?;

        Ok::<_, PyErr>((app.unbind(), asgi.unbind(), loops))
    })
}

/// Converts an ASGI2 app to ASGI3 if needed.
///
/// Reimplements https://github.com/django/asgiref/blob/main/asgiref/compatibility.py#L40
fn ensure_asgi3_app<'py>(py: Python<'py>, app: Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    if is_asgi2_app(py, &app)? {
        let asgi2_invoker = ASGI2Invoker { app: app.unbind() };
        Ok(asgi2_invoker.into_bound_py_any(py)?)
    } else {
        Ok(app)
    }
}

/// Tests to see if an application is ASGI2.
fn is_asgi2_app<'py>(py: Python<'py>, app: &Bound<'py, PyAny>) -> PyResult<bool> {
    // Look for a hint on the object first. This is usually only needed for frameworks using
    // metaclass magic or a rare case where a normal function returns a coroutine, making it
    // effectively a coroutinefunction.
    if let Some(v) = app.getattr_opt("_asgi_single_callable")?
        && v.is_truthy()?
    {
        return Ok(false);
    }
    if let Some(v) = app.getattr_opt("_asgi_double_callable")?
        && v.is_truthy()?
    {
        return Ok(true);
    }

    let inspect = py.import("inspect")?;
    // Classes are always double-callable as they cannot be a coroutine.
    if inspect.call_method1("isclass", (&app,))?.is_truthy()? {
        return Ok(true);
    }

    // inspect.iscoroutinefunction Python 3.12+
    let iscoroutinefunction = inspect
        .getattr_opt("iscoroutinefunction")?
        .unwrap_or_else(|| {
            py.import("asyncio")
                .and_then(|asyncio| asyncio.getattr("iscoroutinefunction"))
                .unwrap()
        });

    // Handle callable objects.
    if let Some(callable) = app.getattr_opt("__call__")?
        && iscoroutinefunction.call1((callable,))?.is_truthy()?
    {
        return Ok(false);
    }

    Ok(!iscoroutinefunction.call1((app,))?.is_truthy()?)
}

/// An ASGI3 application that invokes an ASGI2 application.
#[pyclass(module = "_pyvoy.asgi")]
struct ASGI2Invoker {
    app: Py<PyAny>,
}

#[pymethods]
impl ASGI2Invoker {
    fn __call__<'py>(
        &self,
        py: Python<'py>,
        scope: Bound<'py, PyDict>,
        receive: Bound<'py, PyAny>,
        send: Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let app = self.app.bind(py).call1((scope,))?;
        let coro = app.call1((receive, send))?;
        Ok(coro)
    }
}
