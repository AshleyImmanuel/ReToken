import unittest
from datetime import date

from ledgerkit.ledger import Ledger, Posting
from ledgerkit.money import Money
from ledgerkit.recurring import Recurrence, add_months, post_recurring


class AddMonthsTests(unittest.TestCase):
    def test_simple(self):
        self.assertEqual(add_months(date(2026, 1, 15), 1), date(2026, 2, 15))
        self.assertEqual(add_months(date(2026, 11, 15), 3), date(2027, 2, 15))
        self.assertEqual(add_months(date(2026, 3, 15), -3), date(2025, 12, 15))

    def test_clamps_to_month_end(self):
        self.assertEqual(add_months(date(2026, 1, 31), 1), date(2026, 2, 28))
        self.assertEqual(add_months(date(2028, 1, 31), 1), date(2028, 2, 29))
        self.assertEqual(add_months(date(2026, 5, 31), 1), date(2026, 6, 30))


class RecurrenceTests(unittest.TestCase):
    def test_count(self):
        dates = Recurrence(date(2026, 1, 10), count=3).dates()
        self.assertEqual(dates, [date(2026, 1, 10), date(2026, 2, 10), date(2026, 3, 10)])

    def test_until_is_inclusive(self):
        dates = Recurrence(date(2026, 1, 10), every_months=3, until=date(2026, 7, 10)).dates()
        self.assertEqual(dates, [date(2026, 1, 10), date(2026, 4, 10), date(2026, 7, 10)])

    def test_month_end_does_not_drift(self):
        dates = Recurrence(date(2026, 1, 31), count=4).dates()
        self.assertEqual(
            dates, [date(2026, 1, 31), date(2026, 2, 28), date(2026, 3, 31), date(2026, 4, 30)])

    def test_validation(self):
        with self.assertRaises(ValueError):
            Recurrence(date(2026, 1, 1))
        with self.assertRaises(ValueError):
            Recurrence(date(2026, 1, 1), every_months=0, count=1)
        with self.assertRaises(ValueError):
            Recurrence(date(2026, 2, 1), until=date(2026, 1, 1))

    def test_post_recurring(self):
        ledger = Ledger()
        ledger.open_account("assets:bank", "asset")
        ledger.open_account("expenses:rent", "expense")
        rent = Money.of("1200", "USD")
        posted = post_recurring(
            ledger, Recurrence(date(2026, 1, 1), count=12), "Rent",
            [Posting("expenses:rent", rent), Posting("assets:bank", -rent)])
        self.assertEqual(len(posted), 12)
        self.assertTrue(all("recurring" in t.tags for t in posted))
        self.assertEqual(ledger.balance("expenses:rent"), {"USD": Money.of("14400", "USD")})


if __name__ == "__main__":
    unittest.main()
