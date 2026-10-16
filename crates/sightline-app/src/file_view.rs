//! Values shared by the model, array and pickle views: shapes, element types, size checks
//! and file warnings. As in `view`, no user text here; the window words every message.

use sightline_core::{DType, FileWarning, SizeCheck};

use crate::view::NumberFormat;
use crate::{SizeState, WarningKind, WarningRow};

/// Sizes joined with a multiplication sign, e.g. `2 × 3`; `()` for a single value.
pub fn shape_text(shape: &[u64], format: &NumberFormat) -> String {
    if shape.is_empty() {
        return "()".to_string();
    }
    shape
        .iter()
        .map(|&size| format.count(size))
        .collect::<Vec<_>>()
        .join(" × ")
}

/// The short type name safetensors uses, such as `F32` or `BF16`.
pub fn dtype_name(dtype: &DType) -> String {
    let name = match dtype {
        DType::Bool => "BOOL",
        DType::U8 => "U8",
        DType::I8 => "I8",
        DType::U16 => "U16",
        DType::I16 => "I16",
        DType::U32 => "U32",
        DType::I32 => "I32",
        DType::U64 => "U64",
        DType::I64 => "I64",
        DType::F16 => "F16",
        DType::BF16 => "BF16",
        DType::F32 => "F32",
        DType::F64 => "F64",
        DType::F8E4M3 => "F8_E4M3",
        DType::F8E5M2 => "F8_E5M2",
        DType::Complex64 => "C64",
        DType::Complex128 => "C128",
        DType::Text => "STRING",
        DType::Other(name) => name,
    };
    name.to_string()
}

pub fn size_state(check: &SizeCheck) -> SizeState {
    match check {
        SizeCheck::Consistent => SizeState::Consistent,
        SizeCheck::Mismatch { .. } => SizeState::Mismatch,
        SizeCheck::BadOffsets { .. } => SizeState::BadOffsets,
        SizeCheck::Unknown => SizeState::Unknown,
    }
}

/// A warning line when the file and its header disagree on its size. Sizes are exact
/// bytes, since a difference of a few bytes disappears in rounded units.
pub fn size_check_warning(check: &SizeCheck, format: &NumberFormat) -> Option<WarningRow> {
    match check {
        SizeCheck::Mismatch { expected, actual } => Some(WarningRow {
            kind: WarningKind::SizeMismatch,
            first: format!("{} B", format.count(*expected)).into(),
            second: format!("{} B", format.count(*actual)).into(),
            ..WarningRow::default()
        }),
        SizeCheck::BadOffsets { tensor } => Some(WarningRow {
            kind: WarningKind::BadOffsets,
            column: tensor.as_str().into(),
            ..WarningRow::default()
        }),
        SizeCheck::Consistent | SizeCheck::Unknown => None,
    }
}

pub fn file_warning_row(warning: &FileWarning, format: &NumberFormat) -> WarningRow {
    let kind = |kind| WarningRow {
        kind,
        ..WarningRow::default()
    };
    match warning {
        FileWarning::ExternalData { tensors } => WarningRow {
            kind: WarningKind::ExternalData,
            first: format.count(*tensors).into(),
            ..WarningRow::default()
        },
        FileWarning::DangerousImport { module, name } => WarningRow {
            kind: WarningKind::DangerousImport,
            column: format!("{module}.{name}").into(),
            ..WarningRow::default()
        },
        FileWarning::PickleInside => kind(WarningKind::PickleInside),
        FileWarning::NoStatistics => kind(WarningKind::NoStatistics),
        FileWarning::RunsCodeWhenLoaded => kind(WarningKind::RunsCode),
        FileWarning::StoppedEarly => kind(WarningKind::StoppedEarly),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ENGLISH: NumberFormat = NumberFormat::ENGLISH;

    #[test]
    fn shapes_are_joined_with_a_multiplication_sign() {
        assert_eq!(shape_text(&[2, 3, 1024], &ENGLISH), "2 × 3 × 1,024");
    }

    #[test]
    fn a_scalar_shape_is_empty_parentheses() {
        assert_eq!(shape_text(&[], &ENGLISH), "()");
    }

    #[test]
    fn dtype_names_follow_safetensors() {
        assert_eq!(dtype_name(&DType::BF16), "BF16");
        assert_eq!(dtype_name(&DType::F8E4M3), "F8_E4M3");
        assert_eq!(dtype_name(&DType::Other("F4".to_string())), "F4");
    }

    #[test]
    fn size_states_follow_the_check() {
        assert_eq!(size_state(&SizeCheck::Consistent), SizeState::Consistent);
        assert_eq!(
            size_state(&SizeCheck::Mismatch {
                expected: 1,
                actual: 2
            }),
            SizeState::Mismatch
        );
        assert_eq!(
            size_state(&SizeCheck::BadOffsets {
                tensor: "w".to_string()
            }),
            SizeState::BadOffsets
        );
    }

    #[test]
    fn a_size_mismatch_warns_with_both_sizes_in_bytes() {
        let check = SizeCheck::Mismatch {
            expected: 2_048,
            actual: 2_050,
        };

        assert_eq!(
            size_check_warning(&check, &ENGLISH),
            Some(WarningRow {
                kind: WarningKind::SizeMismatch,
                first: "2,048 B".into(),
                second: "2,050 B".into(),
                ..WarningRow::default()
            })
        );
    }

    #[test]
    fn bad_offsets_warn_with_the_tensor_name() {
        let check = SizeCheck::BadOffsets {
            tensor: "lm_head.weight".to_string(),
        };

        let row = size_check_warning(&check, &ENGLISH).expect("warning");

        assert_eq!(row.kind, WarningKind::BadOffsets);
        assert_eq!(row.column, "lm_head.weight");
    }

    #[test]
    fn a_consistent_or_unknown_size_needs_no_warning() {
        assert_eq!(size_check_warning(&SizeCheck::Consistent, &ENGLISH), None);
        assert_eq!(size_check_warning(&SizeCheck::Unknown, &ENGLISH), None);
    }

    #[test]
    fn a_dangerous_import_names_the_function() {
        let warning = FileWarning::DangerousImport {
            module: "os".to_string(),
            name: "system".to_string(),
        };

        let row = file_warning_row(&warning, &ENGLISH);

        assert_eq!(row.kind, WarningKind::DangerousImport);
        assert_eq!(row.column, "os.system");
    }

    #[test]
    fn external_data_counts_the_tensors() {
        let row = file_warning_row(&FileWarning::ExternalData { tensors: 1_200 }, &ENGLISH);

        assert_eq!(row.kind, WarningKind::ExternalData);
        assert_eq!(row.first, "1,200");
    }

    #[test]
    fn other_file_warnings_map_to_their_kind() {
        let kinds: Vec<WarningKind> = [
            FileWarning::PickleInside,
            FileWarning::NoStatistics,
            FileWarning::RunsCodeWhenLoaded,
            FileWarning::StoppedEarly,
        ]
        .iter()
        .map(|warning| file_warning_row(warning, &ENGLISH).kind)
        .collect();

        assert_eq!(
            kinds,
            [
                WarningKind::PickleInside,
                WarningKind::NoStatistics,
                WarningKind::RunsCode,
                WarningKind::StoppedEarly,
            ]
        );
    }
}
