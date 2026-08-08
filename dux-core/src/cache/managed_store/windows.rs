use std::fs::File;
use std::path::Path;
use std::time::Instant;

use super::{ManagedCacheStoreAccess, ManagedCacheStoreError, ManagedCacheStoreErrorKind, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct Identity;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct ChangeToken;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    ContainerDirectory,
    PrivateDirectory,
    PrivateFile,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Publication {
    Published,
    Collision,
}

fn unsupported<T>() -> Result<T> {
    Err(ManagedCacheStoreError::new(
        ManagedCacheStoreErrorKind::UnsupportedPlatform,
    ))
}

pub(super) fn open_container_parent(_path: &Path) -> Result<(File, Identity)> {
    unsupported()
}

pub(super) fn detach_directory_no_replace(
    _parent: &File,
    _source: &str,
    _source_directory: &File,
    _source_identity: Identity,
    _destination: &str,
) -> Result<()> {
    unsupported()
}

pub(super) fn open_or_create_child_container(
    _parent: &File,
    _path: &Path,
    _access: ManagedCacheStoreAccess,
) -> Result<Option<(File, Identity)>> {
    unsupported()
}

pub(super) fn open_existing_container(_parent: &File, _name: &str) -> Result<Option<File>> {
    unsupported()
}

pub(super) fn validate_path(
    _path: &Path,
    _retained: &File,
    _expected: Identity,
    _kind: Kind,
) -> Result<()> {
    unsupported()
}

pub(super) fn open_existing_private_directory(
    _parent: &File,
    _parent_path: &Path,
    _name: &str,
) -> Result<Option<File>> {
    unsupported()
}

pub(super) fn create_private_directory_exclusive(
    _parent: &File,
    _parent_path: &Path,
    _name: &str,
) -> Result<Option<File>> {
    unsupported()
}

pub(super) fn create_private_file_exclusive(
    _directory: &File,
    _directory_path: &Path,
    _name: &str,
) -> Result<Option<(File, Identity)>> {
    unsupported()
}

pub(super) fn open_named_private_file(
    _directory: &File,
    _directory_path: &Path,
    _name: &str,
    _writable: bool,
) -> Result<Option<(File, Identity)>> {
    unsupported()
}

pub(super) fn identity(_file: &File, _kind: Kind) -> Result<Identity> {
    unsupported()
}

pub(super) const fn same_filesystem(_left: Identity, _right: Identity) -> bool {
    false
}

pub(super) fn change_token(_file: &File) -> Result<ChangeToken> {
    unsupported()
}

pub(super) fn file_usage(_file: &File) -> Result<(u64, u64)> {
    unsupported()
}

pub(super) fn validate_retained(
    _file: &File,
    _expected: Identity,
    _kind: Kind,
    _require_one_link: bool,
) -> Result<()> {
    unsupported()
}

pub(super) fn validate_named(
    _directory: &File,
    _name: &str,
    _retained: &File,
    _expected: Identity,
    _kind: Kind,
) -> Result<()> {
    unsupported()
}

pub(super) fn inventory(
    _directory: &File,
    _maximum_entries: usize,
    _maximum_name_bytes: usize,
    _deadline: Instant,
) -> Result<Vec<String>> {
    unsupported()
}

pub(super) fn exact_name_exists(
    _directory: &File,
    _expected_name: &str,
    _deadline: Instant,
) -> Result<bool> {
    unsupported()
}

pub(super) fn sync_directory(_directory: &File) -> Result<()> {
    unsupported()
}

pub(super) fn publish_directory_no_replace(
    _parent: &File,
    _source: &str,
    _source_directory: &File,
    _source_identity: Identity,
    _destination: &str,
) -> Result<Publication> {
    unsupported()
}

pub(super) fn publish_file_replace(
    _directory: &File,
    _source: &str,
    _source_file: &File,
    _source_identity: Identity,
    _destination: &str,
) -> Result<Publication> {
    unsupported()
}

pub(super) fn remove_retained_file(
    _directory: &File,
    _name: &str,
    _file: File,
    _expected: Identity,
) -> Result<()> {
    unsupported()
}

pub(super) fn remove_app_data_reset_stage_control(
    _directory: &File,
    _name: &str,
    _file: File,
    _expected: Identity,
) -> Result<()> {
    unsupported()
}

pub(super) fn remove_retained_directory(
    _parent: &File,
    _name: &str,
    _directory: File,
    _expected: Identity,
) -> Result<()> {
    unsupported()
}

pub(super) fn remove_app_data_reset_retired_stage_directory(
    _parent: &File,
    _name: &str,
    _directory: File,
    _expected: Identity,
) -> Result<()> {
    unsupported()
}
