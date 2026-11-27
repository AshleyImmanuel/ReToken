import unittest
from decimal import Decimal

from ledgerkit.fx import MissingRate, RateTable
from ledgerkit.money import Money


class RateTableTests(unittest.TestCase):
    def setUp(self):
        self.rates = RateTable()
        self.rates.set("EUR", "USD", Decimal("1.0850"))
        self.rates.set("usd", "jpy", Decimal("151.20"))

    def test_direct_and_inverse(self):
        self.assertEqual(self.rates.rate("EUR", "USD"), Decimal("1.0850"))
        self.assertEqual(self.rates.rate("USD", "USD"), Decimal(1))
        self.assertAlmostEqual(self.rates.rate("USD", "EUR"), Decimal(1) / Decimal("1.0850"))

    def test_no_triangulation(self):
        with self.assertRaises(MissingRate):
            self.rates.rate("EUR", "JPY")

    def test_convert_rounds_half_even(self):
        self.assertEqual(self.rates.convert(Money.of("100", "EUR"), "USD"), Money.of("108.50", "USD"))
        self.assertEqual(self.rates.convert(Money.of("1.00", "USD"), "JPY"), Money.of("151", "JPY"))
        self.assertEqual(self.rates.convert(Money.of("10.00", "USD"), "EUR"), Money.of("9.22", "EUR"))

    def test_setting_a_pair_replaces_its_inverse(self):
        self.rates.set("USD", "EUR", Decimal("0.9"))
        self.assertEqual(self.rates.rate("EUR", "USD"), Decimal(1) / Decimal("0.9"))
        self.assertEqual(len(self.rates), 2)

    def test_rejects_bad_rates(self):
        with self.assertRaises(ValueError):
            self.rates.set("USD", "USD", Decimal(1))
        with self.assertRaises(ValueError):
            self.rates.set("USD", "GBP", Decimal(0))
        with self.assertRaises(ValueError):
            self.rates.set("USD", "XYZ", Decimal(2))

    def test_pairs_are_sorted(self):
        self.assertEqual([(b, q) for b, q, _ in self.rates.pairs()], [("EUR", "USD"), ("USD", "JPY")])


if __name__ == "__main__":
    unittest.main()
