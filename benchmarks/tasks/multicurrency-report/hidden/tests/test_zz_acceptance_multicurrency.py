"""Acceptance tests for the multicurrency-report benchmark task.

The benchmark copies this file into the workspace only after the agent has
finished, so the agent never sees it.
"""

import io
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from decimal import Decimal
from pathlib import Path

from ledgerkit.__main__ import main
from ledgerkit.fx import MissingRate, RateTable
from ledgerkit.money import Money
from ledgerkit.parser import ParseError, load_ledger, parse_journal
from ledgerkit.report import trial_balance

JOURNAL = """\
rate EUR USD 1.0850
rate usd jpy 150
open 2026-01-01 assets:bank asset
open 2026-01-01 assets:wallet asset
open 2026-01-01 equity:opening equity
open 2026-01-01 expenses:food expense
open 2026-01-01 expenses:travel expense

2026-01-01 Opening
  assets:bank       1000.00 USD
  assets:wallet      200.00 EUR
  equity:opening   -1000.00 USD
  equity:opening    -200.00 EUR

2026-01-03 Dinner
  expenses:food       38.50 EUR
  assets:wallet

2026-01-04 Groceries
  expenses:food       61.25 USD
  assets:bank

2026-01-05 Train
  expenses:travel     64.00 EUR
  assets:wallet

rate EUR USD 1.1000
"""


def usd(amount):
    return Money.of(amount, "USD")


class RateDirectiveTests(unittest.TestCase):
    def test_rates_are_parsed_in_order(self):
        rates = parse_journal(JOURNAL).rates
        self.assertEqual(rates, [
            ("EUR", "USD", Decimal("1.0850")),
            ("USD", "JPY", Decimal("150")),
            ("EUR", "USD", Decimal("1.1000")),
        ])

    def test_load_ledger_accepts_rate_lines(self):
        ledger = load_ledger(JOURNAL)
        self.assertEqual(len(ledger), 4)

    def test_malformed_rate_lines(self):
        for bad in ("rate EUR USD", "rate EUR USD abc", "rate EUR USD -1",
                    "rate EUR USD 0", "rate EUR XYZ 1.2", "rate EUR USD 1.2 extra"):
            text = "open 2026-01-01 assets:bank asset\n" + bad + "\n"
            with self.assertRaises(ParseError, msg=bad) as ctx:
                parse_journal(text)
            self.assertEqual(ctx.exception.line, 2, bad)

    def test_from_directives_last_wins(self):
        table = RateTable.from_directives(parse_journal(JOURNAL).rates)
        self.assertEqual(table.rate("EUR", "USD"), Decimal("1.1000"))
        self.assertEqual(table.rate("USD", "JPY"), Decimal("150"))
        self.assertEqual(table.convert(Money.of("10", "EUR"), "USD"), usd("11.00"))


class ConvertedTrialBalanceTests(unittest.TestCase):
    def setUp(self):
        self.ledger = load_ledger(JOURNAL)
        self.rates = RateTable.from_directives([("EUR", "USD", Decimal("1.0850"))])

    def rows(self):
        rows = trial_balance(self.ledger, base_currency="USD", rates=self.rates)
        return {r.account: r for r in rows}

    def test_one_row_per_account_in_base_currency(self):
        rows = trial_balance(self.ledger, base_currency="USD", rates=self.rates)
        self.assertEqual(len(rows), len({r.account for r in rows}))
        self.assertTrue(all(r.currency == "USD" for r in rows))
        self.assertTrue(all(r.debit.currency == "USD" and r.credit.currency == "USD" for r in rows))

    def test_converted_amounts(self):
        rows = self.rows()
        # wallet: 200 - 38.50 - 64.00 = 97.50 EUR -> 105.7875 -> 105.79 USD
        self.assertEqual(rows["assets:wallet"].debit, usd("105.79"))
        # bank: 1000 - 61.25 USD
        self.assertEqual(rows["assets:bank"].debit, usd("938.75"))
        # opening: -1000 USD + (-200 EUR -> -217.00 USD)
        self.assertEqual(rows["equity:opening"].credit, usd("1217.00"))
        self.assertTrue(rows["equity:opening"].debit.is_zero())
        # food: 61.25 USD + (38.50 EUR -> 41.7725 -> 41.77 USD)
        self.assertEqual(rows["expenses:food"].debit, usd("103.02"))
        # travel: 64 EUR -> 69.44 USD
        self.assertEqual(rows["expenses:travel"].debit, usd("69.44"))

    def test_unconverted_behaviour_is_unchanged(self):
        rows = trial_balance(self.ledger)
        self.assertEqual(
            sorted((r.account, r.currency) for r in rows),
            sorted([
                ("assets:bank", "USD"), ("assets:wallet", "EUR"),
                ("equity:opening", "EUR"), ("equity:opening", "USD"),
                ("expenses:food", "EUR"), ("expenses:food", "USD"),
                ("expenses:travel", "EUR"),
            ]),
        )

    def test_missing_rate(self):
        with self.assertRaises(MissingRate):
            trial_balance(self.ledger, base_currency="GBP", rates=self.rates)

    def test_base_currency_needs_rates(self):
        with self.assertRaises(ValueError):
            trial_balance(self.ledger, base_currency="USD")

    def test_accounts_converting_to_zero_are_skipped(self):
        text = (
            "open 2026-01-01 assets:a asset\nopen 2026-01-01 assets:b asset\n\n"
            "2026-01-02 Move\n  assets:a 5.00 USD\n  assets:b -5.00 USD\n\n"
            "2026-01-03 Back\n  assets:b 5.00 USD\n  assets:a -5.00 USD\n"
        )
        rows = trial_balance(load_ledger(text), base_currency="USD", rates=RateTable())
        self.assertEqual(rows, [])


class CliTests(unittest.TestCase):
    def test_tb_base_option(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "books.journal"
            path.write_text(JOURNAL.replace("rate EUR USD 1.1000\n", ""), encoding="utf-8")
            out, err = io.StringIO(), io.StringIO()
            with redirect_stdout(out), redirect_stderr(err):
                code = main(["tb", str(path), "--base", "USD"])
        self.assertEqual(code, 0, err.getvalue())
        text = out.getvalue()
        self.assertIn("105.79 USD", text)
        self.assertIn("1,217.00 USD", text)
        self.assertNotIn("EUR", text)


if __name__ == "__main__":
    unittest.main()
