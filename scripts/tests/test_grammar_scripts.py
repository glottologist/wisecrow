"""Exercise the host scripts without calling Docker, a model, or a speech provider."""

import fcntl
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest


LANGUAGES = ["gd", "fr", "it", "ga", "cy", "es"]
SCRIPTS = Path(__file__).resolve().parents[1]
FAKE_DOCKER = r'''
import json
import os
from pathlib import Path
import sys

args = sys.argv[1:]
with open(os.environ["GRAMMAR_TEST_CALLS"], "a") as calls:
    calls.write(json.dumps(args) + "\n")
if "postgres" in args:
    Path(os.environ["GRAMMAR_TEST_SQL"]).write_text(sys.stdin.read())
elif "wisecrow" in args:
    command = args[args.index("wisecrow") + 1]
    lang = args[args.index("--lang") + 1]
    if lang == os.environ.get("GRAMMAR_TEST_FAIL"):
        print("Error: provider unavailable", file=sys.stderr)
        sys.exit(23)
    if command == "prefetch-grammar-audio":
        missing = os.environ.get("GRAMMAR_TEST_MISSING", "0")
        verb = "Previewed" if "--dry-run" in args else "Voiced"
        print(f"INFO wisecrow: {verb} 2 {lang} example sentences (2 distinct): "
              f"cached 2, missing {missing}, generated 0, failed 0, unsupported 0; "
              "0 bytes generated; 0 clips pruned")
    elif command == "import-pdf":
        print("extractor noise preserved only in the full log")
        print(f"\x1b[32m INFO\x1b[0m wisecrow: Placed 1 {lang} A1 points")
    else:
        sys.exit(24)
'''


class GrammarScripts(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="wisecrow scripts ")
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        (self.root / "scripts").mkdir()
        for name in ("import-grammar.sh", "prefetch-grammar-audio.sh"):
            shutil.copy2(SCRIPTS / name, self.root / "scripts" / name)
        binary = self.root / "bin"
        binary.mkdir()
        docker = binary / "docker"
        docker.write_text(f"#!{sys.executable}\n{FAKE_DOCKER}")
        docker.chmod(0o755)
        self.calls_path = self.root / "calls.jsonl"
        self.sql_path = self.root / "coverage.sql"
        self.environment = {
            **os.environ,
            "PATH": f"{binary}{os.pathsep}{os.environ['PATH']}",
            "GRAMMAR_TEST_CALLS": str(self.calls_path),
            "GRAMMAR_TEST_SQL": str(self.sql_path),
        }

    def run_script(self, name, *args, **environment):
        return subprocess.run(
            ["bash", str(self.root / "scripts" / name), *args],
            cwd="/",
            env={**self.environment, **environment},
            capture_output=True,
            text=True,
            timeout=10,
            check=False,
        )

    def calls(self):
        if not self.calls_path.exists():
            return []
        return [json.loads(line) for line in self.calls_path.read_text().splitlines()]

    def test_audio_generates_and_verifies_each_language_from_any_directory(self):
        result = self.run_script("prefetch-grammar-audio.sh")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        calls = self.calls()
        self.assertEqual(len(calls), 12)
        for index, language in enumerate(LANGUAGES):
            generate, preview = calls[index * 2:index * 2 + 2]
            self.assertEqual(generate[-2:], ["--lang", language])
            self.assertEqual(preview, generate + ["--dry-run"])

    def test_audio_preview_does_not_generate_or_require_a_full_cache(self):
        result = self.run_script(
            "prefetch-grammar-audio.sh", "--preview", "fr", "es",
            GRAMMAR_TEST_MISSING="2",
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(len(self.calls()), 2)
        self.assertTrue(all(call[-1] == "--dry-run" for call in self.calls()))

    def test_audio_rejects_missing_clips_even_when_the_cli_exits_successfully(self):
        result = self.run_script("prefetch-grammar-audio.sh", GRAMMAR_TEST_MISSING="1")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(len(self.calls()), 2)
        self.assertIn("coverage is incomplete", result.stderr)

    def test_provider_failure_stops_both_scripts_and_remains_in_the_log(self):
        for name in ("import-grammar.sh", "prefetch-grammar-audio.sh"):
            with self.subTest(script=name):
                self.calls_path.write_text("")
                result = self.run_script(name, GRAMMAR_TEST_FAIL="gd")
                self.assertEqual(result.returncode, 23, result.stdout + result.stderr)
                jobs = [call for call in self.calls() if "wisecrow" in call]
                self.assertEqual(len(jobs), 1)
                self.assertFalse(self.sql_path.exists())
                self.assertTrue(any(
                    "provider unavailable" in log.read_text()
                    for log in (self.root / "logs").glob("*.log")
                ))

    def test_import_respects_source_levels_and_reports_coverage(self):
        result = self.run_script("import-grammar.sh")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        imports = [call for call in self.calls() if "import-pdf" in call]
        self.assertEqual(len(imports), 6)
        for call, language in zip(imports, LANGUAGES):
            self.assertEqual(call[-4:], ["--file", f"/app/grammar/{language}", "--lang", language])
            self.assertIn("--incremental", call)
            self.assertNotIn("--force", call)
            self.assertNotIn("--level", call)
            self.assertIn(f"Placed 1 {language} A1 points", result.stdout)
        sql = self.sql_path.read_text()
        self.assertIn("BEGIN READ ONLY", sql)
        self.assertIn("LEFT JOIN grammar_rules", sql)
        self.assertNotIn("GREATEST(30", sql)
        self.assertNotIn("extractor noise", result.stdout)
        self.assertTrue(any(
            "extractor noise" in log.read_text()
            for log in (self.root / "logs").glob("*.log")
        ))

    def test_invalid_languages_do_not_start_docker(self):
        for name in ("import-grammar.sh", "prefetch-grammar-audio.sh"):
            with self.subTest(script=name):
                result = self.run_script(name, "gd", "../../private")
                self.assertEqual(result.returncode, 2)
                self.assertEqual(self.calls(), [])

    def test_shared_lock_prevents_overlapping_runs(self):
        (self.root / "logs").mkdir()
        with (self.root / "logs" / "grammar.lock").open("w") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            for name in ("import-grammar.sh", "prefetch-grammar-audio.sh"):
                with self.subTest(script=name):
                    result = self.run_script(name)
                    self.assertEqual(result.returncode, 1)
                    self.assertIn("Another grammar", result.stderr)
                    self.assertEqual(self.calls(), [])


if __name__ == "__main__":
    unittest.main()
