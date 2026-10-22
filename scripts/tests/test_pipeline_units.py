"""The host units prepare corpus and grammar without the incrementing ingest path."""

from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[2]
TEMPLATES = ROOT / "ansible" / "roles" / "wisecrow" / "templates"


class PipelineUnits(unittest.TestCase):
    def test_corpus_unit_prepares_one_step_and_never_ingests(self):
        unit = (TEMPLATES / "wisecrow-corpus.service.j2").read_text()
        script = (ROOT / "scripts" / "prepare-corpus.sh").read_text()
        self.assertIn("Type=simple", unit)
        self.assertIn("Restart=on-failure", unit)
        self.assertNotIn("Restart=on-success", unit)
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
        self.assertIn("Type=simple", unit)
        self.assertIn("Restart=on-failure", unit)
        self.assertNotIn("Restart=on-success", unit)
        self.assertIn("scripts/grammar-service.sh", unit)
        status = script.index("grammar-status")
        idle_wait = script.index("sleep 60")
        importer = script.index("scripts/import-grammar.sh")
        audio = script.index("scripts/prefetch-grammar-audio.sh")
        self.assertLess(status, idle_wait)
        self.assertLess(idle_wait, importer)
        self.assertLess(importer, audio)
        self.assertNotIn("wisecrow ingest", unit)
        self.assertNotIn("wisecrow ingest", script)

    def test_import_grammar_reads_pairs_and_skips_unconfirmed_gaelic(self):
        script = (ROOT / "scripts" / "import-grammar.sh").read_text()
        self.assertIn("deploy/language-pairs.toml", script)
        self.assertIn("classify-rules", script)
        self.assertIn("classify-words", script)
        self.assertIn("generate-paradigms", script)
        self.assertNotIn("languages=(gd fr it ga cy es)", script)
        self.assertIn(
            "Skipping Gaelic paradigms until the slot table is confirmed",
            script,
        )

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

    def test_reader_unit_prepares_one_step_from_the_shelf(self):
        script = (ROOT / "scripts" / "prepare-readers.sh").read_text()
        compose = (ROOT / "docker-compose.deploy.yml").read_text()
        tasks = (ROOT / "ansible" / "roles" / "wisecrow" / "tasks" / "main.yml").read_text()
        self.assertIn("reader-status", script)
        self.assertIn("prepare-readers", script)
        self.assertIn("/app/readers", script)
        self.assertIn("language-pairs.toml", script)
        self.assertIn("75", script)
        self.assertNotIn("--force", script)
        self.assertIn("./readers:/app/readers:ro", compose)
        self.assertIn("wisecrow-reader.service", tasks)


if __name__ == "__main__":
    unittest.main()
