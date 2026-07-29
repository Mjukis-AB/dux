import importlib.util
import pathlib
import shutil
import subprocess
import sys
import tempfile
import unittest


SCRIPT = pathlib.Path(__file__).resolve().parents[1] / "check_destructive_calls.py"
SPEC = importlib.util.spec_from_file_location("check_destructive_calls", SCRIPT)
assert SPEC and SPEC.loader
lint = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = lint
SPEC.loader.exec_module(lint)


class DestructiveCallLintTests(unittest.TestCase):
    def assert_rule(self, path: str, source: str, rule: str) -> None:
        self.assertIn(rule, {finding.rule for finding in lint.scan_source(path, source)})

    def test_rust_multiline_and_aliases_are_rejected(self) -> None:
        self.assert_rule(
            "src/example.rs",
            "std::fs::remove_dir_all\n(\n target\n);",
            "rust-filesystem-effect",
        )
        self.assert_rule(
            "src/example.rs",
            "use std::fs::remove_file as erase;\nfn go() { erase(target); }",
            "rust-filesystem-effect",
        )
        self.assert_rule(
            "src/example.rs",
            "use std::process::Command as C;\nfn go() { C::new(\"tool\"); }",
            "rust-process-spawn",
        )
        self.assert_rule(
            "src/example.rs",
            "fn go() { let launch = std::process::Command::new; launch(\"tool\"); }",
            "rust-process-spawn",
        )
        self.assert_rule(
            "src/example.rs",
            "unsafe { nix::libc::syscall(nix::libc::SYS_renameat2, a, b, c, d, flags); }",
            "rust-platform-delete",
        )
        self.assert_rule(
            "src/example.rs",
            "unsafe { renameatx_np(parent, from, parent, to, flags); }",
            "rust-platform-delete",
        )

    def test_comments_and_literals_do_not_trigger_rust_rules(self) -> None:
        source = '// std::fs::remove_file(path);\nlet text = "remove_dir_all(path)";\n'
        self.assertEqual(lint.scan_source("src/example.rs", source), [])

    def test_annotation_is_line_scoped_and_path_restricted(self) -> None:
        allowed = (
            "fn save_cache() {\n"
            "// DUX-DESTRUCTIVE: allow=cache-write-failure-temp-remove -- create-new temporary cache path is locally owned\n"
            "std::fs::remove_file(path);\n}\n"
        )
        self.assertEqual(lint.scan_source("dux-core/src/cache/mod.rs", allowed), [])
        self.assert_rule("src/example.rs", allowed, "invalid-annotation-scope")

    def test_one_annotation_cannot_cover_two_calls(self) -> None:
        source = (
            "fn save_cache() {\n"
            "// DUX-DESTRUCTIVE: allow=cache-write-failure-temp-remove -- create-new temporary cache owns both calls\n"
            "std::fs::remove_file(a); std::fs::remove_file(b);\n}\n"
        )
        self.assert_rule("dux-core/src/cache/mod.rs", source, "overbroad-annotation")

    def test_stale_and_malformed_annotations_fail(self) -> None:
        self.assert_rule(
            "dux-core/src/cache/mod.rs",
            "// DUX-DESTRUCTIVE: allow=cache-write-failure-temp-remove -- long but unused annotation reason\nfn safe() {}\n",
            "unused-annotation",
        )
        self.assert_rule(
            "src/example.rs",
            "// DUX-DESTRUCTIVE: allow=unknown-id -- short\nstd::fs::remove_file(path);\n",
            "invalid-annotation",
        )

    def test_test_only_requires_a_test_context(self) -> None:
        call = (
            "// DUX-DESTRUCTIVE: allow=test-cargo-config-transient-remove -- temporary directory is exclusively owned by this test\n"
            "std::fs::remove_file(path);\n"
        )
        self.assert_rule("src/example.rs", call, "invalid-annotation-scope")
        path = "dux-core/src/planner/cargo_config.rs"
        self.assert_rule(path, call, "invalid-annotation-scope")
        allowed_test = (
            "#[cfg(test)]\nmod tests {\nfn create_then_remove_is_still_a_terminal_vnode_event() {\n"
            + call
            + "}\n}\n"
        )
        self.assertEqual(lint.scan_source(path, allowed_test), [])
        escaped = "#[cfg(test)]\nmod tests {}\nfn product() {\n" + call + "}\n"
        self.assert_rule(path, escaped, "invalid-annotation-scope")

    def test_shell_traps_moves_and_find_delete_are_rejected(self) -> None:
        self.assert_rule("script.sh", "trap 'rm -rf \"$tmp\"' EXIT\n", "shell-remove")
        self.assert_rule("script.sh", "r''m -rf \"$tmp\"\n", "shell-remove")
        self.assert_rule("script.sh", '"rm" -rf "$tmp"\n', "shell-remove")
        self.assert_rule("script.sh", "r\\\nm -rf \"$tmp\"\n", "shell-remove")
        self.assert_rule("script.sh", "mv \"$from\" \"$to\"\n", "shell-move")
        self.assert_rule("script.sh", "find build -type f -delete\n", "shell-find-delete")
        self.assert_rule("script.sh", "sh -c \"$command\"\n", "shell-command-string")
        self.assert_rule("script.ps1", "Remove-Item -Recurse $path\n", "powershell-remove")
        self.assert_rule("script.sh", 'wipe() { rm -rf "$1"; }\n', "shell-remove")
        self.assert_rule("script.sh", 'function wipe { rm -rf "$1"; }; wipe target\n', "shell-remove")
        self.assert_rule("script.sh", 'wipe=rm; "$wipe" -rf target\n', "shell-remove")
        self.assert_rule("script.sh", "alias wipe='rm -rf'\nwipe target\n", "shell-remove")
        self.assert_rule("script.sh", "alias wipe='command rm -rf'; wipe target\n", "shell-remove")
        self.assert_rule("script.sh", "exec rm -rf target\n", "shell-remove")
        self.assert_rule("script.sh", "trap 'echo ready; rm -rf target' EXIT\n", "shell-remove")
        self.assert_rule("script.sh", '"$dynamic" target\n', "shell-indirect-command")

    def test_harmless_shell_strings_are_ignored(self) -> None:
        self.assertEqual(lint.scan_source("script.sh", 'echo "never invoke rm here"\n'), [])
        self.assertEqual(lint.scan_source("script.sh", "find build -type f -print -quit\n"), [])
        self.assertEqual(lint.scan_source("tool.ps1", 'Write-Host "do not use Remove-Item"\n'), [])
        self.assertEqual(lint.scan_source("tool.cmd", "echo never run del here\n"), [])

    def test_empty_shell_assignment_is_not_an_indirect_command(self) -> None:
        self.assertEqual(lint.scan_source("script.sh", 'output_path=""\n'), [])

    def test_nested_rust_comments_are_ignored(self) -> None:
        source = "/* outer /* std::fs::remove_file(path); */ still comment */\nfn safe() {}\n"
        self.assertEqual(lint.scan_source("src/example.rs", source), [])

    def test_unregistered_clippy_suppression_is_rejected(self) -> None:
        source = "#[allow(clippy::disallowed_methods)]\nfn product() {}\n"
        self.assert_rule("src/example.rs", source, "clippy-suppression-drift")
        self.assert_rule("src/example.rs", source, "invalid-clippy-suppression")
        conditional = "#[cfg_attr(target_os = \"macos\", allow(clippy::disallowed_methods))]\nfn product() {}\n"
        self.assert_rule("src/example.rs", conditional, "invalid-clippy-suppression")

    def test_swift_and_python_effects_are_rejected(self) -> None:
        self.assert_rule(
            "App.swift",
            "try FileManager.default.removeItem(at: url)\n",
            "swift-filesystem-effect",
        )
        self.assert_rule(
            "App.swift",
            "try FileManager.default.evictUbiquitousItem(at: url)\n",
            "swift-filesystem-effect",
        )
        self.assert_rule(
            "App.swift",
            "let erase = FileManager.default.removeItem\ntry erase(at: url)\n",
            "swift-filesystem-effect",
        )
        self.assert_rule("App.swift", "typealias Runner = Process\nRunner()\n", "swift-process-spawn")
        self.assert_rule("tool.py", "import shutil\nshutil.rmtree(path)\n", "python-filesystem-or-process-effect")
        self.assert_rule("tool.py", "import subprocess\nsubprocess.run(['rm'])\n", "python-filesystem-or-process-effect")
        self.assert_rule(
            "tool.py",
            "import subprocess as sp\nsp.run(['tool'])\n",
            "python-filesystem-or-process-effect",
        )
        self.assert_rule(
            "tool.py",
            "from os import unlink as erase\nerase(path)\n",
            "python-filesystem-or-process-effect",
        )
        self.assert_rule(
            "tool.py",
            "import os\nerase = os.remove\nerase(path)\n",
            "python-filesystem-or-process-effect",
        )
        self.assert_rule("tool.py", "open(path, 'w')\n", "python-truncation-effect")
        self.assert_rule(
            "tool.py",
            'import os\nwipe = getattr(os, "remove")\nwipe(path)\n',
            "python-filesystem-or-process-effect",
        )
        self.assert_rule(
            "tool.py",
            'import os\ngetattr(os, "remove")(path)\n',
            "python-filesystem-or-process-effect",
        )
        self.assert_rule(
            "tool.py",
            'import os\nops = {"wipe": os.remove}\nwipe = ops["wipe"]\nwipe(path)\n',
            "python-filesystem-or-process-effect",
        )
        self.assert_rule(
            "tool.py",
            'import os\nops = {"wipe": os.remove}\nops["wipe"](path)\n',
            "python-filesystem-or-process-effect",
        )
        self.assert_rule(
            "tool.py",
            "import os\nname = input()\nwipe = getattr(os, name)\n",
            "python-reflective-effect-access",
        )

    def test_c_family_and_platform_process_effects_are_rejected(self) -> None:
        source = "remove(path); rename(old, new); truncate(path, 0); system(command);\n"
        self.assert_rule("tool.c", source, "c-filesystem-effect")
        self.assert_rule("tool.c", source, "c-process-spawn")
        self.assert_rule("tool.ps1", "Start-Process tool\n", "powershell-process-spawn")
        self.assert_rule("tool.cmd", "call tool.cmd\n", "batch-process-spawn")
        self.assert_rule("tool.c", "void (*wipe)(char *) = remove;\n", "c-filesystem-effect")
        self.assert_rule("tool.c", "int (*run)(char *) = system;\n", "c-process-spawn")
        self.assert_rule("tool.c", "#define WIPE remove\nWIPE(path);\n", "c-filesystem-effect")
        self.assert_rule("tool.c", "#define RUN system\nRUN(command);\n", "c-process-spawn")

    def test_finder_exception_requires_real_fixed_command_shape(self) -> None:
        source = (
            "fn open_in_finder() {\n"
            "// DUX-DESTRUCTIVE: allow=finder-reveal -- fixed Finder reveal command has reviewed argument structure\n"
            'std::process::Command::new("rm"); // Command::new("open").arg("-R")\n'
            "}\n"
        )
        self.assert_rule("dux-cli/src/app/state.rs", source, "invalid-annotation-scope")

    def test_python_strings_do_not_trigger(self) -> None:
        self.assertEqual(lint.scan_source("tool.py", 'text = "shutil.rmtree(path)"\n'), [])

    def test_in_memory_and_thread_operations_are_ignored(self) -> None:
        source = "tree.remove_node(id); values.remove(&key); std::thread::spawn(worker);\n"
        self.assertEqual(lint.scan_source("src/example.rs", source), [])
        self.assertEqual(lint.scan_source("script.sh", "find build -type f -print -quit\n"), [])

    def test_retired_legacy_cli_architecture_cannot_be_reintroduced(self) -> None:
        module_name = "legacy" + "_cli"
        symbol_name = "Legacy" + "Cli" + "PermanentDelete"
        path = f"dux-core/src/cleanup/{module_name}.rs"

        self.assert_rule(
            "dux-core/src/cleanup/mod.rs",
            f"pub mod {module_name};\n",
            "retired-legacy-cli-architecture",
        )
        self.assert_rule(
            "dux-cli/src/app/state.rs",
            f"let adapter: {symbol_name}Worker;\n",
            "retired-legacy-cli-architecture",
        )
        self.assert_rule(
            "dux-ffi/src/lib.rs",
            f"use dux_core::cleanup::{module_name}::{symbol_name}Executor;\n",
            "retired-legacy-cli-architecture",
        )
        self.assert_rule(path, "pub struct SafePlaceholder;\n", "retired-legacy-cli-architecture")

    def test_retired_legacy_cli_markers_outside_product_rust_are_ignored(self) -> None:
        module_name = "legacy" + "_cli"
        symbol_name = "Legacy" + "Cli" + "PermanentDelete"
        self.assertEqual(
            lint.scan_source("examples/migration.rs", f"mod {module_name}; struct {symbol_name};\n"),
            [],
        )

    def test_xcframework_builder_rejects_arbitrary_output_before_build(self) -> None:
        builder = SCRIPT.parents[1] / "dux-macos/scripts/build-rust-xcframework.sh"
        # DUX-DESTRUCTIVE: allow=test-lint-rejected-xcframework-output -- execute fixed script with an inert rejected output argument
        result = subprocess.run(
            [str(builder), "/tmp/dux-unsafe-output.xcframework"],
            check=False,
            capture_output=True,
            text=True,
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("output must be the repository-owned XCFramework path", result.stderr)

    def test_xcframework_builder_rejects_symlinked_output_parent(self) -> None:
        source_builder = SCRIPT.parents[1] / "dux-macos/scripts/build-rust-xcframework.sh"
        with tempfile.TemporaryDirectory() as temporary:
            fake_repo = pathlib.Path(temporary) / "repo"
            scripts = fake_repo / "dux-macos/scripts"
            scripts.mkdir(parents=True)
            builder = scripts / source_builder.name
            shutil.copy2(source_builder, builder)
            external = pathlib.Path(temporary) / "external"
            external.mkdir()
            (fake_repo / "dux-macos/Generated").symlink_to(external, target_is_directory=True)
            # DUX-DESTRUCTIVE: allow=test-lint-symlinked-xcframework-parent -- run copied fixed script against isolated symlink fixture
            result = subprocess.run(
                [str(builder)],
                check=False,
                capture_output=True,
                text=True,
            )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("refusing a symlinked XCFramework output parent", result.stderr)

    def test_repository_inventory_includes_generated_untracked_and_executables(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            repo = pathlib.Path(temporary)
            # DUX-DESTRUCTIVE: allow=test-lint-fixture-git-init -- initialize only the isolated policy-test repository
            subprocess.run(["git", "init", "-q", str(repo)], check=True)
            generated = repo / "Generated.swift"
            generated.write_text("let erase = FileManager.default.removeItem\n", encoding="utf-8")
            # DUX-DESTRUCTIVE: allow=test-lint-fixture-git-add -- stage only the isolated generated policy fixture
            subprocess.run(["git", "-C", str(repo), "add", generated.name], check=True)
            command = repo / "cleanup.command"
            command.write_text("#!/bin/sh\nr''m -rf \"$target\"\n", encoding="utf-8")
            command.chmod(0o755)
            unknown = repo / "unknown-tool"
            unknown.write_text("#!/usr/bin/env ruby\nputs 'safe'\n", encoding="utf-8")
            unknown.chmod(0o755)

            findings, count = lint.scan_repository(repo, enforce_registry=False)

        self.assertEqual(count, 3)
        by_path = {(finding.path, finding.rule) for finding in findings}
        self.assertIn(("Generated.swift", "swift-filesystem-effect"), by_path)
        self.assertIn(("cleanup.command", "shell-remove"), by_path)
        self.assertIn(("unknown-tool", "unsupported-executable-language"), by_path)


if __name__ == "__main__":
    unittest.main()
