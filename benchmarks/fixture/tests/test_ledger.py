import unittest
from datetime import date

from ledgerkit.ledger import (
    AccountClosed,
    Ledger,
    LedgerError,
    Posting,
    UnbalancedTransaction,
    UnknownAccount,
)
from ledgerkit.money import Money


def usd(amount):
    return Money.of(amount, "USD")


def make_ledger():
    ledger = Ledger()
    ledger.open_account("assets:bank", "asset", date(2026, 1, 1))
    ledger.open_account("assets:cash", "asset", date(2026, 1, 1))
    ledger.open_account("income:salary", "income", date(2026, 1, 1))
    ledger.open_account("expenses:food", "expense", date(2026, 1, 1))
    ledger.open_account("expenses:rent", "expense", date(2026, 1, 1))
    return ledger


class AccountTests(unittest.TestCase):
    def test_accounts_are_ordered_by_kind_then_name(self):
        names = [a.name for a in make_ledger().accounts()]
        self.assertEqual(
            names,
            ["assets:bank", "assets:cash", "income:salary", "expenses:food", "expenses:rent"],
        )

    def test_rejects_bad_kind_duplicate_and_bad_name(self):
        ledger = make_ledger()
        with self.assertRaises(LedgerError):
            ledger.open_account("x", "savings")
        with self.assertRaises(LedgerError):
            ledger.open_account("assets:bank", "asset")
        for bad in ("", "assets:", ":bank", "assets:my bank"):
            with self.assertRaises(LedgerError, msg=bad):
                ledger.open_account(bad, "asset")

    def test_close_requires_zero_balance(self):
        ledger = make_ledger()
        ledger.post(date(2026, 1, 2), "Withdraw", [
            Posting("assets:cash", usd("20")), Posting("assets:bank", usd("-20"))])
        with self.assertRaises(LedgerError):
            ledger.close_account("assets:cash", date(2026, 1, 3))
        ledger.post(date(2026, 1, 3), "Lunch", [
            Posting("expenses:food", usd("20")), Posting("assets:cash", usd("-20"))])
        ledger.close_account("assets:cash", date(2026, 1, 3))
        with self.assertRaises(AccountClosed):
            ledger.post(date(2026, 1, 4), "Late", [
                Posting("assets:cash", usd("1")), Posting("assets:bank", usd("-1"))])


class PostTests(unittest.TestCase):
    def test_balanced_transaction_is_recorded(self):
        ledger = make_ledger()
        t = ledger.post(date(2026, 1, 5), " Salary ", [
            Posting("assets:bank", usd("3000")), Posting("income:salary", usd("-3000"))],
            tags=["Payroll"])
        self.assertEqual(t.description, "Salary")
        self.assertEqual(t.tags, frozenset({"payroll"}))
        self.assertEqual(len(ledger), 1)

    def test_unbalanced_is_rejected(self):
        ledger = make_ledger()
        with self.assertRaises(UnbalancedTransaction):
            ledger.post(date(2026, 1, 5), "Oops", [
                Posting("assets:bank", usd("10")), Posting("income:salary", usd("-9.99"))])

    def test_each_currency_must_balance_separately(self):
        ledger = make_ledger()
        with self.assertRaises(UnbalancedTransaction):
            ledger.post(date(2026, 1, 5), "Mixed", [
                Posting("assets:bank", usd("10")),
                Posting("income:salary", Money.of("-10", "EUR"))])

    def test_needs_two_postings_and_no_zero_postings(self):
        ledger = make_ledger()
        with self.assertRaises(UnbalancedTransaction):
            ledger.post(date(2026, 1, 5), "One", [Posting("assets:bank", usd("0"))])
        with self.assertRaises(LedgerError):
            ledger.post(date(2026, 1, 5), "Zero", [
                Posting("assets:bank", usd("0")), Posting("income:salary", usd("0"))])

    def test_unknown_or_unopened_account(self):
        ledger = make_ledger()
        with self.assertRaises(UnknownAccount):
            ledger.post(date(2026, 1, 5), "?", [
                Posting("assets:nowhere", usd("1")), Posting("assets:bank", usd("-1"))])
        with self.assertRaises(AccountClosed):
            ledger.post(date(2025, 12, 31), "Early", [
                Posting("assets:cash", usd("1")), Posting("assets:bank", usd("-1"))])

    def test_transactions_stay_in_date_order(self):
        ledger = make_ledger()
        for day, text in [(10, "b"), (3, "a"), (10, "c"), (1, "first")]:
            ledger.post(date(2026, 1, day), text, [
                Posting("expenses:food", usd("1")), Posting("assets:cash", usd("-1"))])
        self.assertEqual([t.description for t in ledger.transactions()], ["first", "a", "b", "c"])


class BalanceTests(unittest.TestCase):
    def setUp(self):
        self.ledger = make_ledger()
        self.ledger.post(date(2026, 1, 5), "Salary", [
            Posting("assets:bank", usd("3000")), Posting("income:salary", usd("-3000"))])
        self.ledger.post(date(2026, 1, 6), "Rent", [
            Posting("expenses:rent", usd("1200")), Posting("assets:bank", usd("-1200"))])
        self.ledger.post(date(2026, 2, 2), "Groceries", [
            Posting("expenses:food", usd("85.40")), Posting("assets:bank", usd("-85.40"))],
            tags=["food"])

    def test_balance_and_as_of(self):
        self.assertEqual(self.ledger.balance("assets:bank"), {"USD": usd("1714.60")})
        self.assertEqual(
            self.ledger.balance("assets:bank", as_of=date(2026, 1, 31)), {"USD": usd("1800")})

    def test_balance_rolls_up_sub_accounts(self):
        self.assertEqual(self.ledger.balance("expenses"), {"USD": usd("1285.40")})

    def test_balance_of_unknown_account(self):
        with self.assertRaises(UnknownAccount):
            self.ledger.balance("liabilities")

    def test_balances_cover_every_account_with_postings(self):
        balances = self.ledger.balances()
        self.assertEqual(sorted(balances), [
            "assets:bank", "expenses:food", "expenses:rent", "income:salary"])
        self.assertEqual(balances["income:salary"], {"USD": usd("-3000")})

    def test_filters(self):
        self.assertEqual(len(self.ledger.transactions(account="expenses:food")), 1)
        self.assertEqual(len(self.ledger.transactions(start=date(2026, 2, 1))), 1)
        self.assertEqual(len(self.ledger.transactions(end=date(2026, 1, 5))), 1)
        self.assertEqual(len(self.ledger.transactions(tag="FOOD")), 1)


if __name__ == "__main__":
    unittest.main()
