//! Turns a pickle inspection into the values the pickle view shows.

use sightline_core::PickleSummary;

use crate::file_view::file_warning_row;
use crate::view::{NumberFormat, size_text, to_int};
use crate::{ImportRow, PickleInfo, WarningRow};

pub fn pickle_info(
    name: &str,
    size_bytes: u64,
    summary: &PickleSummary,
    format: &NumberFormat,
) -> PickleInfo {
    PickleInfo {
        name: name.into(),
        size: size_text(size_bytes, format).into(),
        protocol: summary
            .protocol
            .map_or_else(String::new, |protocol| protocol.to_string())
            .into(),
        protocol_known: summary.protocol.is_some(),
        opcodes: format.count(summary.opcodes).into(),
        imports: to_int(summary.imports.len()),
        calls: format.count(summary.calls).into(),
        dangerous: to_int(
            summary
                .imports
                .iter()
                .filter(|import| import.dangerous)
                .count(),
        ),
    }
}

pub fn import_rows(summary: &PickleSummary) -> Vec<ImportRow> {
    summary
        .imports
        .iter()
        .map(|import| ImportRow {
            module: import.module.as_str().into(),
            name: import.name.as_str().into(),
            dangerous: import.dangerous,
        })
        .collect()
}

pub fn pickle_warnings(summary: &PickleSummary, format: &NumberFormat) -> Vec<WarningRow> {
    summary
        .warnings
        .iter()
        .map(|warning| file_warning_row(warning, format))
        .collect()
}

#[cfg(test)]
mod tests {
    use sightline_core::{FileWarning, PickleImport};

    use super::*;
    use crate::WarningKind;

    const ENGLISH: NumberFormat = NumberFormat::ENGLISH;

    fn summary(protocol: Option<u8>) -> PickleSummary {
        PickleSummary {
            protocol,
            opcodes: 12_345,
            imports: vec![
                PickleImport {
                    module: "collections".to_string(),
                    name: "OrderedDict".to_string(),
                    dangerous: false,
                },
                PickleImport {
                    module: "os".to_string(),
                    name: "system".to_string(),
                    dangerous: true,
                },
            ],
            calls: 3,
            warnings: vec![
                FileWarning::RunsCodeWhenLoaded,
                FileWarning::DangerousImport {
                    module: "os".to_string(),
                    name: "system".to_string(),
                },
            ],
        }
    }

    #[test]
    fn info_counts_imports_calls_and_dangerous_ones() {
        let info = pickle_info("model.pkl", 100, &summary(Some(4)), &ENGLISH);

        assert_eq!(info.protocol, "4");
        assert!(info.protocol_known);
        assert_eq!(info.opcodes, "12,345");
        assert_eq!(info.imports, 2);
        assert_eq!(info.calls, "3");
        assert_eq!(info.dangerous, 1);
    }

    #[test]
    fn protocol_0_and_1_have_no_number() {
        assert!(!pickle_info("a.pkl", 1, &summary(None), &ENGLISH).protocol_known);
    }

    #[test]
    fn import_rows_keep_the_danger_mark() {
        let rows = import_rows(&summary(Some(2)));

        assert_eq!(rows.len(), 2);
        assert!(!rows[0].dangerous);
        assert!(rows[1].dangerous);
        assert_eq!(
            (rows[1].module.as_str(), rows[1].name.as_str()),
            ("os", "system")
        );
    }

    #[test]
    fn warnings_follow_the_inspection() {
        let kinds: Vec<WarningKind> = pickle_warnings(&summary(Some(2)), &ENGLISH)
            .iter()
            .map(|row| row.kind)
            .collect();

        assert_eq!(kinds, [WarningKind::RunsCode, WarningKind::DangerousImport]);
    }
}
