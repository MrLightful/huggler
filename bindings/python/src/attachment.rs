use crate::sync;
use huggler::Attachment as File;
use pyo3::{
    prelude::*,
    types::{PyBytes, PyDict},
};

#[pyclass(module = "huggler", frozen)]
pub(crate) struct Attachment(pub(crate) File);

#[pymethods]
impl Attachment {
    #[classattr]
    const MAX_FILES: usize = File::MAX_FILES;
    #[classattr]
    const MAX_FILE_BYTES: usize = File::MAX_FILE_BYTES;
    #[classattr]
    const MAX_TOTAL_BYTES: usize = File::MAX_TOTAL_BYTES;
    #[classattr]
    const MAX_IMAGE_BYTES: usize = File::MAX_IMAGE_BYTES;

    #[staticmethod]
    fn from_bytes(py: Python<'_>, name: String, bytes: &[u8]) -> PyResult<Self> {
        sync(py, || File::from_bytes(name, bytes.to_vec())).map(Self)
    }

    #[staticmethod]
    fn validate_batch(py: Python<'_>, files: Vec<Py<Self>>) -> PyResult<()> {
        let files = files
            .iter()
            .map(|file| file.get().0.clone())
            .collect::<Vec<_>>();
        sync(py, || File::validate_batch(&files))
    }

    #[getter]
    fn name(&self) -> &str {
        &self.0.name
    }

    #[getter]
    fn mime(&self) -> &str {
        &self.0.mime
    }

    #[getter]
    fn bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.0.bytes)
    }

    fn text(&self) -> Option<&str> {
        self.0.text()
    }

    fn pdf_text_excerpt(&self, py: Python<'_>) -> Option<String> {
        py.detach(|| self.0.pdf_text_excerpt())
    }

    fn as_record<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let file = PyDict::new(py);
        file.set_item("name", self.name())?;
        file.set_item("mime", self.mime())?;
        file.set_item("bytes", pyo3::types::PyList::new(py, &self.0.bytes)?)?;
        Ok(file)
    }
}
