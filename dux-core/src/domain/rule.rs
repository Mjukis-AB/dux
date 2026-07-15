use std::net::{Ipv4Addr, Ipv6Addr};
use std::str::FromStr;
use std::time::Duration;

use thiserror::Error;

use super::{CandidateAction, CandidateCategory, LocalizedTextKey, RuleRef, SafetyTier};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProvenanceUrl(String);

impl ProvenanceUrl {
    pub fn new(value: impl Into<String>) -> Result<Self, RuleValidationError> {
        let value = value.into();
        let parsed = split_http_url(&value);
        if value.trim() != value
            || !value.is_ascii()
            || value.chars().any(char::is_control)
            || value.chars().any(char::is_whitespace)
            || parsed.is_none_or(|(authority, remainder)| {
                !is_valid_http_authority(authority) || !is_valid_http_url_remainder(remainder)
            })
        {
            return Err(RuleValidationError::InvalidProvenance);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn split_http_url(value: &str) -> Option<(&str, &str)> {
    let remainder = value
        .strip_prefix("https://")
        .or_else(|| value.strip_prefix("http://"))?;
    let authority_end = remainder.find(['/', '?', '#']).unwrap_or(remainder.len());
    Some(remainder.split_at(authority_end))
}

fn is_valid_http_url_remainder(remainder: &str) -> bool {
    let (path_and_query, fragment) = match remainder.split_once('#') {
        Some((before, fragment)) => (before, Some(fragment)),
        None => (remainder, None),
    };
    let (path, query) = match path_and_query.split_once('?') {
        Some((path, query)) => (path, Some(query)),
        None => (path_and_query, None),
    };

    (path.is_empty() || path.starts_with('/'))
        && is_valid_uri_component(path, true, false)
        && query.is_none_or(|query| is_valid_uri_component(query, true, true))
        && fragment.is_none_or(|fragment| is_valid_uri_component(fragment, true, true))
}

fn is_valid_uri_component(value: &str, allow_slash: bool, allow_question: bool) -> bool {
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte == b'%' {
            if index + 2 >= bytes.len()
                || !bytes[index + 1].is_ascii_hexdigit()
                || !bytes[index + 2].is_ascii_hexdigit()
            {
                return false;
            }
            index += 3;
            continue;
        }
        let allowed = byte.is_ascii_alphanumeric()
            || matches!(
                byte,
                b'-' | b'.'
                    | b'_'
                    | b'~'
                    | b':'
                    | b'@'
                    | b'!'
                    | b'$'
                    | b'&'
                    | b'\''
                    | b'('
                    | b')'
                    | b'*'
                    | b'+'
                    | b','
                    | b';'
                    | b'='
            )
            || (allow_slash && byte == b'/')
            || (allow_question && byte == b'?');
        if !allowed {
            return false;
        }
        index += 1;
    }
    true
}

fn is_valid_http_authority(authority: &str) -> bool {
    if authority.is_empty() || authority.contains(['@', '\\']) {
        return false;
    }

    let (host, port) = if let Some(bracketed) = authority.strip_prefix('[') {
        let Some((host, suffix)) = bracketed.split_once(']') else {
            return false;
        };
        if Ipv6Addr::from_str(host).is_err() {
            return false;
        }
        let port = match suffix.strip_prefix(':') {
            Some(port) => Some(port),
            None if suffix.is_empty() => None,
            None => return false,
        };
        return port.is_none_or(is_valid_port);
    } else if let Some((host, port)) = authority.rsplit_once(':') {
        if host.contains(':') {
            return false;
        }
        (host, Some(port))
    } else {
        (authority, None)
    };

    port.is_none_or(is_valid_port) && is_valid_host(host)
}

fn is_valid_port(port: &str) -> bool {
    port.parse::<u16>().is_ok_and(|port| port != 0)
}

fn is_valid_host(host: &str) -> bool {
    if Ipv4Addr::from_str(host).is_ok() {
        return true;
    }
    if host
        .bytes()
        .all(|byte| byte.is_ascii_digit() || byte == b'.')
    {
        return false;
    }
    host.len() <= 253
        && host.contains('.')
        && host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RuleScope {
    Home,
    UserCacheDirectory,
    ConfiguredProjectRoots,
    SelectedScanRoot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleMatcherDefinition {
    pub path_component: Option<String>,
    pub required_ancestor_markers_any: Vec<String>,
    pub required_markers_all: Vec<String>,
    pub forbidden_markers_any: Vec<String>,
    pub exact_bundle_identifiers: Vec<String>,
    pub excluded_descendants: Vec<String>,
    pub protected_descendants: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleMatcher {
    path_component: Option<String>,
    required_ancestor_markers_any: Vec<String>,
    required_markers_all: Vec<String>,
    forbidden_markers_any: Vec<String>,
    exact_bundle_identifiers: Vec<String>,
    excluded_descendants: Vec<String>,
    protected_descendants: Vec<String>,
}

impl RuleMatcher {
    pub fn try_new(definition: RuleMatcherDefinition) -> Result<Self, RuleValidationError> {
        if definition.path_component.is_none() && definition.exact_bundle_identifiers.is_empty() {
            return Err(RuleValidationError::MissingMatchSelector);
        }
        if definition.path_component.is_some() && !definition.exact_bundle_identifiers.is_empty() {
            return Err(RuleValidationError::AmbiguousMatchSelector);
        }
        if definition
            .path_component
            .as_deref()
            .is_some_and(|value| !is_valid_path_component(value))
            || definition
                .required_ancestor_markers_any
                .iter()
                .chain(definition.required_markers_all.iter())
                .chain(definition.forbidden_markers_any.iter())
                .chain(definition.excluded_descendants.iter())
                .chain(definition.protected_descendants.iter())
                .any(|value| !is_valid_relative_path(value))
            || definition
                .exact_bundle_identifiers
                .iter()
                .any(|value| !is_valid_bundle_identifier(value))
        {
            return Err(RuleValidationError::InvalidMatcherValue);
        }

        Ok(Self {
            path_component: definition.path_component,
            required_ancestor_markers_any: definition.required_ancestor_markers_any,
            required_markers_all: definition.required_markers_all,
            forbidden_markers_any: definition.forbidden_markers_any,
            exact_bundle_identifiers: definition.exact_bundle_identifiers,
            excluded_descendants: definition.excluded_descendants,
            protected_descendants: definition.protected_descendants,
        })
    }

    pub fn path_component(&self) -> Option<&str> {
        self.path_component.as_deref()
    }

    pub fn required_ancestor_markers_any(&self) -> &[String] {
        &self.required_ancestor_markers_any
    }

    pub fn required_markers_all(&self) -> &[String] {
        &self.required_markers_all
    }

    pub fn forbidden_markers_any(&self) -> &[String] {
        &self.forbidden_markers_any
    }

    pub fn exact_bundle_identifiers(&self) -> &[String] {
        &self.exact_bundle_identifiers
    }

    pub fn excluded_descendants(&self) -> &[String] {
        &self.excluded_descendants
    }

    pub fn protected_descendants(&self) -> &[String] {
        &self.protected_descendants
    }
}

fn is_valid_path_component(value: &str) -> bool {
    is_meaningful_value(value)
        && value.len() <= 255
        && value != "."
        && value != ".."
        && !value.ends_with('.')
        && !value.contains(['<', '>', ':', '"', '/', '\\', '|', '?', '*'])
}

fn is_valid_relative_path(value: &str) -> bool {
    is_meaningful_value(value)
        && !value.contains('\\')
        && value.split('/').all(is_valid_path_component)
}

fn is_valid_bundle_identifier(value: &str) -> bool {
    is_meaningful_value(value)
        && value.len() <= 255
        && value.contains('.')
        && value.split('.').all(|component| {
            !component.is_empty()
                && component
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
}

fn is_meaningful_value(value: &str) -> bool {
    !value.is_empty() && value.trim() == value && !value.chars().any(char::is_control)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActivityGuard {
    ProcessName(String),
    BundleIdentifier(String),
}

impl ActivityGuard {
    fn is_valid(&self) -> bool {
        match self {
            Self::ProcessName(value) => is_valid_path_component(value),
            Self::BundleIdentifier(value) => is_valid_bundle_identifier(value),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleGuards {
    minimum_age: Option<Duration>,
    minimum_bytes: u64,
    inactive_processes: Vec<ActivityGuard>,
    requires_cloud_upload_complete: bool,
}

impl RuleGuards {
    pub fn try_new(
        minimum_age: Option<Duration>,
        minimum_bytes: u64,
        inactive_processes: Vec<ActivityGuard>,
        requires_cloud_upload_complete: bool,
    ) -> Result<Self, RuleValidationError> {
        if minimum_age == Some(Duration::ZERO)
            || inactive_processes.iter().any(|guard| !guard.is_valid())
        {
            return Err(RuleValidationError::InvalidGuardValue);
        }
        Ok(Self {
            minimum_age,
            minimum_bytes,
            inactive_processes,
            requires_cloud_upload_complete,
        })
    }

    pub fn minimum_age(&self) -> Option<Duration> {
        self.minimum_age
    }

    pub fn minimum_bytes(&self) -> u64 {
        self.minimum_bytes
    }

    pub fn inactive_processes(&self) -> &[ActivityGuard] {
        &self.inactive_processes
    }

    pub fn requires_cloud_upload_complete(&self) -> bool {
        self.requires_cloud_upload_complete
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleDefinition {
    pub reference: RuleRef,
    pub title_key: LocalizedTextKey,
    pub category: CandidateCategory,
    pub scope: RuleScope,
    pub matcher: RuleMatcher,
    pub guards: RuleGuards,
    pub safety: SafetyTier,
    pub action: CandidateAction,
    pub schedule_eligible: bool,
    pub explanation_key: LocalizedTextKey,
    pub provenance: Vec<ProvenanceUrl>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    reference: RuleRef,
    title_key: LocalizedTextKey,
    category: CandidateCategory,
    scope: RuleScope,
    matcher: RuleMatcher,
    guards: RuleGuards,
    safety: SafetyTier,
    action: CandidateAction,
    schedule_eligible: bool,
    explanation_key: LocalizedTextKey,
    provenance: Vec<ProvenanceUrl>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RuleValidationError {
    #[error("rule safety tier and action are incompatible")]
    IncompatibleSafetyAndAction,
    #[error("scheduled rules must be safe-regenerable permanent-removal rules")]
    InvalidSchedulePolicy,
    #[error("rule provenance must contain at least one source URL")]
    MissingProvenance,
    #[error("rule provenance must be a valid absolute HTTP(S) URL")]
    InvalidProvenance,
    #[error("rule must contain a path-component or exact-bundle match selector")]
    MissingMatchSelector,
    #[error("rule must use exactly one match-selector family")]
    AmbiguousMatchSelector,
    #[error("rule matcher values must use safe platform-neutral component syntax")]
    InvalidMatcherValue,
    #[error("rule guard values must be meaningful, trimmed, and control-free")]
    InvalidGuardValue,
    #[error("safe-evictable rules must require confirmed cloud-upload state")]
    MissingCloudUploadGuard,
}

impl Rule {
    pub fn try_new(definition: RuleDefinition) -> Result<Self, RuleValidationError> {
        if !definition.action.is_compatible_with(definition.safety) {
            return Err(RuleValidationError::IncompatibleSafetyAndAction);
        }
        if definition.schedule_eligible
            && !definition.safety.is_schedule_policy_pair(definition.action)
        {
            return Err(RuleValidationError::InvalidSchedulePolicy);
        }
        if definition.provenance.is_empty() {
            return Err(RuleValidationError::MissingProvenance);
        }
        if definition.safety == SafetyTier::SafeEvictable
            && !definition.guards.requires_cloud_upload_complete()
        {
            return Err(RuleValidationError::MissingCloudUploadGuard);
        }

        Ok(Self {
            reference: definition.reference,
            title_key: definition.title_key,
            category: definition.category,
            scope: definition.scope,
            matcher: definition.matcher,
            guards: definition.guards,
            safety: definition.safety,
            action: definition.action,
            schedule_eligible: definition.schedule_eligible,
            explanation_key: definition.explanation_key,
            provenance: definition.provenance,
        })
    }

    pub fn reference(&self) -> &RuleRef {
        &self.reference
    }

    pub fn title_key(&self) -> &LocalizedTextKey {
        &self.title_key
    }

    pub fn category(&self) -> CandidateCategory {
        self.category
    }

    pub fn scope(&self) -> RuleScope {
        self.scope
    }

    pub fn matcher(&self) -> &RuleMatcher {
        &self.matcher
    }

    pub fn guards(&self) -> &RuleGuards {
        &self.guards
    }

    pub fn safety(&self) -> SafetyTier {
        self.safety
    }

    pub fn action(&self) -> CandidateAction {
        self.action
    }

    pub fn schedule_eligible(&self) -> bool {
        self.schedule_eligible
    }

    pub fn explanation_key(&self) -> &LocalizedTextKey {
        &self.explanation_key
    }

    pub fn provenance(&self) -> &[ProvenanceUrl] {
        &self.provenance
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{RuleId, RuleRevision};

    fn matcher_definition() -> RuleMatcherDefinition {
        RuleMatcherDefinition {
            path_component: Some("target".to_owned()),
            required_ancestor_markers_any: vec!["Cargo.toml".to_owned()],
            required_markers_all: Vec::new(),
            forbidden_markers_any: Vec::new(),
            exact_bundle_identifiers: Vec::new(),
            excluded_descendants: Vec::new(),
            protected_descendants: Vec::new(),
        }
    }

    fn matcher() -> RuleMatcher {
        RuleMatcher::try_new(matcher_definition()).unwrap()
    }

    fn definition(safety: SafetyTier, action: CandidateAction) -> RuleDefinition {
        RuleDefinition {
            reference: RuleRef::new(
                RuleId::new("developer.rust.target").unwrap(),
                RuleRevision::new(1).unwrap(),
            ),
            title_key: LocalizedTextKey::new("rule.developer.rust.target.title").unwrap(),
            category: CandidateCategory::DeveloperArtifact,
            scope: RuleScope::ConfiguredProjectRoots,
            matcher: matcher(),
            guards: RuleGuards::try_new(
                Some(Duration::from_secs(7 * 86_400)),
                100 * 1024 * 1024,
                Vec::new(),
                safety == SafetyTier::SafeEvictable,
            )
            .unwrap(),
            safety,
            action,
            schedule_eligible: false,
            explanation_key: LocalizedTextKey::new("rule.developer.rust.target.explanation")
                .unwrap(),
            provenance: vec![
                ProvenanceUrl::new("https://doc.rust-lang.org/cargo/guide/build-cache.html")
                    .unwrap(),
            ],
        }
    }

    #[test]
    fn rule_policy_cannot_encode_incompatible_cleanup_authority() {
        let error = Rule::try_new(definition(
            SafetyTier::ReviewRequired,
            CandidateAction::RemoveKnownRegenerableContents,
        ))
        .unwrap_err();
        assert_eq!(error, RuleValidationError::IncompatibleSafetyAndAction);

        let error = Rule::try_new(definition(
            SafetyTier::SafeRegenerable,
            CandidateAction::EvictLocalCopy,
        ))
        .unwrap_err();
        assert_eq!(error, RuleValidationError::IncompatibleSafetyAndAction);
    }

    #[test]
    fn scheduled_rules_require_the_only_allowed_policy_pair() {
        let mut allowed = definition(
            SafetyTier::SafeRegenerable,
            CandidateAction::RemoveKnownRegenerableContents,
        );
        allowed.schedule_eligible = true;
        assert!(Rule::try_new(allowed).is_ok());

        let mut rejected = definition(SafetyTier::ReviewRequired, CandidateAction::MoveToTrash);
        rejected.schedule_eligible = true;
        assert_eq!(
            Rule::try_new(rejected).unwrap_err(),
            RuleValidationError::InvalidSchedulePolicy
        );
    }

    #[test]
    fn rules_require_auditable_provenance() {
        let mut missing = definition(SafetyTier::Informational, CandidateAction::RevealOnly);
        missing.provenance.clear();
        assert_eq!(
            Rule::try_new(missing).unwrap_err(),
            RuleValidationError::MissingProvenance
        );
        for invalid in [
            " ",
            "not-a-url",
            " https://example.com",
            "https://",
            "https://.",
            "https://-example.com",
            "https://example-.com",
            "https://example..com",
            "https://example.com:0",
            "https://example.com:not-a-port",
            "https://1.2.3.999",
            "https://example.com\\redirect",
            "https://example.com/path\\redirect",
            "https://example.com/bad%escape",
            "https://example.com/bad%2",
            "https://example.com/<unsafe>",
            "https://example.com/path[bad]",
            "https://example.com/#one#two",
            "https://user@example.com",
            "ftp://example.com",
            "http://not a host",
            "https://example.com/\nunsafe",
        ] {
            assert!(ProvenanceUrl::new(invalid).is_err(), "accepted {invalid:?}");
        }
    }

    #[test]
    fn matcher_and_guard_domains_reject_empty_or_ambiguous_values() {
        assert_eq!(
            RuleMatcher::try_new(RuleMatcherDefinition {
                path_component: None,
                required_ancestor_markers_any: Vec::new(),
                required_markers_all: Vec::new(),
                forbidden_markers_any: Vec::new(),
                exact_bundle_identifiers: Vec::new(),
                excluded_descendants: Vec::new(),
                protected_descendants: Vec::new(),
            })
            .unwrap_err(),
            RuleValidationError::MissingMatchSelector
        );
        assert_eq!(
            RuleGuards::try_new(Some(Duration::ZERO), 0, Vec::new(), false).unwrap_err(),
            RuleValidationError::InvalidGuardValue
        );
        assert_eq!(
            RuleMatcher::try_new(RuleMatcherDefinition {
                path_component: Some(String::new()),
                required_ancestor_markers_any: Vec::new(),
                required_markers_all: Vec::new(),
                forbidden_markers_any: Vec::new(),
                exact_bundle_identifiers: Vec::new(),
                excluded_descendants: Vec::new(),
                protected_descendants: Vec::new(),
            })
            .unwrap_err(),
            RuleValidationError::InvalidMatcherValue
        );
        assert_eq!(
            RuleMatcher::try_new(RuleMatcherDefinition {
                path_component: Some("target".to_owned()),
                required_ancestor_markers_any: Vec::new(),
                required_markers_all: Vec::new(),
                forbidden_markers_any: Vec::new(),
                exact_bundle_identifiers: vec!["com.example.App".to_owned()],
                excluded_descendants: Vec::new(),
                protected_descendants: Vec::new(),
            })
            .unwrap_err(),
            RuleValidationError::AmbiguousMatchSelector
        );
        assert_eq!(
            RuleGuards::try_new(
                None,
                0,
                vec![ActivityGuard::ProcessName(String::new())],
                false,
            )
            .unwrap_err(),
            RuleValidationError::InvalidGuardValue
        );
        assert_eq!(
            RuleGuards::try_new(
                None,
                0,
                vec![ActivityGuard::BundleIdentifier("Safari".to_owned())],
                false,
            )
            .unwrap_err(),
            RuleValidationError::InvalidGuardValue
        );
    }

    #[test]
    fn matcher_paths_use_platform_neutral_relative_grammar() {
        for path_component in [
            ".",
            "..",
            "../target",
            "target/child",
            "target\\child",
            "C:",
            "target*",
            "target.",
        ] {
            let mut definition = matcher_definition();
            definition.path_component = Some(path_component.to_owned());
            assert_eq!(
                RuleMatcher::try_new(definition).unwrap_err(),
                RuleValidationError::InvalidMatcherValue,
                "accepted component {path_component:?}"
            );
        }

        for relative_path in [
            "/Cargo.toml",
            "../Cargo.toml",
            "nested/../Cargo.toml",
            "nested//Cargo.toml",
            "nested\\Cargo.toml",
        ] {
            let mut definition = matcher_definition();
            definition.required_markers_all = vec![relative_path.to_owned()];
            assert_eq!(
                RuleMatcher::try_new(definition).unwrap_err(),
                RuleValidationError::InvalidMatcherValue,
                "accepted relative path {relative_path:?}"
            );
        }
    }

    #[test]
    fn safe_eviction_rules_require_the_upload_guard() {
        let mut unsafe_eviction =
            definition(SafetyTier::SafeEvictable, CandidateAction::EvictLocalCopy);
        unsafe_eviction.guards = RuleGuards::try_new(None, 0, Vec::new(), false).unwrap();
        assert_eq!(
            Rule::try_new(unsafe_eviction).unwrap_err(),
            RuleValidationError::MissingCloudUploadGuard
        );
    }
}
