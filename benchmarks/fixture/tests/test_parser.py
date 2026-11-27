import unittest
from datetime import date

from ledgerkit.money import Money
from ledgerkit.parser import ParseError, load_ledger, parse_journal

JOURNAL = """\
; household books
open 2026-01-01 assets:bank asset
open 2026-01-01 income:salary income
open 2026-01-01 expenses:food expense

2026-01-05 January salary #payroll
  assets:bank      3,000.00 USD
  income:salary   -3000.00 USD   ; gross

2026-01-07 Coffee #food #small
  expenses:food       4.50 USD
  assets:bank
"""


class ParseTests(unittest.TestCase):
    def test_parse_structure(self):
        journal = parse_journal(JOURNAL)
        self.assertEqual(journal.accounts[0], (date(2026, 1, 1), "assets:bank", "asset"))
        self.assertEqual(len(journal.transactions), 2)
        coffee = journal.transactions[1]
        self.assertEqual(coffee.description, "Coffee")
        self.assertEqual(coffee.tags, ["food", "small"])
        self.assertEqual(coffee.postings[1], ("assets:bank", None))

    def test_load_fills_elided_amount(self):
        ledger = load_ledger(JOURNAL)
        self.assertEqual(ledger.balance("assets:bank"), {"USD": Money.of("2995.50", "USD")})

    def test_errors_carry_line_numbers(self):
        cases = {
            "2026-13-01 Bad month\n  a 1 USD\n": 1,
            "  assets:bank 1 USD\n": 1,
            "open 2026-01-01 a asset\n\nwhat is this\n": 3,
            "2026-01-01 x\n  a 1.0.0 USD\n": 2,
            "2026-01-01 #onlytag\n": 1,
        }
        for text, line in cases.items():
            with self.assertRaises(ParseError, msg=text) as ctx:
                load_ledger(text)
            self.assertEqual(ctx.exception.line, line, text)

    def test_two_elided_amounts(self):
        text = JOURNAL + "\n2026-01-08 Two\n  expenses:food\n  assets:bank\n"
        with self.assertRaises(ParseError):
            load_ledger(text)

    def test_unbalanced_reports_header_line(self):
        text = JOURNAL + "\n2026-01-09 Off\n  expenses:food 1 USD\n  assets:bank -2 USD\n"
        with self.assertRaises(ParseError) as ctx:
            load_ledger(text)
        self.assertEqual(ctx.exception.line, 14)


if __name__ == "__main__":
    unittest.main()
