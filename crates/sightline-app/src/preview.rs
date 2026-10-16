//! Opens files for the window: reads on a background thread so the window keeps responding.

use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;

use sightline_core::notebook::notebook;
use sightline_core::{
    ArraySummary, Limits, ModelSummary, PickleSummary, ReadError, Summary, TableSummary, summarize,
};
use slint::{ComponentHandle, ModelRc, VecModel};

use crate::view::NumberFormat;
use crate::{
    AppWindow, NotebookState, ViewState, array_view, model_view, notebook_file, pickle_view, view,
};

struct Loaded {
    size_bytes: u64,
    summary: Summary,
}

/// The file the window shows, kept for "Open in notebook".
struct Shown {
    path: PathBuf,
    summary: Summary,
}

#[derive(Clone)]
pub struct Previewer {
    window: slint::Weak<AppWindow>,
    /// Number of the newest request; results of older ones are dropped.
    latest: Arc<AtomicU64>,
    shown: Arc<Mutex<Option<Shown>>>,
}

impl Previewer {
    pub fn new(window: &AppWindow) -> Self {
        Self {
            window: window.as_weak(),
            latest: Arc::new(AtomicU64::new(0)),
            shown: Arc::new(Mutex::new(None)),
        }
    }

    pub fn open(&self, path: PathBuf) {
        let request = self.latest.fetch_add(1, Ordering::SeqCst) + 1;
        let name = file_name(&path);
        if let Some(window) = self.window.upgrade() {
            window.set_loading_name(name.as_str().into());
            window.set_notebook_state(NotebookState::Idle);
            window.set_state(ViewState::Loading);
        }
        self.set_shown(None);
        let previewer = self.clone();
        thread::spawn(move || {
            let result = load(&path);
            let window = previewer.window.clone();
            // Fails only when the window is already closed; then nobody needs the result.
            let _ = window.upgrade_in_event_loop(move |window| {
                if previewer.latest.load(Ordering::SeqCst) != request {
                    return;
                }
                if let Ok(loaded) = &result {
                    previewer.set_shown(Some(Shown {
                        path: path.clone(),
                        summary: loaded.summary.clone(),
                    }));
                }
                show(&window, &name, result);
            });
        });
    }

    /// Writes a notebook for the shown file and opens it; the window says where it went.
    pub fn open_notebook(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let saved = {
            let shown = self.shown.lock().unwrap_or_else(PoisonError::into_inner);
            let Some(shown) = shown.as_ref() else {
                return;
            };
            let text = notebook(&shown.path, &shown.summary);
            notebook_file::notebook_folder()
                .and_then(|folder| notebook_file::save_new(&folder, &shown.path, &text))
        };
        match saved.and_then(|path| notebook_file::open(&path).map(|()| path)) {
            Ok(path) => {
                window.set_notebook_detail(path.display().to_string().into());
                window.set_notebook_state(NotebookState::Saved);
            }
            Err(error) => {
                window.set_notebook_detail(error.to_string().into());
                window.set_notebook_state(NotebookState::Failed);
            }
        }
    }

    fn set_shown(&self, shown: Option<Shown>) {
        *self.shown.lock().unwrap_or_else(PoisonError::into_inner) = shown;
    }
}

fn load(path: &Path) -> Result<Loaded, ReadError> {
    let size_bytes = fs::metadata(path)?.len();
    let summary = summarize(path, &Limits::default())?;
    Ok(Loaded {
        size_bytes,
        summary,
    })
}

fn show(window: &AppWindow, name: &str, result: Result<Loaded, ReadError>) {
    let format = NumberFormat::from_separators(
        &window.get_decimal_separator(),
        &window.get_thousands_separator(),
    );
    match result {
        Ok(Loaded {
            size_bytes,
            summary,
        }) => match summary {
            Summary::Table(table) => show_table(window, name, size_bytes, &table, &format),
            Summary::Model(model) => show_model(window, name, size_bytes, &model, &format),
            Summary::Array(array) => show_array(window, name, size_bytes, &array, &format),
            Summary::Pickle(pickle) => show_pickle(window, name, size_bytes, &pickle, &format),
        },
        Err(error) => {
            window.set_failure(view::failure_info(name, &error));
            window.set_state(ViewState::Failed);
        }
    }
}

fn show_table(
    window: &AppWindow,
    name: &str,
    size_bytes: u64,
    table: &TableSummary,
    format: &NumberFormat,
) {
    let titles: Vec<slint::SharedString> = table
        .columns
        .iter()
        .map(|column| column.name.as_str().into())
        .collect();
    let rows: Vec<ModelRc<crate::Cell>> = view::head_rows(&table.head)
        .into_iter()
        .map(model)
        .collect();
    window.set_info(view::file_info(name, size_bytes, table, format));
    window.set_schema(model(view::schema_rows(&table.columns, format)));
    let histograms: Vec<ModelRc<f32>> = view::histograms(&table.columns)
        .into_iter()
        .map(model)
        .collect();
    window.set_head_titles(model(titles));
    window.set_head_rows(model(rows));
    window.set_histograms(model(histograms));
    window.set_warnings(model(view::warning_rows(&table.insights, format)));
    window.set_state(ViewState::Loaded);
}

fn show_model(
    window: &AppWindow,
    name: &str,
    size_bytes: u64,
    summary: &ModelSummary,
    format: &NumberFormat,
) {
    let tensors = &summary.tensors;
    window.set_model_info(model_view::model_info(name, size_bytes, summary, format));
    window.set_dtype_rows(model(model_view::dtype_rows(tensors, format)));
    window.set_metadata_rows(model(model_view::metadata_rows(summary)));
    window.set_io_rows(model(model_view::io_rows(summary, format)));
    window.set_operator_rows(model(model_view::operator_rows(summary, format)));
    window.set_layer_rows(model(model_view::layer_rows(tensors, format)));
    window.set_tensor_rows(model(model_view::tensor_rows(tensors, format)));
    window.set_warnings(model(model_view::model_warnings(summary, format)));
    window.set_state(ViewState::Model);
}

fn show_array(
    window: &AppWindow,
    name: &str,
    size_bytes: u64,
    summary: &ArraySummary,
    format: &NumberFormat,
) {
    window.set_array_info(array_view::array_info(name, size_bytes, summary, format));
    window.set_array_histogram(model(array_view::array_histogram(summary)));
    window.set_warnings(model(array_view::array_warnings(summary, format)));
    window.set_state(ViewState::Array);
}

fn show_pickle(
    window: &AppWindow,
    name: &str,
    size_bytes: u64,
    summary: &PickleSummary,
    format: &NumberFormat,
) {
    window.set_pickle_info(pickle_view::pickle_info(name, size_bytes, summary, format));
    window.set_import_rows(model(pickle_view::import_rows(summary)));
    window.set_warnings(model(pickle_view::pickle_warnings(summary, format)));
    window.set_state(ViewState::Pickle);
}

fn model<T: Clone + 'static>(items: Vec<T>) -> ModelRc<T> {
    ModelRc::from(Rc::new(VecModel::from(items)))
}

fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}
