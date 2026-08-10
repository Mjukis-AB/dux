//! Read-only, path-free catalog for authoring category-scoped automation.

use sha2::{Digest, Sha256};

use super::{
    AutomationScheduleAuthoringBinding, AutomationScheduleDraftConfig, AutomationScheduleScope,
    CandidateAction, CandidateCategory, LocalizedTextKey, Rule, RuleRef, RuleScope, SafetyTier,
};

pub const AUTOMATION_SCHEDULE_AUTHORING_CATALOG_POLICY_REVISION: u32 = 1;
pub const MAX_AUTOMATION_SCHEDULE_AUTHORING_CATEGORIES: usize = 10;
pub const MAX_AUTOMATION_SCHEDULE_AUTHORING_RULES: usize = 256;

const MEMBERSHIP_DOMAIN: &[u8] = b"dux.automation.schedule-authoring.membership.v1\0";
const CATEGORY_ORDER: [CandidateCategory; MAX_AUTOMATION_SCHEDULE_AUTHORING_CATEGORIES] = [
    CandidateCategory::DeveloperArtifact,
    CandidateCategory::ApplicationCache,
    CandidateCategory::BrowserCache,
    CandidateCategory::LogAndDiagnostic,
    CandidateCategory::InstallerAndDownload,
    CandidateCategory::DeviceAndSimulatorData,
    CandidateCategory::CloudFile,
    CandidateCategory::LargeReviewItem,
    CandidateCategory::ProtectedSystemData,
    CandidateCategory::UnknownStorage,
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AutomationScheduleAuthoringRule {
    rule: RuleRef,
    title_key: LocalizedTextKey,
}

impl AutomationScheduleAuthoringRule {
    pub fn rule(&self) -> &RuleRef {
        &self.rule
    }

    pub fn title_key(&self) -> &LocalizedTextKey {
        &self.title_key
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AutomationScheduleAuthoringCategory {
    category: CandidateCategory,
    binding: AutomationScheduleAuthoringBinding,
    rules: Vec<AutomationScheduleAuthoringRule>,
}

impl AutomationScheduleAuthoringCategory {
    pub const fn category(&self) -> CandidateCategory {
        self.category
    }

    pub const fn binding(&self) -> AutomationScheduleAuthoringBinding {
        self.binding
    }

    pub fn rules(&self) -> &[AutomationScheduleAuthoringRule] {
        &self.rules
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AutomationScheduleAuthoringCatalog {
    policy_revision: u32,
    categories: Vec<AutomationScheduleAuthoringCategory>,
}

impl AutomationScheduleAuthoringCatalog {
    pub const fn policy_revision(&self) -> u32 {
        self.policy_revision
    }

    pub fn categories(&self) -> &[AutomationScheduleAuthoringCategory] {
        &self.categories
    }

    pub fn selectable_rule_count(&self) -> usize {
        self.categories
            .iter()
            .map(|category| category.rules.len())
            .sum()
    }

    pub(crate) fn category(
        &self,
        category: CandidateCategory,
    ) -> Option<&AutomationScheduleAuthoringCategory> {
        self.categories
            .iter()
            .find(|choice| choice.category == category)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AutomationScheduleAuthoringCatalogBuildError {
    BoundsExceeded,
    DuplicateRule,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AutomationScheduleAuthoringSelectionError {
    BindingRequired,
    BindingStale,
    InvalidSelection,
}

pub(crate) fn validate_automation_schedule_authoring_selection(
    catalog: &AutomationScheduleAuthoringCatalog,
    config: &AutomationScheduleDraftConfig,
) -> Result<(), AutomationScheduleAuthoringSelectionError> {
    let AutomationScheduleScope::Category(category) = config.scope() else {
        return Ok(());
    };
    let Some(binding) = config.authoring_binding() else {
        return Err(AutomationScheduleAuthoringSelectionError::BindingRequired);
    };
    let Some(choice) = catalog.category(*category) else {
        return Err(AutomationScheduleAuthoringSelectionError::InvalidSelection);
    };
    if binding != choice.binding() {
        return Err(AutomationScheduleAuthoringSelectionError::BindingStale);
    }
    if config
        .excluded_rules()
        .iter()
        .any(|excluded| !choice.rules().iter().any(|rule| rule.rule() == excluded))
        || config.excluded_rules().len() == choice.rules().len()
    {
        return Err(AutomationScheduleAuthoringSelectionError::InvalidSelection);
    }
    Ok(())
}

pub(crate) fn automation_schedule_authoring_rule_is_selectable(rule: &Rule) -> bool {
    rule.schedule_eligible()
        && rule.safety() == SafetyTier::SafeRegenerable
        && rule.action() == CandidateAction::RemoveKnownRegenerableContents
        && rule.scope() == RuleScope::UserCacheDirectory
        && rule.matcher().protected_descendants().is_empty()
}

pub(crate) fn build_automation_schedule_authoring_catalog<'a>(
    rules: impl Iterator<Item = &'a Rule>,
) -> Result<AutomationScheduleAuthoringCatalog, AutomationScheduleAuthoringCatalogBuildError> {
    let mut selected = rules
        .filter(|rule| automation_schedule_authoring_rule_is_selectable(rule))
        .collect::<Vec<_>>();
    if selected.len() > MAX_AUTOMATION_SCHEDULE_AUTHORING_RULES {
        return Err(AutomationScheduleAuthoringCatalogBuildError::BoundsExceeded);
    }
    selected.sort_by(|left, right| left.reference().cmp(right.reference()));
    if selected
        .windows(2)
        .any(|pair| pair[0].reference() == pair[1].reference())
    {
        return Err(AutomationScheduleAuthoringCatalogBuildError::DuplicateRule);
    }

    let mut categories = Vec::new();
    for category in CATEGORY_ORDER {
        let category_rules = selected
            .iter()
            .copied()
            .filter(|rule| rule.category() == category)
            .map(|rule| AutomationScheduleAuthoringRule {
                rule: rule.reference().clone(),
                title_key: rule.title_key().clone(),
            })
            .collect::<Vec<_>>();
        if category_rules.is_empty() {
            continue;
        }
        let binding = automation_schedule_authoring_membership_binding(
            category,
            category_rules.iter().map(|rule| &rule.rule),
        );
        categories.push(AutomationScheduleAuthoringCategory {
            category,
            binding,
            rules: category_rules,
        });
    }
    debug_assert!(categories.len() <= MAX_AUTOMATION_SCHEDULE_AUTHORING_CATEGORIES);
    Ok(AutomationScheduleAuthoringCatalog {
        policy_revision: AUTOMATION_SCHEDULE_AUTHORING_CATALOG_POLICY_REVISION,
        categories,
    })
}

pub(crate) fn automation_schedule_authoring_membership_binding<'a>(
    category: CandidateCategory,
    rules: impl Iterator<Item = &'a RuleRef>,
) -> AutomationScheduleAuthoringBinding {
    let rules = rules.collect::<Vec<_>>();
    let mut hasher = Sha256::new();
    hasher.update(MEMBERSHIP_DOMAIN);
    hasher.update(AUTOMATION_SCHEDULE_AUTHORING_CATALOG_POLICY_REVISION.to_be_bytes());
    hasher.update([category_discriminant(category)]);
    hasher.update(u16::try_from(rules.len()).unwrap_or(u16::MAX).to_be_bytes());
    for rule in rules {
        let id = rule.id().as_str().as_bytes();
        hasher.update(u16::try_from(id.len()).unwrap_or(u16::MAX).to_be_bytes());
        hasher.update(id);
        hasher.update(rule.revision().get().to_be_bytes());
    }
    AutomationScheduleAuthoringBinding::try_new(
        AUTOMATION_SCHEDULE_AUTHORING_CATALOG_POLICY_REVISION,
        hasher.finalize().into(),
    )
    .expect("the compiled authoring policy revision is nonzero")
}

const fn category_discriminant(category: CandidateCategory) -> u8 {
    match category {
        CandidateCategory::DeveloperArtifact => 0,
        CandidateCategory::ApplicationCache => 1,
        CandidateCategory::BrowserCache => 2,
        CandidateCategory::LogAndDiagnostic => 3,
        CandidateCategory::InstallerAndDownload => 4,
        CandidateCategory::DeviceAndSimulatorData => 5,
        CandidateCategory::CloudFile => 6,
        CandidateCategory::LargeReviewItem => 7,
        CandidateCategory::ProtectedSystemData => 8,
        CandidateCategory::UnknownStorage => 9,
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::domain::{
        AutomationConfirmationMode, AutomationScheduleCadence, ProvenanceUrl, RuleDefinition,
        RuleGuards, RuleId, RuleMatcher, RuleMatcherDefinition, RuleRevision,
    };

    fn rule(
        id: &str,
        title: &str,
        category: CandidateCategory,
        scope: RuleScope,
        protected_descendants: Vec<String>,
    ) -> Rule {
        Rule::try_new(RuleDefinition {
            reference: RuleRef::new(RuleId::new(id).unwrap(), RuleRevision::new(1).unwrap()),
            title_key: LocalizedTextKey::new(title).unwrap(),
            category,
            scope,
            matcher: RuleMatcher::try_new(RuleMatcherDefinition {
                path_component: Some("cache".to_owned()),
                required_ancestor_markers_any: Vec::new(),
                required_markers_all: Vec::new(),
                forbidden_markers_any: Vec::new(),
                exact_bundle_identifiers: Vec::new(),
                excluded_descendants: Vec::new(),
                protected_descendants,
            })
            .unwrap(),
            guards: RuleGuards::try_new(Some(Duration::from_secs(1)), 0, Vec::new(), false)
                .unwrap(),
            safety: SafetyTier::SafeRegenerable,
            action: CandidateAction::RemoveKnownRegenerableContents,
            schedule_eligible: true,
            explanation_key: LocalizedTextKey::new("rule.synthetic.explanation").unwrap(),
            provenance: vec![ProvenanceUrl::new("https://example.com/rule").unwrap()],
        })
        .unwrap()
    }

    #[test]
    fn catalog_uses_fixed_category_order_and_canonical_rule_refs() {
        let application = rule(
            "cache.application.zeta",
            "rule.application.zeta.title",
            CandidateCategory::ApplicationCache,
            RuleScope::UserCacheDirectory,
            Vec::new(),
        );
        let developer_zeta = rule(
            "cache.developer.zeta",
            "rule.developer.zeta.title",
            CandidateCategory::DeveloperArtifact,
            RuleScope::UserCacheDirectory,
            Vec::new(),
        );
        let developer_alpha = rule(
            "cache.developer.alpha",
            "rule.developer.alpha.title",
            CandidateCategory::DeveloperArtifact,
            RuleScope::UserCacheDirectory,
            Vec::new(),
        );
        let rules = [application, developer_zeta, developer_alpha];
        let catalog = build_automation_schedule_authoring_catalog(rules.iter()).unwrap();

        assert_eq!(catalog.policy_revision(), 1);
        assert_eq!(catalog.selectable_rule_count(), 3);
        assert_eq!(
            catalog.categories()[0].category(),
            CandidateCategory::DeveloperArtifact
        );
        assert_eq!(
            catalog.categories()[1].category(),
            CandidateCategory::ApplicationCache
        );
        assert_eq!(
            catalog.categories()[0]
                .rules()
                .iter()
                .map(|rule| rule.rule().id().as_str())
                .collect::<Vec<_>>(),
            ["cache.developer.alpha", "cache.developer.zeta"]
        );
    }

    #[test]
    fn membership_binding_ignores_titles_but_changes_with_exact_membership() {
        let original = rule(
            "cache.application.one",
            "rule.application.original.title",
            CandidateCategory::ApplicationCache,
            RuleScope::UserCacheDirectory,
            Vec::new(),
        );
        let renamed = rule(
            "cache.application.one",
            "rule.application.renamed.title",
            CandidateCategory::ApplicationCache,
            RuleScope::UserCacheDirectory,
            Vec::new(),
        );
        let added = rule(
            "cache.application.two",
            "rule.application.two.title",
            CandidateCategory::ApplicationCache,
            RuleScope::UserCacheDirectory,
            Vec::new(),
        );
        let original_catalog =
            build_automation_schedule_authoring_catalog([&original].into_iter()).unwrap();
        let renamed_catalog =
            build_automation_schedule_authoring_catalog([&renamed].into_iter()).unwrap();
        let expanded_catalog =
            build_automation_schedule_authoring_catalog([&original, &added].into_iter()).unwrap();
        assert_eq!(
            original_catalog.categories()[0].binding(),
            renamed_catalog.categories()[0].binding()
        );
        assert_ne!(
            original_catalog.categories()[0].binding(),
            expanded_catalog.categories()[0].binding()
        );
    }

    #[test]
    fn catalog_predicate_rejects_wrong_scope_and_protected_descendants() {
        let wrong_scope = rule(
            "cache.application.scope",
            "rule.application.scope.title",
            CandidateCategory::ApplicationCache,
            RuleScope::Home,
            Vec::new(),
        );
        let protected = rule(
            "cache.application.protected",
            "rule.application.protected.title",
            CandidateCategory::ApplicationCache,
            RuleScope::UserCacheDirectory,
            vec!["preserve".to_owned()],
        );
        let catalog =
            build_automation_schedule_authoring_catalog([&wrong_scope, &protected].into_iter())
                .unwrap();
        assert!(catalog.categories().is_empty());
    }

    #[test]
    fn catalog_fails_closed_above_the_fixed_rule_bound() {
        let rules = (0..=MAX_AUTOMATION_SCHEDULE_AUTHORING_RULES)
            .map(|ordinal| {
                rule(
                    &format!("cache.synthetic.r{ordinal}"),
                    &format!("rule.synthetic.r{ordinal}.title"),
                    CandidateCategory::ApplicationCache,
                    RuleScope::UserCacheDirectory,
                    Vec::new(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            build_automation_schedule_authoring_catalog(rules.iter()),
            Err(AutomationScheduleAuthoringCatalogBuildError::BoundsExceeded)
        );
    }

    #[test]
    fn selection_validation_blocks_membership_drift_foreign_exclusions_and_all_excluded() {
        let one = rule(
            "cache.application.one",
            "rule.application.one.title",
            CandidateCategory::ApplicationCache,
            RuleScope::UserCacheDirectory,
            Vec::new(),
        );
        let two = rule(
            "cache.application.two",
            "rule.application.two.title",
            CandidateCategory::ApplicationCache,
            RuleScope::UserCacheDirectory,
            Vec::new(),
        );
        let reviewed = build_automation_schedule_authoring_catalog([&one].into_iter()).unwrap();
        let current =
            build_automation_schedule_authoring_catalog([&one, &two].into_iter()).unwrap();
        let reviewed_config = AutomationScheduleDraftConfig::try_new_bound(
            AutomationScheduleScope::Category(CandidateCategory::ApplicationCache),
            AutomationScheduleCadence::Monthly,
            Duration::ZERO,
            0,
            1,
            Vec::new(),
            true,
            AutomationConfirmationMode::RequireConfirmation,
            reviewed.categories()[0].binding(),
        )
        .unwrap();
        assert_eq!(
            validate_automation_schedule_authoring_selection(&current, &reviewed_config),
            Err(AutomationScheduleAuthoringSelectionError::BindingStale)
        );

        let current_binding = current.categories()[0].binding();
        let foreign = RuleRef::new(
            RuleId::new("cache.browser.foreign").unwrap(),
            RuleRevision::new(1).unwrap(),
        );
        let foreign_config = AutomationScheduleDraftConfig::try_new_bound(
            AutomationScheduleScope::Category(CandidateCategory::ApplicationCache),
            AutomationScheduleCadence::Monthly,
            Duration::ZERO,
            0,
            1,
            vec![foreign],
            true,
            AutomationConfirmationMode::RequireConfirmation,
            current_binding,
        )
        .unwrap();
        assert_eq!(
            validate_automation_schedule_authoring_selection(&current, &foreign_config),
            Err(AutomationScheduleAuthoringSelectionError::InvalidSelection)
        );

        let all_excluded = AutomationScheduleDraftConfig::try_new_bound(
            AutomationScheduleScope::Category(CandidateCategory::ApplicationCache),
            AutomationScheduleCadence::Monthly,
            Duration::ZERO,
            0,
            1,
            vec![one.reference().clone(), two.reference().clone()],
            true,
            AutomationConfirmationMode::RequireConfirmation,
            current_binding,
        )
        .unwrap();
        assert_eq!(
            validate_automation_schedule_authoring_selection(&current, &all_excluded),
            Err(AutomationScheduleAuthoringSelectionError::InvalidSelection)
        );
    }
}
