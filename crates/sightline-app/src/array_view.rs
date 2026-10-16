//! Turns an `.npy` array summary into the values the array view shows.

use sightline_core::{ArraySummary, Basis};

use crate::file_view::{file_warning_row, shape_text, size_check_warning};
use crate::view::{NumberFormat, relative_heights, size_text};
use crate::{ArrayInfo, WarningRow};

pub fn array_info(
    name: &str,
    size_bytes: u64,
    summary: &ArraySummary,
    format: &NumberFormat,
) -> ArrayInfo {
    let elements = summary
        .shape
        .iter()
        .try_fold(1u64, |count, &size| count.checked_mul(size));
    let mut info = ArrayInfo {
        name: name.into(),
        size: size_text(size_bytes, format).into(),
        descr: summary.descr.as_str().into(),
        shape: shape_text(&summary.shape, format).into(),
        elements: elements
            .map_or_else(|| "—".to_string(), |count| format.count(count))
            .into(),
        fortran_order: summary.fortran_order,
        ..ArrayInfo::default()
    };
    if let Some(stats) = &summary.stats {
        let optional =
            |value: Option<f64>| value.map_or_else(|| "—".to_string(), |v| format.number(v));
        info.has_stats = true;
        info.values = format.count(stats.values).into();
        info.min = optional(stats.min).into();
        info.max = optional(stats.max).into();
        info.nan = format.count(stats.nan).into();
        info.infinite = format.count(stats.infinite).into();
        info.sampled = stats.basis == Basis::Sample;
    }
    info
}

/// Bar heights from 0 to 1; empty without statistics.
pub fn array_histogram(summary: &ArraySummary) -> Vec<f32> {
    summary
        .stats
        .as_ref()
        .map_or_else(Vec::new, |stats| relative_heights(&stats.histogram))
}

pub fn array_warnings(summary: &ArraySummary, format: &NumberFormat) -> Vec<WarningRow> {
    size_check_warning(&summary.size_check, format)
        .into_iter()
        .chain(
            summary
                .warnings
                .iter()
                .map(|warning| file_warning_row(warning, format)),
        )
        .collect()
}

#[cfg(test)]
mod tests {
    use sightline_core::{ArrayStats, DType, FileWarning, SizeCheck};

    use super::*;
    use crate::WarningKind;

    const ENGLISH: NumberFormat = NumberFormat::ENGLISH;
    const TURKISH: NumberFormat = NumberFormat {
        decimal: ',',
        thousands: '.',
    };

    fn summary(stats: Option<ArrayStats>) -> ArraySummary {
        ArraySummary {
            descr: "<f4".to_string(),
            dtype: DType::F32,
            shape: vec![1000, 3],
            fortran_order: false,
            size_check: SizeCheck::Consistent,
            stats,
            warnings: Vec::new(),
        }
    }

    fn stats(basis: Basis) -> ArrayStats {
        ArrayStats {
            values: 3000,
            basis,
            min: Some(-1.5),
            max: Some(2500.25),
            nan: 2,
            infinite: 0,
            histogram: vec![1, 4, 2],
        }
    }

    #[test]
    fn info_has_shape_type_and_value_count() {
        let info = array_info("a.npy", 12_128, &summary(None), &ENGLISH);

        assert_eq!(info.shape, "1,000 × 3");
        assert_eq!(info.descr, "<f4");
        assert_eq!(info.elements, "3,000");
        assert!(!info.has_stats);
    }

    #[test]
    fn statistics_are_written_in_the_language_style() {
        let info = array_info(
            "a.npy",
            1,
            &summary(Some(stats(Basis::WholeFile))),
            &TURKISH,
        );

        assert!(info.has_stats);
        assert_eq!(info.min, "-1,5");
        assert_eq!(info.max, "2.500,25");
        assert_eq!(info.nan, "2");
        assert!(!info.sampled);
    }

    #[test]
    fn a_sample_is_marked() {
        let info = array_info("a.npy", 1, &summary(Some(stats(Basis::Sample))), &ENGLISH);

        assert!(info.sampled);
        assert_eq!(info.values, "3,000");
    }

    #[test]
    fn tiny_and_huge_numbers_use_scientific_notation() {
        assert_eq!(ENGLISH.number(0.00001234), "1.234e-5");
        assert_eq!(ENGLISH.number(-3e20), "-3e20");
        assert_eq!(ENGLISH.number(0.0), "0");
        assert_eq!(ENGLISH.number(-42.0), "-42");
    }

    #[test]
    fn the_histogram_is_relative_to_the_tallest_bin() {
        assert_eq!(
            array_histogram(&summary(Some(stats(Basis::WholeFile)))),
            vec![0.25, 1.0, 0.5]
        );
        assert!(array_histogram(&summary(None)).is_empty());
    }

    #[test]
    fn warnings_include_the_size_check_and_file_warnings() {
        let mut summary = summary(None);
        summary.size_check = SizeCheck::Mismatch {
            expected: 10,
            actual: 5,
        };
        summary.warnings = vec![FileWarning::NoStatistics];

        let kinds: Vec<WarningKind> = array_warnings(&summary, &ENGLISH)
            .iter()
            .map(|row| row.kind)
            .collect();

        assert_eq!(
            kinds,
            [WarningKind::SizeMismatch, WarningKind::NoStatistics]
        );
    }
}
