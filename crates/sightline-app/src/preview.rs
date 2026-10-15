//! Opens files for the window: reads on a background thread so the window keeps responding.

use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;

use sightline_core::{Limits, ReadError, Summary, TableSummary, summarize};
use slint::{ComponentHandle, ModelRc, VecModel};

use crate::view::NumberFormat;
use crate::{AppWindow, ViewState, view};

struct Loaded {
    size_bytes: u64,
    table: TableSummary,
}

#[derive(Clone)]
pub struct Previewer {
    window: slint::Weak<AppWindow>,
    /// Number of the newest request; results of older ones are dropped.
    latest: Arc<AtomicU64>,
}

impl Previewer {
    pub fn new(window: &AppWindow) -> Self {
        Self {
            window: window.as_weak(),
            latest: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn open(&self, path: PathBuf) {
        let request = self.latest.fetch_add(1, Ordering::SeqCst) + 1;
        let name = file_name(&path);
        if let Some(window) = self.window.upgrade() {
            window.set_loading_name(name.as_str().into());
            window.set_state(ViewState::Loading);
        }
        let window = self.window.clone();
        let latest = Arc::clone(&self.latest);
        thread::spawn(move || {
            let result = load(&path);
            // Fails only when the window is already closed; then nobody needs the result.
            let _ = window.upgrade_in_event_loop(move |window| {
                if latest.load(Ordering::SeqCst) == request {
                    show(&window, &name, result);
                }
            });
        });
    }
}

fn load(path: &Path) -> Result<Loaded, ReadError> {
    let size_bytes = fs::metadata(path)?.len();
    let Summary::Table(table) = summarize(path, &Limits::default())?;
    Ok(Loaded { size_bytes, table })
}

fn show(window: &AppWindow, name: &str, result: Result<Loaded, ReadError>) {
    match result {
        Ok(loaded) => show_table(window, name, &loaded),
        Err(error) => {
            window.set_failure(view::failure_info(name, &error));
            window.set_state(ViewState::Failed);
        }
    }
}

fn show_table(window: &AppWindow, name: &str, loaded: &Loaded) {
    let table = &loaded.table;
    let titles: Vec<slint::SharedString> = table
        .columns
        .iter()
        .map(|column| column.name.as_str().into())
        .collect();
    let rows: Vec<ModelRc<crate::Cell>> = view::head_rows(&table.head)
        .into_iter()
        .map(model)
        .collect();
    let format = NumberFormat::from_separators(
        &window.get_decimal_separator(),
        &window.get_thousands_separator(),
    );
    window.set_info(view::file_info(name, loaded.size_bytes, table, &format));
    window.set_schema(model(view::schema_rows(&table.columns, &format)));
    let histograms: Vec<ModelRc<f32>> = view::histograms(&table.columns)
        .into_iter()
        .map(model)
        .collect();
    window.set_head_titles(model(titles));
    window.set_head_rows(model(rows));
    window.set_histograms(model(histograms));
    window.set_warnings(model(view::warning_rows(&table.insights, &format)));
    window.set_state(ViewState::Loaded);
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
