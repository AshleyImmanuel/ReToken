import io
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path

from ledgerkit.__main__ import main

EXAMPLE = Path(__file__).resolve().parent.parent / "examples" / "household.journal"


def run(*argv):
    out, err = io.StringIO(), io.StringIO()
    with redirect_stdout(out), redirect_stderr(err):
        code = main(list(argv))
    return code, out.getvalue(), err.getvalue()


class CliTests(unittest.TestCase):
    def test_trial_balance(self):
        code, out, _ = run("tb", str(EXAMPLE))
        self.assertEqual(code, 0)
        self.assertIn("assets:checking", out)
        self.assertIn("Total USD", out)
        self.assertIn("Total EUR", out)

    def test_balance(self):
        code, out, _ = run("balance", str(EXAMPLE), "expenses", "--as-of", "2026-01-31")
        self.assertEqual(code, 0)
        self.assertIn("USD", out)

    def test_income(self):
        code, out, _ = run("income", str(EXAMPLE), "--start", "2026-01-01", "--end", "2026-01-31")
        self.assertEqual(code, 0)
        self.assertTrue(out.strip())

    def test_parse_error_exit_code(self):
        with tempfile.TemporaryDirectory() as tmp:
            bad = Path(tmp) / "bad.journal"
            bad.write_text("nonsense\n", encoding="utf-8")
            code, _, err = run("tb", str(bad))
        self.assertEqual(code, 1)
        self.assertIn("line 1", err)

    def test_missing_file(self):
        code, _, err = run("tb", "does-not-exist.journal")
        self.assertEqual(code, 2)
        self.assertIn("cannot read", err)


if __name__ == "__main__":
    unittest.main()
