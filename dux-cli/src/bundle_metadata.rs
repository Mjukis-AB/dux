use std::io::{self, Write as _};
use std::process::ExitCode;

use dux_core::{DATABASE_SCHEMA_VERSION, SNAPSHOT_FORMAT_VERSION};
use serde::Serialize;

const RECORD_VERSION: u32 = 1;
const PRODUCT: &str = "dux-cli";
const MAX_VERSION_BYTES: usize = 128;
const MAX_DOCUMENT_BYTES: usize = 1_024;

#[derive(Debug, Serialize)]
struct BundleMetadataDocument {
    record_version: u32,
    product: &'static str,
    version: &'static str,
    database_schema_version: u32,
    snapshot_format_version: u32,
}

pub(crate) fn run() -> ExitCode {
    let document = match encoded_document() {
        Ok(document) => document,
        Err(()) => {
            eprintln!("Error: DUX bundle metadata is invalid.");
            return ExitCode::from(70);
        }
    };
    match write_document(&document) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => ExitCode::SUCCESS,
        Err(_) => {
            eprintln!("Error: DUX could not write bundle metadata.");
            ExitCode::from(70)
        }
    }
}

fn encoded_document() -> Result<String, ()> {
    let version = env!("CARGO_PKG_VERSION");
    if version.is_empty()
        || version.len() > MAX_VERSION_BYTES
        || version
            .bytes()
            .any(|byte| byte.is_ascii_control() || !byte.is_ascii())
    {
        return Err(());
    }
    let document = BundleMetadataDocument {
        record_version: RECORD_VERSION,
        product: PRODUCT,
        version,
        database_schema_version: DATABASE_SCHEMA_VERSION,
        snapshot_format_version: SNAPSHOT_FORMAT_VERSION,
    };
    let encoded = serde_json::to_string(&document).map_err(|_| ())?;
    (encoded.len() <= MAX_DOCUMENT_BYTES)
        .then_some(encoded)
        .ok_or(())
}

fn write_document(document: &str) -> io::Result<()> {
    let stdout = io::stdout();
    let mut lock = stdout.lock();
    lock.write_all(document.as_bytes())?;
    lock.write_all(b"\n")?;
    lock.flush()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use serde_json::Value;

    use super::*;

    #[test]
    fn metadata_json_has_one_exact_bounded_golden_shape() {
        let json = encoded_document().unwrap();

        assert_eq!(
            json,
            format!(
                "{{\"record_version\":1,\"product\":\"dux-cli\",\"version\":\"{}\",\
                 \"database_schema_version\":{},\"snapshot_format_version\":{}}}",
                env!("CARGO_PKG_VERSION"),
                DATABASE_SCHEMA_VERSION,
                SNAPSHOT_FORMAT_VERSION
            )
        );
        assert!(json.len() <= MAX_DOCUMENT_BYTES);
    }

    #[test]
    fn metadata_json_parser_observes_only_the_frozen_path_free_keys() {
        let json = encoded_document().unwrap();
        let value: Value = serde_json::from_str(&json).unwrap();
        let object = value.as_object().unwrap();
        let keys = object.keys().map(String::as_str).collect::<BTreeSet<_>>();

        assert_eq!(
            keys,
            BTreeSet::from([
                "database_schema_version",
                "product",
                "record_version",
                "snapshot_format_version",
                "version",
            ])
        );
        assert_eq!(object["record_version"], 1);
        assert_eq!(object["product"], PRODUCT);
        assert_eq!(object["version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(
            object["database_schema_version"],
            u64::from(DATABASE_SCHEMA_VERSION)
        );
        assert_eq!(
            object["snapshot_format_version"],
            u64::from(SNAPSHOT_FORMAT_VERSION)
        );
    }
}
