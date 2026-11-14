use std::fs;
use std::path::Path;
use tantalus_assets::pc::unity_raw::UnityRawBundle;

fn u32be(output: &mut Vec<u8>, value: u32) {
    output.extend_from_slice(&value.to_be_bytes());
}

fn fixture(path: &Path) {
    let mut directory = Vec::new();
    u32be(&mut directory, 1);
    directory.extend_from_slice(b"CAB-test\0");
    let data_offset = (directory.len() + 8) as u32;
    u32be(&mut directory, data_offset);
    u32be(&mut directory, 4);

    let header_size = 60_u32;
    let complete_size = header_size + directory.len() as u32 + 4;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"UnityRaw\0");
    u32be(&mut bytes, 3);
    bytes.extend_from_slice(b"5.x.x\0");
    bytes.extend_from_slice(b"5.2.3p2\0");
    u32be(&mut bytes, complete_size);
    u32be(&mut bytes, header_size);
    u32be(&mut bytes, 1);
    u32be(&mut bytes, 1);
    u32be(&mut bytes, directory.len() as u32);
    u32be(&mut bytes, directory.len() as u32);
    u32be(&mut bytes, complete_size);
    u32be(&mut bytes, 52);
    bytes.push(0);
    bytes.extend_from_slice(&directory);
    bytes.extend_from_slice(b"test");
    fs::write(path, bytes).unwrap();
}

#[test]
fn unity_raw_directory_and_bounded_entry_read_work() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("bundle-日本語.bin");
    fixture(&path);
    let bundle = UnityRawBundle::open(&path).unwrap();
    let report = bundle.report();
    assert_eq!(report.format_version, 3);
    assert_eq!(report.player_version, "5.x.x");
    assert_eq!(report.engine_version, "5.2.3p2");
    assert_eq!(report.header_size, 60);
    assert_eq!(report.entries.len(), 1);
    assert_eq!(report.entries[0].path, "CAB-test");
    assert_eq!(report.entries[0].absolute_offset, 81);
    assert_eq!(bundle.read_entry(0, 4).unwrap(), b"test");
    assert!(bundle.read_entry(0, 3).is_err());
    assert!(bundle.read_entry(1, 4).is_err());
}

#[test]
fn malformed_headers_and_ranges_are_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("bundle.bin");
    fs::write(&path, b"UnityFS\0").unwrap();
    assert!(UnityRawBundle::open(&path).is_err());

    fixture(&path);
    let mut bytes = fs::read(&path).unwrap();
    bytes[0..8].copy_from_slice(b"UnityFS\0");
    fs::write(&path, &bytes).unwrap();
    assert!(UnityRawBundle::open(&path).is_err());

    fixture(&path);
    let mut bytes = fs::read(&path).unwrap();
    bytes[76] = 99;
    fs::write(&path, &bytes).unwrap();
    assert!(UnityRawBundle::open(&path).is_err());
}
