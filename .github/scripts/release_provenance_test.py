import importlib.util
import contextlib
import copy
import io
import os
import re
import unittest
from pathlib import Path
from unittest import mock


MODULE_PATH = Path(__file__).with_name("release_provenance.py")
WORKFLOW_PATH = MODULE_PATH.parent.parent / "workflows/codex-plugin-release.yml"
SPEC = importlib.util.spec_from_file_location("release_provenance", MODULE_PATH)
assert SPEC and SPEC.loader
release_provenance = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(release_provenance)


class ReleaseProvenanceTest(unittest.TestCase):
    @staticmethod
    def valid_environment(wait_timer: int = 0):
        return {
            "can_admins_bypass": False,
            "protection_rules": [
                {
                    "id": 1,
                    "node_id": "reviewer-rule-node",
                    "type": "required_reviewers",
                    "prevent_self_review": True,
                    "reviewers": [{"type": "User", "reviewer": {"id": 1}}],
                },
                {
                    "id": 2,
                    "node_id": "wait-rule-node",
                    "type": "wait_timer",
                    "wait_timer": wait_timer,
                },
                {
                    "id": 3,
                    "node_id": "branch-rule-node",
                    "type": "branch_policy",
                },
            ],
            "deployment_branch_policy": {
                "protected_branches": False,
                "custom_branch_policies": True,
            },
        }

    @staticmethod
    def valid_policies():
        return {
            "total_count": 3,
            "branch_policies": [
                {
                    "id": 11,
                    "node_id": "stage-policy-node",
                    "name": "stage",
                    "type": "branch",
                },
                {
                    "id": 12,
                    "node_id": "main-policy-node",
                    "name": "main",
                    "type": "branch",
                },
                {
                    "id": 13,
                    "node_id": "tag-policy-node",
                    "name": "v*",
                    "type": "tag",
                },
            ],
        }

    def assert_environment_rejected(
        self, environment, expected: str, policies=None
    ) -> None:
        if policies is None:
            policies = self.valid_policies()
        with mock.patch.object(
            release_provenance, "api_get", side_effect=[environment, policies]
        ):
            with self.assertRaisesRegex(SystemExit, expected):
                release_provenance.verify_environment()

    def test_release_actions_are_pinned_and_credentials_are_gated(self) -> None:
        workflow = WORKFLOW_PATH.read_text(encoding="utf-8")
        self.assertIn('- ".github/scripts/**"', workflow)
        uses = re.findall(r"^\s*-?\s*uses:\s*([^\s#]+)", workflow, re.MULTILINE)
        self.assertGreater(len(uses), 0)
        for action in uses:
            self.assertRegex(action, r"^[^@]+@[0-9a-f]{40}$")

        production_start = workflow.index("\n  production-native:")
        production_end = workflow.index("\n  verify-native-artifacts:")
        production = workflow[production_start:production_end]
        signing_start = workflow.index("\n  sign-release-archive:")
        signing_end = workflow.index("\n  publish-marketplace:")
        signing = workflow[signing_start:signing_end]
        self.assertIn("needs: [release-provenance, native-build]", production)
        self.assertIn("needs: [release-provenance, assemble]", signing)
        self.assertIn("name: codex-plugin-production", production)
        self.assertIn("name: codex-plugin-production", signing)
        self.assertIn("permissions:\n      contents: read\n      id-token: write", signing)
        self.assertNotIn("contents: write", signing)
        self.assertIn("if: always() && matrix.platform_signing == 'apple'", production)
        self.assertIn('security delete-keychain "$keychain_path"', production)
        self.assertIn('rm -f -- "$cert_path" "$keychain_path"', production)

        provenance_start = workflow.index("\n  release-provenance:")
        provenance_end = workflow.index("\n  native-build:")
        provenance = workflow[provenance_start:provenance_end]
        self.assertIn("persist-credentials: false", provenance)
        self.assertNotIn("git fetch", provenance)
        self.assertNotIn("http.extraheader", provenance)
        self.assertNotIn("credential.helper", provenance)

        apple_secrets = (
            "MACOS_CERTIFICATE_P12_BASE64",
            "MACOS_CERTIFICATE_PASSWORD",
            "MACOS_SIGNING_IDENTITY",
            "APPLE_TEAM_ID",
            "MACOS_KEYCHAIN_PASSWORD",
            "APPLE_ID",
            "APPLE_APP_SPECIFIC_PASSWORD",
        )
        for secret in apple_secrets:
            reference = "${{ secrets." + secret + " }}"
            self.assertGreater(workflow.count(reference), 0)
            self.assertEqual(workflow.count(reference), production.count(reference))

    def test_environment_gate_accepts_only_complete_protected_schema(self) -> None:
        for wait_timer in (0, 43_200):
            environment = self.valid_environment(wait_timer)
            environment["future_top_level_field"] = {"compatible": True}
            environment["protection_rules"][0]["future_rule_field"] = "accepted"
            environment["protection_rules"][0]["reviewers"][0][
                "future_reviewer_field"
            ] = 1
            policies = self.valid_policies()
            policies["future_collection_field"] = None
            policies["branch_policies"][0]["future_policy_field"] = []
            with self.subTest(wait_timer=wait_timer), mock.patch.object(
                release_provenance, "api_get", side_effect=[environment, policies]
            ):
                release_provenance.verify_environment()

    def test_environment_gate_rejects_negative_and_malformed_schemas(self) -> None:
        valid_environment = self.valid_environment()
        malformed_cases = (
            (None, None, "response must be an object"),
            (
                {"can_admins_bypass": False},
                None,
                "protection_rules must be a list",
            ),
            (
                {
                    "can_admins_bypass": False,
                    "protection_rules": "not-a-list",
                    "deployment_branch_policy": {},
                },
                None,
                "protection_rules must be a list",
            ),
            (valid_environment, [], "API response must be an object"),
            (
                valid_environment,
                {"branch_policies": []},
                "total_count must be a non-negative integer",
            ),
            (
                valid_environment,
                {"total_count": 1, "branch_policies": ["not-an-object"]},
                "branch policies must be objects",
            ),
            (
                valid_environment,
                {"total_count": 0, "branch_policies": []},
                "must exclusively allow branch policies",
            ),
        )
        for environment, policies, expected in malformed_cases:
            with self.subTest(expected=expected):
                self.assert_environment_rejected(environment, expected, policies)

    def test_environment_gate_requires_admin_bypass_to_be_exactly_false(self) -> None:
        cases = []
        environment = self.valid_environment()
        del environment["can_admins_bypass"]
        cases.append(environment)
        for bad_value in (True, None, 0, "false"):
            environment = self.valid_environment()
            environment["can_admins_bypass"] = bad_value
            cases.append(environment)

        for environment in cases:
            with self.subTest(value=environment.get("can_admins_bypass", "missing")):
                self.assert_environment_rejected(
                    environment, "must disable administrator protection bypass"
                )

    def test_environment_gate_rejects_bad_rule_identity_and_cardinality(self) -> None:
        cases = []
        environment = self.valid_environment()
        del environment["protection_rules"][0]["id"]
        cases.append((environment, "protection rule id must be a positive integer"))
        for bad_id in (None, True, -1, 0, "1"):
            environment = self.valid_environment()
            environment["protection_rules"][0]["id"] = bad_id
            cases.append((environment, "protection rule id must be a positive integer"))
        environment = self.valid_environment()
        del environment["protection_rules"][0]["node_id"]
        cases.append((environment, "protection rule node_id must be a non-empty string"))
        for bad_node_id in (None, ""):
            environment = self.valid_environment()
            environment["protection_rules"][0]["node_id"] = bad_node_id
            cases.append((environment, "protection rule node_id must be a non-empty string"))
        for duplicate_field in ("id", "node_id"):
            environment = self.valid_environment()
            environment["protection_rules"][1][duplicate_field] = environment[
                "protection_rules"
            ][0][duplicate_field]
            cases.append((environment, "protection rules must be unique"))
        for duplicate_type in ("required_reviewers", "wait_timer", "branch_policy"):
            environment = self.valid_environment()
            duplicate = copy.deepcopy(
                next(
                    rule
                    for rule in environment["protection_rules"]
                    if rule["type"] == duplicate_type
                )
            )
            duplicate["id"] = 99
            duplicate["node_id"] = "duplicate-node"
            environment["protection_rules"].append(duplicate)
            cases.append((environment, f"duplicate {duplicate_type} rules"))
        environment = self.valid_environment()
        for rule_id in range(4, 8):
            environment["protection_rules"].append(
                {
                    "id": rule_id,
                    "node_id": f"extra-{rule_id}",
                    "type": "wait_timer",
                    "wait_timer": 0,
                }
            )
        cases.append((environment, "duplicate wait_timer rules"))
        environment = self.valid_environment()
        del environment["protection_rules"][0]
        cases.append((environment, "exactly one required-reviewers rule"))

        for environment, expected in cases:
            with self.subTest(expected=expected):
                self.assert_environment_rejected(environment, expected)

    def test_environment_gate_rejects_malformed_wait_and_rule_variants(self) -> None:
        cases = []
        environment = self.valid_environment()
        del environment["protection_rules"][1]["wait_timer"]
        cases.append((environment, "wait_timer must be an integer from 0 to 43200"))
        for bad_wait in (None, True, "30", -1, 43_201):
            environment = self.valid_environment()
            environment["protection_rules"][1]["wait_timer"] = bad_wait
            cases.append((environment, "wait_timer must be an integer from 0 to 43200"))
        for rule_index, field, value in (
            (0, "wait_timer", 0),
            (1, "reviewers", []),
            (1, "prevent_self_review", True),
            (2, "wait_timer", 0),
            (2, "reviewers", []),
            (2, "prevent_self_review", True),
        ):
            environment = self.valid_environment()
            environment["protection_rules"][rule_index][field] = value
            cases.append((environment, "fields from another rule variant"))
        environment = self.valid_environment()
        del environment["protection_rules"][2]
        cases.append((environment, "exactly one branch-policy rule"))

        for environment, expected in cases:
            with self.subTest(expected=expected):
                self.assert_environment_rejected(environment, expected)

    def test_environment_gate_rejects_malformed_deployment_policy_flags(self) -> None:
        cases = []
        for field in ("protected_branches", "custom_branch_policies"):
            environment = self.valid_environment()
            del environment["deployment_branch_policy"][field]
            cases.append(environment)
        for field, value in (
            ("protected_branches", True),
            ("protected_branches", 0),
            ("custom_branch_policies", False),
            ("custom_branch_policies", 1),
        ):
            environment = self.valid_environment()
            environment["deployment_branch_policy"][field] = value
            cases.append(environment)

        for environment in cases:
            with self.subTest(policy=environment["deployment_branch_policy"]):
                self.assert_environment_rejected(
                    environment, "must use custom branch and tag deployment policies"
                )

    def test_environment_gate_rejects_malformed_required_reviewers(self) -> None:
        base = self.valid_environment()

        def with_reviewers(reviewers):
            value = copy.deepcopy(base)
            value["protection_rules"][0]["reviewers"] = reviewers
            return value

        duplicate = [
            {"type": "User", "reviewer": {"id": 1}},
            {"type": "User", "reviewer": {"id": 1}},
        ]
        malformed_cases = (
            (with_reviewers(None), "one to six required reviewers"),
            (with_reviewers([]), "one to six required reviewers"),
            (with_reviewers([None]), "reviewer entries must be objects"),
            (with_reviewers([{}]), "reviewer type must be User or Team"),
            (
                with_reviewers(
                    [{"type": "Organization", "reviewer": {"id": 1}}]
                ),
                "reviewer type must be User or Team",
            ),
            (
                with_reviewers([{"type": "User"}]),
                "reviewer payload must be an object",
            ),
            (
                with_reviewers([{"type": "User", "reviewer": None}]),
                "reviewer payload must be an object",
            ),
            (
                with_reviewers([{"type": "User", "reviewer": {}}]),
                "reviewer id must be a positive integer",
            ),
            (
                with_reviewers([{"type": "User", "reviewer": {"id": True}}]),
                "reviewer id must be a positive integer",
            ),
            (
                with_reviewers([{"type": "Team", "reviewer": {"id": 0}}]),
                "reviewer id must be a positive integer",
            ),
            (with_reviewers(duplicate), "required reviewers must be unique"),
            (
                with_reviewers(
                    [
                        {"type": "User", "reviewer": {"id": reviewer_id}}
                        for reviewer_id in range(1, 8)
                    ]
                ),
                "one to six required reviewers",
            ),
        )
        no_self_review = copy.deepcopy(base)
        no_self_review["protection_rules"][0]["prevent_self_review"] = False
        malformed_cases += ((no_self_review, "must prevent self-review"),)
        extra_rule_cases = (
            (
                {**copy.deepcopy(base), "protection_rules": [base["protection_rules"][0], None]},
                "protection rules must be objects",
            ),
            (
                {**copy.deepcopy(base), "protection_rules": [base["protection_rules"][0], {}]},
                "missing or unsupported type",
            ),
            (
                {
                    **copy.deepcopy(base),
                    "protection_rules": [
                        base["protection_rules"][0],
                        {"type": "unknown_future_rule"},
                    ],
                },
                "missing or unsupported type",
            ),
        )
        for environment, expected in malformed_cases + extra_rule_cases:
            with self.subTest(expected=expected):
                self.assert_environment_rejected(environment, expected)

    def test_environment_gate_rejects_malformed_or_duplicate_policy_entities(self) -> None:
        cases = []
        policies = self.valid_policies()
        del policies["branch_policies"][0]["id"]
        cases.append((policies, "branch policy id must be a positive integer"))
        for bad_id in (None, True, -1, 0, "11"):
            policies = self.valid_policies()
            policies["branch_policies"][0]["id"] = bad_id
            cases.append((policies, "branch policy id must be a positive integer"))
        policies = self.valid_policies()
        del policies["branch_policies"][0]["node_id"]
        cases.append((policies, "branch policy node_id must be a non-empty string"))
        for bad_node_id in (None, ""):
            policies = self.valid_policies()
            policies["branch_policies"][0]["node_id"] = bad_node_id
            cases.append((policies, "branch policy node_id must be a non-empty string"))
        for bad_name in (None, ""):
            policies = self.valid_policies()
            policies["branch_policies"][0]["name"] = bad_name
            cases.append((policies, "branch policy name must be a non-empty string"))
        for bad_type in (None, "environment"):
            policies = self.valid_policies()
            policies["branch_policies"][0]["type"] = bad_type
            cases.append((policies, "branch policy type must be branch or tag"))
        for duplicate_field in ("id", "node_id"):
            policies = self.valid_policies()
            policies["branch_policies"][1][duplicate_field] = policies[
                "branch_policies"
            ][0][duplicate_field]
            cases.append((policies, "branch policies must be unique"))
        policies = self.valid_policies()
        duplicate = copy.deepcopy(policies["branch_policies"][0])
        duplicate["id"] = 99
        duplicate["node_id"] = "duplicate-tuple-node"
        policies["branch_policies"].append(duplicate)
        policies["total_count"] = 4
        cases.append((policies, "branch policies must be unique"))
        policies = self.valid_policies()
        policies["branch_policies"][0]["name"] = "*"
        cases.append((policies, "must exclusively allow branch policies"))

        for policies, expected in cases:
            with self.subTest(expected=expected):
                self.assert_environment_rejected(
                    self.valid_environment(), expected, policies
                )

    def test_branch_policy_pagination_loads_all_declared_entries(self) -> None:
        exact = [
            {"id": 1, "node_id": "stage", "name": "stage", "type": "branch"},
            {"id": 2, "node_id": "main", "name": "main", "type": "branch"},
            {"id": 3, "node_id": "tag", "name": "v*", "type": "tag"},
        ]
        first_page = [exact[index % len(exact)] for index in range(100)]
        requested: list[str] = []

        def get_page(path: str):
            requested.append(path)
            if path.endswith("page=1"):
                return {"total_count": 101, "branch_policies": first_page}
            if path.endswith("page=2"):
                return {"total_count": 101, "branch_policies": [exact[0]]}
            self.fail(f"unexpected policy page: {path}")

        policies = release_provenance.paginated_branch_policies(
            get_page, "environments/prod/deployment-branch-policies"
        )

        self.assertEqual(len(policies), 101)
        self.assertEqual(len(requested), 2)
        self.assertEqual(
            {(policy["name"], policy["type"]) for policy in policies},
            {("stage", "branch"), ("main", "branch"), ("v*", "tag")},
        )

    def test_branch_policy_pagination_rejects_late_extra_and_truncation(self) -> None:
        exact = [
            {"id": 1, "node_id": "stage", "name": "stage", "type": "branch"},
            {"id": 2, "node_id": "main", "name": "main", "type": "branch"},
            {"id": 3, "node_id": "tag", "name": "v*", "type": "tag"},
        ]
        first_page = [exact[index % len(exact)] for index in range(100)]

        def late_extra(path: str):
            if path.endswith("page=1"):
                return {"total_count": 101, "branch_policies": first_page}
            return {
                "total_count": 101,
                "branch_policies": [
                    {
                        "id": 101,
                        "node_id": "late-extra",
                        "name": "*",
                        "type": "branch",
                    }
                ],
            }

        policies = release_provenance.paginated_branch_policies(
            late_extra, "environments/prod/deployment-branch-policies"
        )
        configured = {(policy["name"], policy["type"]) for policy in policies}
        self.assertIn(("*", "branch"), configured)
        self.assertNotEqual(
            configured,
            {("stage", "branch"), ("main", "branch"), ("v*", "tag")},
        )

        environment = self.valid_environment()

        def gate_api(path: str):
            if path == "environments/codex-plugin-production":
                return environment
            return late_extra(path)

        with mock.patch.object(
            release_provenance, "api_get", side_effect=gate_api
        ):
            with self.assertRaisesRegex(
                SystemExit, "branch policies must be unique"
            ):
                release_provenance.verify_environment()

        with self.assertRaisesRegex(SystemExit, "ended before total_count"):
            release_provenance.paginated_branch_policies(
                lambda _path: {"total_count": 101, "branch_policies": exact},
                "environments/prod/deployment-branch-policies",
            )

        inconsistent_calls = 0

        def inconsistent_total(_path: str):
            nonlocal inconsistent_calls
            inconsistent_calls += 1
            if inconsistent_calls == 1:
                return {"total_count": 101, "branch_policies": first_page}
            return {"total_count": 102, "branch_policies": [exact[0]]}

        with self.assertRaisesRegex(SystemExit, "changed between pages"):
            release_provenance.paginated_branch_policies(
                inconsistent_total,
                "environments/prod/deployment-branch-policies",
            )

    def test_main_emits_outputs_only_for_exact_approved_merge(self) -> None:
        release_sha = "a" * 40
        first_parent = "b" * 40
        head_sha = "c" * 40
        pull = {
            "number": 42,
            "merged_at": "2026-07-21T00:00:00Z",
            "merge_commit_sha": release_sha,
            "base": {"ref": "main"},
            "head": {"sha": head_sha},
            "user": {"login": "author"},
        }
        approval = {
            "id": 9,
            "state": "APPROVED",
            "commit_id": head_sha,
            "user": {"login": "reviewer"},
        }

        def fake_api(path: str):
            if path == f"commits/{release_sha}/pulls?per_page=100&page=1":
                return [pull]
            if path == f"compare/{release_sha}...main":
                return {
                    "status": "ahead",
                    "base_commit": {"sha": release_sha},
                    "merge_base_commit": {"sha": release_sha},
                }
            if path == "pulls/42/reviews?per_page=100&page=1":
                return [approval]
            self.fail(f"unexpected API path: {path}")

        def fake_git(*args: str):
            if args == ("rev-list", "--parents", "-n", "1", release_sha):
                return f"{release_sha} {first_parent} {head_sha}"
            if args == ("show", "-s", "--format=%s", release_sha):
                return "Merge pull request #42 from feature/release"
            self.fail(f"unexpected git arguments: {args}")

        stdout = io.StringIO()
        stderr = io.StringIO()
        with (
            mock.patch.dict(os.environ, {"RELEASE_SHA": release_sha}),
            mock.patch.object(release_provenance, "git", side_effect=fake_git),
            mock.patch.object(release_provenance, "api_get", side_effect=fake_api),
            mock.patch.object(
                release_provenance, "verify_environment"
            ) as environment_gate,
            contextlib.redirect_stdout(stdout),
            contextlib.redirect_stderr(stderr),
        ):
            release_provenance.main()

        environment_gate.assert_called_once_with()
        self.assertEqual(
            stdout.getvalue(),
            "release-mode=true\n"
            f"release-sha={release_sha}\n"
            "release-base=main\n"
            "release-pr=42\n",
        )
        self.assertIn(
            f"verified {release_sha} as approved standard merge of PR #42 into main",
            stderr.getvalue(),
        )

    def test_main_rejects_bad_parents_exact_pr_and_subject(self) -> None:
        release_sha = "a" * 40
        first_parent = "b" * 40
        head_sha = "c" * 40
        with (
            mock.patch.dict(os.environ, {"RELEASE_SHA": release_sha}),
            mock.patch.object(
                release_provenance,
                "git",
                return_value=f"{release_sha} {first_parent}",
            ),
            mock.patch.object(release_provenance, "verify_environment"),
        ):
            with self.assertRaisesRegex(SystemExit, "not a two-parent merge"):
                release_provenance.main()

        wrong_pull = {
            "number": 42,
            "merged_at": "2026-07-21T00:00:00Z",
            "merge_commit_sha": "d" * 40,
            "base": {"ref": "main"},
        }
        with (
            mock.patch.dict(os.environ, {"RELEASE_SHA": release_sha}),
            mock.patch.object(
                release_provenance,
                "git",
                return_value=f"{release_sha} {first_parent} {head_sha}",
            ),
            mock.patch.object(release_provenance, "verify_environment"),
            mock.patch.object(
                release_provenance, "api_get", return_value=[wrong_pull]
            ),
        ):
            with self.assertRaisesRegex(SystemExit, "exact merge_commit_sha"):
                release_provenance.main()

        valid_pull = {
            "number": 42,
            "merged_at": "2026-07-21T00:00:00Z",
            "merge_commit_sha": release_sha,
            "base": {"ref": "main"},
            "head": {"sha": head_sha},
            "user": {"login": "author"},
        }

        def subject_api(path: str):
            if path.startswith(f"commits/{release_sha}/pulls?"):
                return [valid_pull]
            if path == f"compare/{release_sha}...main":
                return {
                    "status": "ahead",
                    "base_commit": {"sha": release_sha},
                    "merge_base_commit": {"sha": release_sha},
                }
            self.fail(f"unexpected API path before subject rejection: {path}")

        def bad_subject_git(*args: str):
            if args[0] == "rev-list":
                return f"{release_sha} {first_parent} {head_sha}"
            return "custom merge title"

        with (
            mock.patch.dict(os.environ, {"RELEASE_SHA": release_sha}),
            mock.patch.object(
                release_provenance, "git", side_effect=bad_subject_git
            ),
            mock.patch.object(release_provenance, "verify_environment"),
            mock.patch.object(
                release_provenance, "api_get", side_effect=subject_api
            ),
        ):
            with self.assertRaisesRegex(SystemExit, "not a standard GitHub PR merge"):
                release_provenance.main()

    def test_compare_api_proves_private_branch_ancestry_without_fetch(self) -> None:
        release_sha = "a" * 40
        ahead = {
            "status": "ahead",
            "base_commit": {"sha": release_sha},
            "merge_base_commit": {"sha": release_sha},
        }
        identical = {
            "status": "identical",
            "base_commit": {"sha": release_sha},
            "merge_base_commit": {"sha": release_sha},
        }
        diverged = {
            "status": "diverged",
            "base_commit": {"sha": release_sha},
            "merge_base_commit": {"sha": "b" * 40},
        }

        self.assertTrue(
            release_provenance.comparison_contains_release(ahead, release_sha)
        )
        self.assertTrue(
            release_provenance.comparison_contains_release(identical, release_sha)
        )
        self.assertFalse(
            release_provenance.comparison_contains_release(diverged, release_sha)
        )

    def test_stale_approval_is_rejected_after_head_changes(self) -> None:
        old_head = "1" * 40
        current_head = "2" * 40
        reviews = [
            {
                "id": 1,
                "state": "APPROVED",
                "commit_id": old_head,
                "user": {"login": "reviewer"},
            }
        ]

        self.assertEqual(
            release_provenance.current_head_approvers(
                reviews, "author", current_head
            ),
            [],
        )

    def test_later_dismissal_invalidates_head_approval(self) -> None:
        head = "2" * 40
        reviews = [
            {
                "id": 1,
                "state": "APPROVED",
                "commit_id": head,
                "user": {"login": "reviewer"},
            },
            {
                "id": 2,
                "state": "DISMISSED",
                "commit_id": head,
                "user": {"login": "reviewer"},
            },
        ]

        self.assertEqual(
            release_provenance.current_head_approvers(reviews, "author", head),
            [],
        )

    def test_reviews_after_first_hundred_are_loaded_and_can_approve(self) -> None:
        head = "2" * 40
        first_page = [
            {
                "id": review_id,
                "state": "COMMENTED",
                "commit_id": head,
                "user": {"login": f"commenter-{review_id}"},
            }
            for review_id in range(1, 101)
        ]
        final_approval = {
            "id": 101,
            "state": "APPROVED",
            "commit_id": head,
            "user": {"login": "late-reviewer"},
        }
        requested: list[str] = []

        def get_page(path: str):
            requested.append(path)
            if path.endswith("page=1"):
                return first_page
            if path.endswith("page=2"):
                return [final_approval]
            self.fail(f"unexpected page request: {path}")

        reviews = release_provenance.paginated_list(
            get_page, "pulls/42/reviews"
        )

        self.assertEqual(len(reviews), 101)
        self.assertEqual(len(requested), 2)
        self.assertEqual(
            release_provenance.current_head_approvers(reviews, "author", head),
            ["late-reviewer"],
        )


if __name__ == "__main__":
    unittest.main()
