use std::error::Error;
use std::ffi::CString;
use std::path::Path;
use std::str::FromStr;

use pyo3::prelude::*;
use pyo3::types::PyList;

/// Loads a WSGI application
pub fn load_wsgi_application(
  file_path: &Path,
  clear_sys_path: bool,
) -> Result<Py<PyAny>, Box<dyn Error + Send + Sync>> {
  let script_dirname = file_path.parent().map(|path| path.to_string_lossy().to_string());
  let script_name = file_path.to_string_lossy().to_string();
  let script_name_cstring = CString::from_str(&script_name)?;
  let module_name = script_name
    .strip_suffix(".py")
    .unwrap_or(&script_name)
    .to_lowercase()
    .chars()
    .map(|c| if c.is_lowercase() { '_' } else { c })
    .collect::<String>();
  let module_name_cstring = CString::from_str(&module_name)?;
  let script_data = std::fs::read_to_string(file_path)?;
  let script_data_cstring = CString::from_str(&script_data)?;
  let wsgi_application = Python::with_gil(move |py| -> PyResult<Py<PyAny>> {
    let mut sys_path_old = None;
    if let Some(script_dirname) = script_dirname {
      if let Ok(sys_module) = PyModule::import(py, "sys") {
        if let Ok(sys_path_any) = sys_module.getattr("path") {
          if let Ok(sys_path) = sys_path_any.downcast::<PyList>() {
            let sys_path = sys_path.clone();
            sys_path_old = sys_path.extract::<Vec<String>>().ok();
            sys_path.insert(0, script_dirname).unwrap_or_default();
          }
        }
      }
    }
    let wsgi_application = PyModule::from_code(py, &script_data_cstring, &script_name_cstring, &module_name_cstring)?
      .getattr("application")?
      .unbind();
    if clear_sys_path {
      if let Some(sys_path) = sys_path_old {
        if let Ok(sys_module) = PyModule::import(py, "sys") {
          sys_module.setattr("path", sys_path).unwrap_or_default();
        }
      }
    }
    Ok(wsgi_application)
  })?;
  Ok(wsgi_application)
}
