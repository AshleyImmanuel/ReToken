import unittest
from datetime import date

from ledgerkit.ledger import Ledger, Posting
from ledgerkit.money import Money
from ledgerkit.report import format_trial_balance, income_statement, totals, trial_balance


def usd(amount):
    return Money.of(amount, "USD")


class ReportTests(unittest.TestCase):
    def setUp(self):
        ledger = Ledger()
        ledger.open_account("assets:bank", "asset")
        ledger.open_account("liabilities:card", "liability")
        ledger.open_account("income:salary", "income")
        ledger.open_account("expenses:food", "expense")
        ledger.post(date(2026, 1, 5), "Salary", [
            Posting("assets:bank", usd("3000")), Posting("income:salary", usd("-3000"))])
        ledger.post(date(2026, 1, 9), "Dinner on card", [
            Posting("expenses:food", usd("62.10")), Posting("liabilities:card", usd("-62.10"))])
        ledger.post(date(2026, 2, 1), "Euro coffee", [
            Posting("expenses:food", Money.of("3.20", "EUR")),
            Posting("assets:bank", Money.of("-3.20", "EUR"))])
        self.ledger = ledger

    def test_trial_balance_rows(self):
        rows = trial_balance(self.ledger)
        self.assertEqual(
            [(r.account, r.currency, str(r.debit), str(r.credit)) for r in rows],
            [
                ("assets:bank", "EUR", "0.00 EUR", "3.20 EUR"),
                ("assets:bank", "USD", "3,000.00 USD", "0.00 USD"),
                ("liabilities:card", "USD", "0.00 USD", "62.10 USD"),
                ("income:salary", "USD", "0.00 USD", "3,000.00 USD"),
                ("expenses:food", "EUR", "3.20 EUR", "0.00 EUR"),
                ("expenses:food", "USD", "62.10 USD", "0.00 USD"),
            ],
        )

    def test_totals_balance_per_currency(self):
        for bucket in totals(trial_balance(self.ledger)).values():
            self.assertEqual(bucket["debit"], bucket["credit"])

    def test_as_of(self):
        rows = trial_balance(self.ledger, as_of=date(2026, 1, 31))
        self.assertEqual({r.currency for r in rows}, {"USD"})

    def test_format(self):
        text = format_trial_balance(trial_balance(self.ledger))
        self.assertIn("Total USD", text)
        self.assertIn("3,062.10 USD", text)
        self.assertEqual(format_trial_balance([]), "(no balances)\n")

    def test_income_statement(self):
        net = income_statement(self.ledger, date(2026, 1, 1), date(2026, 1, 31))
        self.assertEqual(net, {"USD": usd("2937.90")})


if __name__ == "__main__":
    unittest.main()
