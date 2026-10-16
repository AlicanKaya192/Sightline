//! The pickles here are written byte by byte (some copied from `pickle.dumps` output);
//! none of them is ever loaded.

mod common;

use common::{fixture_path, pickle, write_bytes};
use sightline_core::{
    FileWarning, Limits, PickleImport, PickleReader, PickleSummary, ReadError, Reader, Summary,
};

fn inspect(name: &str, bytes: &[u8]) -> PickleSummary {
    pickle(summarize(name, bytes).expect("summary"))
}

fn summarize(name: &str, bytes: &[u8]) -> Result<Summary, ReadError> {
    PickleReader.summarize(&write_bytes(name, bytes), &Limits::default())
}

fn import(module: &str, name: &str, dangerous: bool) -> PickleImport {
    PickleImport {
        module: module.to_string(),
        name: name.to_string(),
        dangerous,
    }
}

#[test]
fn accepts_pkl_and_pickle_extensions() {
    assert!(PickleReader.can_read("model.pkl".as_ref()));
    assert!(PickleReader.can_read("data.PICKLE".as_ref()));
    assert!(!PickleReader.can_read("weights.pt".as_ref()));
}

#[test]
fn a_malicious_pickle_is_never_run_and_its_call_is_flagged() {
    let target = fixture_path("pickle-was-run.txt");
    assert!(
        !target.exists(),
        "left over from an earlier run: {target:?}"
    );
    let payload = format!("cbuiltins\nopen\n(V{}\nVw\ntR.", target.display());

    let summary = inspect("malicious.pkl", payload.as_bytes());

    assert!(!target.exists(), "the pickle was run");
    assert_eq!(summary.imports, vec![import("builtins", "open", true)]);
    assert_eq!(summary.calls, 1);
    assert!(summary.warnings.contains(&FileWarning::DangerousImport {
        module: "builtins".to_string(),
        name: "open".to_string(),
    }));
}

#[test]
fn every_pickle_warns_that_loading_it_can_run_code() {
    let summary = inspect("plain.pkl", b"(dp0\nVa\np1\n(lp2\nI1\naF2.5\naVx\np3\nas.");

    assert!(summary.warnings.contains(&FileWarning::RunsCodeWhenLoaded));
    assert!(summary.imports.is_empty());
    assert_eq!(summary.calls, 0);
}

#[test]
fn protocol_0_has_no_protocol_number() {
    let summary = inspect(
        "protocol0.pkl",
        b"(dp0\nVa\np1\n(lp2\nI1\naF2.5\naVx\np3\nas.",
    );

    assert_eq!(summary.protocol, None);
    assert_eq!(summary.opcodes, 17);
}

#[test]
fn global_imports_are_listed_with_the_protocol() {
    // pickle.dumps(collections.OrderedDict(a=1), protocol=2)
    let bytes = b"\x80\x02ccollections\nOrderedDict\nq\x00)Rq\x01X\x01\x00\x00\x00aq\x02K\x01s.";

    let summary = inspect("ordered-dict.pkl", bytes);

    assert_eq!(summary.protocol, Some(2));
    assert_eq!(
        summary.imports,
        vec![import("collections", "OrderedDict", false)]
    );
    assert_eq!(summary.calls, 1);
    assert!(
        !summary
            .warnings
            .iter()
            .any(|w| matches!(w, FileWarning::DangerousImport { .. }))
    );
}

#[test]
fn stack_global_takes_its_names_from_the_strings_before_it() {
    // pickle.dumps(datetime.date(2020, 1, 2), protocol=4)
    let bytes = b"\x80\x04\x95\x20\x00\x00\x00\x00\x00\x00\x00\x8c\x08datetime\x94\x8c\x04date\x94\x93\x94C\x04\x07\xe4\x01\x02\x94\x85\x94R\x94.";

    let summary = inspect("date.pkl", bytes);

    assert_eq!(summary.protocol, Some(4));
    assert_eq!(summary.imports, vec![import("datetime", "date", false)]);
}

#[test]
fn stack_global_follows_names_kept_in_the_memo() {
    // "os" is memoized, dropped, and fetched again before STACK_GLOBAL.
    let bytes = b"\x80\x04\x8c\x02os\x940h\x00\x8c\x06system\x93\x8c\x02ls\x85R.";

    let summary = inspect("memo.pkl", bytes);

    assert_eq!(summary.imports, vec![import("os", "system", true)]);
}

#[test]
fn a_stack_global_whose_names_are_not_text_is_dangerous() {
    let bytes = b"\x80\x04K\x01K\x02\x93).";

    let summary = inspect("unknown-global.pkl", bytes);

    assert_eq!(summary.imports, vec![import("?", "?", true)]);
}

#[test]
fn inst_and_obj_style_imports_are_found() {
    let bytes = b"(Vcalc\nisubprocess\nPopen\n.";

    let summary = inspect("inst.pkl", bytes);

    assert_eq!(summary.imports, vec![import("subprocess", "Popen", true)]);
}

#[test]
fn each_import_is_listed_once() {
    let bytes = b"\x80\x02ccollections\nOrderedDict\n)Rccollections\nOrderedDict\n)R\x86.";

    let summary = inspect("twice.pkl", bytes);

    assert_eq!(
        summary.imports,
        vec![import("collections", "OrderedDict", false)]
    );
    assert_eq!(summary.calls, 2);
}

#[test]
fn large_byte_strings_are_skipped_not_kept() {
    let mut bytes = b"\x80\x05\x8e".to_vec();
    bytes.extend_from_slice(&(1_000_000u64).to_le_bytes());
    bytes.resize(bytes.len() + 1_000_000, b'x');
    bytes.extend_from_slice(b"\x94.");

    let summary = inspect("bytes.pkl", &bytes);

    assert_eq!(summary.protocol, Some(5));
    assert_eq!(summary.opcodes, 4);
    assert!(!summary.warnings.contains(&FileWarning::StoppedEarly));
}

#[test]
fn a_second_pickle_after_stop_is_inspected_too() {
    let bytes = b"\x80\x02N.\x80\x02cos\nsystem\n.";

    let summary = inspect("concatenated.pkl", bytes);

    assert_eq!(summary.imports, vec![import("os", "system", true)]);
}

#[test]
fn a_cut_pickle_reports_what_came_before_the_cut() {
    let bytes = b"\x80\x02cos\nsystem\n(X\xff\x00\x00\x00ab";

    let summary = inspect("cut.pkl", bytes);

    assert_eq!(summary.imports, vec![import("os", "system", true)]);
    assert!(summary.warnings.contains(&FileWarning::StoppedEarly));
}

#[test]
fn an_unknown_opcode_stops_the_inspection() {
    let bytes = b"\x80\x02cos\nsystem\n\xff.";

    let summary = inspect("unknown-opcode.pkl", bytes);

    assert_eq!(summary.imports.len(), 1);
    assert!(summary.warnings.contains(&FileWarning::StoppedEarly));
}

#[test]
fn a_file_that_does_not_start_with_an_opcode_is_rejected() {
    let result = summarize("not-a-pickle.pkl", b"\xffhello");

    assert!(
        matches!(result, Err(ReadError::Malformed { .. })),
        "{result:?}"
    );
}

#[test]
fn an_empty_file_is_rejected() {
    let result = summarize("empty.pkl", b"");

    assert!(
        matches!(result, Err(ReadError::Malformed { .. })),
        "{result:?}"
    );
}
