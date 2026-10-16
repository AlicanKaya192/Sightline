mod common;

use common::{table, write_bytes};
use sightline_core::{
    CsvReader, EXTENSIONS, Limits, NpyReader, OnnxReader, ParquetReader, PickleReader, ReadError,
    Reader, SafetensorsReader, TableFormat, summarize,
};

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

#[test]
fn every_listed_extension_has_a_reader() {
    let readers: [&dyn Reader; 6] = [
        &ParquetReader,
        &CsvReader,
        &SafetensorsReader,
        &OnnxReader,
        &NpyReader,
        &PickleReader,
    ];
    for extension in EXTENSIONS {
        let path = std::path::PathBuf::from(format!("file.{extension}"));
        assert!(
            readers.iter().any(|reader| reader.can_read(&path)),
            "{extension}"
        );
    }
}
