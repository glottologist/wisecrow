"""The host units prepare corpus and grammar without the incrementing ingest path."""

from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[2]
TEMPLATES = ROOT / "ansible" / "roles" / "wisecrow" / "templates"


class PipelineUnits(unittest.TestCase):
    def test_corpus_unit_prepares_one_step_and_never_ingests(self):
        unit = (TEMPLATES / "wisecrow-corpus.service.j2").read_text()
        script = (ROOT / "scripts" / "prepare-corpus.sh").read_text()
        self.assertIn("Type=oneshot", unit)
        self.assertIn("Restart=on-success", unit)
        self.assertIn("scripts/prepare-corpus.sh", unit)
        self.assertIn(
            "wisecrow prepare-corpus --config /app/language-pairs.toml",
            script,
        )
        self.assertIn("sleep 60", script)
        self.assertNotIn("wisecrow ingest", unit)
        self.assertNotIn("wisecrow ingest", script)

    def test_grammar_unit_imports_only_after_pending_status(self):
        unit = (TEMPLATES / "wisecrow-grammar.service.j2").read_text()
        script = (ROOT / "scripts" / "grammar-service.sh").read_text()
        self.assertIn("Type=oneshot", unit)
        self.assertIn("Restart=on-success", unit)
        self.assertIn("scripts/grammar-service.sh", unit)
        status = script.index("grammar-status")
        idle_exit = script.index("exit 0")
        importer = script.index("scripts/import-grammar.sh")
        audio = script.index("scripts/prefetch-grammar-audio.sh")
        self.assertLess(status, idle_exit)
        self.assertLess(idle_exit, importer)
        self.assertLess(importer, audio)
        self.assertNotIn("wisecrow ingest", unit)
        self.assertNotIn("wisecrow ingest", script)

    def test_compose_mounts_the_pair_file_read_only(self):
        compose = (ROOT / "docker-compose.deploy.yml").read_text()
        self.assertIn(
            "./deploy/language-pairs.toml:/app/language-pairs.toml:ro",
            compose,
        )

    def test_ansible_enables_the_units_without_starting_them(self):
        tasks = (ROOT / "ansible" / "roles" / "wisecrow" / "tasks" / "main.yml").read_text()
        self.assertIn("wisecrow-corpus.service", tasks)
        self.assertIn("wisecrow-grammar.service", tasks)
        enabled = tasks.split("Enable the corpus and grammar units", 1)[1]
        self.assertIn("enabled: true", enabled)
        self.assertNotIn("state: started", enabled)


if __name__ == "__main__":
    unittest.main()
