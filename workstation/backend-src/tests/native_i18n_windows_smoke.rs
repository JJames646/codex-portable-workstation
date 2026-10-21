#![cfg(windows)]

use std::fs;
use std::path::PathBuf;

use yunshu_workstation_runtime::native_i18n;
use yunshu_workstation_runtime::paths::WorkstationPaths;

#[test]
#[ignore = "requires an explicit isolated Windows fixture"]
fn applies_and_reads_native_i18n_with_bundled_classic_level() {
    let root = std::env::var_os("YUNSHU_NATIVE_I18N_SMOKE_ROOT")
        .map(PathBuf::from)
        .expect("YUNSHU_NATIVE_I18N_SMOKE_ROOT is required");
    let payload_path = std::env::var_os("YUNSHU_NATIVE_I18N_SMOKE_PAYLOAD")
        .map(PathBuf::from)
        .expect("YUNSHU_NATIVE_I18N_SMOKE_PAYLOAD is required");
    assert!(root.to_string_lossy().contains(".native-i18n-smoke"));

    let paths = WorkstationPaths::from_root(root.clone());
    let database = root.join("data/Codex/Default/Local Storage/leveldb");
    fs::create_dir_all(&database).unwrap();
    let sentinel = root.join("outside-leveldb-sentinel.txt");
    fs::write(&sentinel, b"keep").unwrap();

    let payload = native_i18n::load_payload(&payload_path).unwrap();
    native_i18n::apply(&paths, &database, &payload).unwrap();
    assert!(native_i18n::verify(&paths, &database, &payload).unwrap());
    assert_eq!(fs::read(&sentinel).unwrap(), b"keep");
}
