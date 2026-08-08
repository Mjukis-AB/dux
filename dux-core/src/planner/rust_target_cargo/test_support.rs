use super::*;

#[cfg(test)]
impl RustTargetCargoMetadataWitness {
    pub(in crate::planner) fn cargo_release(&self) -> (u32, u32, u32) {
        (
            self.cargo.release.major,
            self.cargo.release.minor,
            self.cargo.release.patch,
        )
    }

    pub(in crate::planner) fn cargo_version_sha256(&self) -> [u8; 32] {
        self.cargo.version_sha256
    }

    pub(in crate::planner) fn cargo_executable_sha256(&self) -> [u8; 32] {
        self.cargo.file.sha256()
    }

    pub(in crate::planner) fn cargo_executable_identity(
        &self,
    ) -> crate::path_validation::FilesystemIdentity {
        self.cargo.file.path().target_identity()
    }

    pub(in crate::planner) fn cargo_environment_identities(
        &self,
    ) -> [crate::path_validation::FilesystemIdentity; 3] {
        [
            self.cargo.environment.home.identity(),
            self.cargo.environment.cargo_home.identity(),
            self.cargo.environment.temporary_directory.identity(),
        ]
    }

    pub(in crate::planner) fn cargo_executable_path(&self) -> &Path {
        &self.cargo.executable
    }

    pub(in crate::planner) fn cargo_executable_parent_identity(
        &self,
    ) -> crate::path_validation::FilesystemIdentity {
        self.cargo.parent.identity()
    }

    pub(in crate::planner) fn live(&self) -> &RustTargetLiveWitness {
        &self.live
    }

    pub(in crate::planner) fn metadata_sha256(&self) -> [u8; 32] {
        self.metadata_sha256
    }

    pub(in crate::planner) fn configuration_policy_revision(&self) -> u32 {
        self.configuration.policy_revision
    }

    pub(in crate::planner) fn configuration_lookup_count(&self) -> u32 {
        self.configuration.lookup_count
    }

    pub(in crate::planner) fn configuration_root_count(&self) -> u32 {
        self.configuration.root_config_count
    }

    pub(in crate::planner) fn configuration_file_count(&self) -> u32 {
        self.configuration.config_file_count
    }

    pub(in crate::planner) fn configuration_include_edge_count(&self) -> u32 {
        self.configuration.include_edge_count
    }

    pub(in crate::planner) fn configuration_byte_count(&self) -> u64 {
        self.configuration.config_byte_count
    }

    pub(in crate::planner) fn configuration_closure_sha256(&self) -> [u8; 32] {
        self.configuration.closure_sha256
    }

    pub(in crate::planner) fn configuration_read_intent_sha256(&self) -> [u8; 32] {
        self.configuration.read_intent_sha256
    }

    pub(in crate::planner) fn workspace_manifest_policy_revision(&self) -> u32 {
        self.workspace.policy_revision
    }

    pub(in crate::planner) fn workspace_glob_policy_revision(&self) -> u32 {
        self.workspace_glob.policy_revision
    }

    pub(in crate::planner) fn workspace_glob_pattern_count(&self) -> u32 {
        self.workspace_glob.pattern_count
    }

    pub(in crate::planner) fn workspace_glob_directory_count(&self) -> u32 {
        self.workspace_glob.observed_directory_count
    }

    pub(in crate::planner) fn workspace_glob_entry_count(&self) -> u32 {
        self.workspace_glob.namespace_entry_count
    }

    pub(in crate::planner) fn workspace_glob_raw_match_count(&self) -> u32 {
        self.workspace_glob.raw_match_count
    }

    pub(in crate::planner) fn workspace_glob_directory_match_count(&self) -> u32 {
        self.workspace_glob.directory_match_count
    }

    pub(in crate::planner) fn workspace_glob_closure_sha256(&self) -> [u8; 32] {
        self.workspace_glob.closure_sha256
    }

    pub(in crate::planner) fn workspace_glob_root_manifest_sha256(&self) -> [u8; 32] {
        self.workspace_glob.root_manifest_sha256
    }

    pub(in crate::planner) fn workspace_membership_consistency_policy_revision(&self) -> u32 {
        self.workspace_membership_consistency.policy_revision
    }

    pub(in crate::planner) fn workspace_seed_package_count(&self) -> u32 {
        self.workspace_membership_consistency.seed_package_count
    }

    pub(in crate::planner) fn workspace_excluded_member_count(&self) -> u32 {
        self.workspace_membership_consistency.excluded_member_count
    }

    pub(in crate::planner) fn workspace_reachable_package_count(&self) -> u32 {
        self.workspace_membership_consistency
            .reachable_package_count
    }

    pub(in crate::planner) fn workspace_default_member_count(&self) -> u32 {
        self.workspace_membership_consistency.default_member_count
    }

    pub(in crate::planner) fn workspace_membership_consistency_closure_sha256(&self) -> [u8; 32] {
        self.workspace_membership_consistency.closure_sha256
    }

    pub(in crate::planner) fn manifest_probe_policy_revision(&self) -> u32 {
        self.manifest_probes.policy_revision
    }

    pub(in crate::planner) fn manifest_probe_count(&self) -> u32 {
        self.manifest_probes.probe_count
    }

    pub(in crate::planner) fn present_ancestor_manifest_count(&self) -> u32 {
        self.manifest_probes.manifest_count
    }

    pub(in crate::planner) fn absent_ancestor_manifest_count(&self) -> u32 {
        self.manifest_probes.absent_probe_count
    }

    pub(in crate::planner) fn ancestor_manifest_byte_count(&self) -> u64 {
        self.manifest_probes.manifest_bytes
    }

    pub(in crate::planner) fn manifest_probe_closure_sha256(&self) -> [u8; 32] {
        self.manifest_probes.closure_sha256
    }

    pub(in crate::planner) fn workspace_member_count(&self) -> u32 {
        self.workspace.workspace_member_count
    }

    pub(in crate::planner) fn workspace_manifest_count(&self) -> u32 {
        self.workspace.manifest_count
    }

    pub(in crate::planner) fn workspace_manifest_closure_sha256(&self) -> [u8; 32] {
        self.workspace.closure_sha256
    }

    pub(in crate::planner) fn path_dependency_policy_revision(&self) -> u32 {
        self.path_dependencies.policy_revision
    }

    pub(in crate::planner) fn dependency_declaration_count(&self) -> u32 {
        self.path_dependencies.dependency_declaration_count
    }

    pub(in crate::planner) fn local_path_dependency_count(&self) -> u32 {
        self.path_dependencies.local_path_dependency_count
    }

    pub(in crate::planner) fn unique_local_dependency_manifest_count(&self) -> u32 {
        self.path_dependencies.unique_local_manifest_count
    }

    pub(in crate::planner) fn path_dependency_closure_sha256(&self) -> [u8; 32] {
        self.path_dependencies.closure_sha256
    }

    pub(in crate::planner) fn dependency_manifest_policy_revision(&self) -> u32 {
        self.dependency_manifests.policy_revision
    }

    pub(in crate::planner) fn independently_declared_local_dependency_count(&self) -> u32 {
        self.dependency_manifests.local_dependency_count
    }

    pub(in crate::planner) fn independent_dependency_manifest_count(&self) -> u32 {
        self.dependency_manifests.unique_local_manifest_count
    }

    pub(in crate::planner) fn dependency_manifest_closure_sha256(&self) -> [u8; 32] {
        self.dependency_manifests.closure_sha256
    }

    pub(in crate::planner) fn package_metadata_policy_revision(&self) -> u32 {
        self.package_metadata.policy_revision
    }

    pub(in crate::planner) fn package_metadata_package_count(&self) -> u32 {
        self.package_metadata.package_count
    }

    pub(in crate::planner) fn implicit_readme_probe_count(&self) -> u32 {
        self.package_metadata.implicit_readme_probe_count
    }

    pub(in crate::planner) fn implicit_readme_selection_count(&self) -> u32 {
        self.package_metadata.implicit_readme_selection_count
    }

    pub(in crate::planner) fn declared_readme_count(&self) -> u32 {
        self.package_metadata.declared_readme_count
    }

    pub(in crate::planner) fn license_file_count(&self) -> u32 {
        self.package_metadata.license_file_count
    }

    pub(in crate::planner) fn package_metadata_closure_sha256(&self) -> [u8; 32] {
        self.package_metadata.closure_sha256
    }

    pub(in crate::planner) fn target_namespace_policy_revision(&self) -> u32 {
        self.target_namespace.policy_revision
    }

    pub(in crate::planner) fn target_namespace_package_count(&self) -> u32 {
        self.target_namespace.package_count
    }

    pub(in crate::planner) fn target_namespace_target_count(&self) -> u32 {
        self.target_namespace.target_count
    }

    pub(in crate::planner) fn target_namespace_count(&self) -> u32 {
        self.target_namespace.namespace_count
    }

    pub(in crate::planner) fn target_namespace_closure_sha256(&self) -> [u8; 32] {
        self.target_namespace.closure_sha256
    }

    pub(in crate::planner) fn launch_policy_revision(&self) -> u32 {
        self.launch_policy_revision
    }

    pub(in crate::planner) fn running_code_directory_hash_sha256(&self) -> [u8; 32] {
        self.running_code_directory_hash_sha256
    }

    pub(in crate::planner) fn resolution_policy_revision(&self) -> u32 {
        self.resolution_policy_revision
    }

    pub(in crate::planner) fn enrollment_revision(&self) -> u64 {
        self.enrollment_revision
    }
}

#[cfg(test)]
impl CargoExecutableObservation {
    pub(in crate::planner) fn executable_sha256(&self) -> [u8; 32] {
        self.file.sha256()
    }

    pub(in crate::planner) fn executable_identity(
        &self,
    ) -> crate::path_validation::FilesystemIdentity {
        self.file.path().target_identity()
    }

    pub(in crate::planner) fn environment_identities(
        &self,
    ) -> [crate::path_validation::FilesystemIdentity; 3] {
        [
            self.environment.home.identity(),
            self.environment.cargo_home.identity(),
            self.environment.temporary_directory.identity(),
        ]
    }
}

#[cfg(test)]
pub(in crate::planner) fn validate_cargo_metadata_for_test(
    live: RustTargetLiveWitness,
    cargo: &CargoExecutableObservation,
    timeout: Duration,
    stdout_bytes: usize,
    stderr_bytes: usize,
) -> Result<RustTargetCargoMetadataWitness, CargoMetadataValidationError> {
    let limits = ProcessLimits {
        timeout,
        stdout_bytes,
        stderr_bytes,
    };
    validate_cargo_metadata_with_limits(
        live,
        cargo,
        RunnerLimits {
            version: ProcessLimits::version(),
            metadata: limits,
        },
        None,
        ConfigurationFenceMode::UnarmedForParallelTest,
        || {},
    )
}

#[cfg(all(test, target_os = "macos"))]
pub(in crate::planner) fn signed_cargo_output_limit_for_test(
    cargo: &CargoExecutableObservation,
    current_directory: &Path,
) -> Result<(), CargoMetadataValidationError> {
    let directory = RetainedCargoDirectory::capture(current_directory)?;
    let environment = capture_resolution_environment()?;
    run_cargo(
        cargo.launch_target(),
        &directory,
        &[OsStr::new("--version"), OsStr::new("--verbose")],
        ProcessLimits {
            timeout: Duration::from_secs(5),
            stdout_bytes: 1,
            stderr_bytes: VERSION_STDERR_LIMIT,
        },
        &environment,
        CargoInputGuards::default(),
    )
    .map(|_| ())
}

#[cfg(test)]
pub(in crate::planner) fn validate_cargo_metadata_with_input_fences_for_test(
    live: RustTargetLiveWitness,
    cargo: &CargoExecutableObservation,
) -> Result<RustTargetCargoMetadataWitness, CargoMetadataValidationError> {
    validate_cargo_metadata_with_limits(
        live,
        cargo,
        RunnerLimits::production(),
        None,
        ConfigurationFenceMode::Armed,
        || {},
    )
}

#[cfg(test)]
pub(in crate::planner) fn revalidate_retained_cargo_directory_after_hook_for_test(
    path: &Path,
    hook: impl FnOnce(),
) -> Result<(), CargoMetadataValidationError> {
    let directory = RetainedCargoDirectory::capture(path)?;
    hook();
    directory.revalidate()
}

#[cfg(test)]
pub(in crate::planner) fn validate_cargo_metadata_with_enrollment_hook_for_test(
    live: RustTargetLiveWitness,
    cargo: &CargoExecutableObservation,
    store: Arc<StoreCoordinator>,
    enrollment: CargoEnrollmentSetting,
    after_metadata: impl FnOnce(),
) -> Result<RustTargetCargoMetadataWitness, CargoMetadataValidationError> {
    validate_cargo_metadata_with_limits(
        live,
        cargo,
        RunnerLimits::production(),
        Some(EnrollmentGuard { store, enrollment }),
        ConfigurationFenceMode::UnarmedForParallelTest,
        after_metadata,
    )
}
