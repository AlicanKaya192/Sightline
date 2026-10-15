mod common;

use common::{table, write_bytes};
use sightline_core::{Limits, ReadError, TableFormat, summarize};

#[test]
fn picks_the_reader_by_file_name() {
    let path = write_bytes("dispatch.csv", b"a;b\n1;2\n");

    let summary = table(summarize(&path, &Limits::default()).expect("summary"));

    assert!(matches!(summary.format, TableFormat::Csv { .. }));
}

#[test]
fn rejects_an_unknown_file_type() {
    let path = write_bytes("notes.txt", b"hello");

    let result = summarize(&path, &Limits::default());

    assert!(
        matches!(result, Err(ReadError::UnsupportedFormat(ref p)) if *p == path),
        "{result:?}"
    );
}
