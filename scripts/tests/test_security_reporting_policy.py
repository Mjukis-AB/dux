import pathlib
import unittest


REPO = pathlib.Path(__file__).resolve().parents[2]


def normalized(path: pathlib.Path) -> str:
    return " ".join(path.read_text(encoding="utf-8").split())


POLICY = normalized(REPO / "SECURITY.md")
SECURITY_DESIGN = normalized(REPO / "SECURITY_DESIGN.md")
ROADMAP = normalized(REPO / "ROADMAP.md")
README = normalized(REPO / "README.md")
CHANGELOG = normalized(REPO / "CHANGELOG.md")
QUALIFICATION = normalized(
    REPO / "docs" / "testing" / "permanent-safe-cleanup-qualification.md"
)


class SecurityReportingPolicyTests(unittest.TestCase):
    def test_policy_is_discoverable_but_does_not_claim_an_active_channel(self) -> None:
        self.assertIn("[SECURITY.md](SECURITY.md)", README)
        self.assertIn("Private vulnerability reporting is not active yet", POLICY)
        self.assertIn("repository setting is not active yet", SECURITY_DESIGN)
        self.assertIn(
            "https://github.com/Mjukis-AB/dux/security/advisories/new",
            POLICY,
        )
        self.assertIn('`"enabled": true`', POLICY)
        self.assertNotRegex(POLICY, r"(?i)mailto:|security@")
        self.assertIn(
            "GitHub Private Vulnerability Reporting is not active yet",
            README,
        )
        self.assertIn("repository setting is still disabled", CHANGELOG)

    def test_policy_defines_support_and_response_expectations(self) -> None:
        for required in (
            "## Supported versions",
            "Latest published standalone CLI release",
            "No public production release is supported yet",
            "## Response expectations",
            "within 3 business days",
            "within 7 business days",
            "every 14 calendar days",
            "not guaranteed remediation deadlines",
        ):
            with self.subTest(required=required):
                self.assertIn(required, POLICY)

    def test_policy_keeps_sensitive_reports_and_evidence_private(self) -> None:
        for required in (
            "Do not put vulnerability details",
            "public issue",
            "Do not send real user files",
            "Replace personal paths and names with stable placeholders",
            "## Coordinated disclosure",
            "## Good-faith research",
            "Use only systems and data you own or are explicitly authorized to test",
        ):
            with self.subTest(required=required):
                self.assertIn(required, POLICY)

    def test_activation_is_an_external_release_gate_not_a_document_claim(self) -> None:
        self.assertIn(
            "gh api --method PUT repos/Mjukis-AB/dux/private-vulnerability-reporting",
            POLICY,
        )
        self.assertIn(
            "Activate and verify GitHub Private Vulnerability Reporting",
            SECURITY_DESIGN,
        )
        self.assertIn(
            "GitHub Private Vulnerability Reporting for the public repository was "
            "observed disabled during this checkpoint",
            ROADMAP,
        )
        self.assertIn(
            "The release gate remains open until a maintainer explicitly authorizes "
            "the external setting change",
            ROADMAP,
        )
        self.assertIn("an active private vulnerability-reporting channel", QUALIFICATION)
        self.assertIn(
            "a project-operated private vulnerability-reporting channel has been "
            "activated and verified",
            SECURITY_DESIGN,
        )

    def test_adr_0011_policy_decision_is_no_longer_listed_as_open(self) -> None:
        stale = (
            "policy explicitly decides whether durable non-executability is sufficient"
        )
        self.assertNotIn(stale, SECURITY_DESIGN)
        self.assertIn("ADR 0011's v1 durable non-executability policy", SECURITY_DESIGN)
        self.assertNotIn("prior-boot cleanup reconciliation", QUALIFICATION)


if __name__ == "__main__":
    unittest.main()
