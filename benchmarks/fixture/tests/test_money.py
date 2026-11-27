import unittest
from decimal import Decimal

from ledgerkit.money import CurrencyMismatch, Money, total


class MoneyTests(unittest.TestCase):
    def test_rounds_half_even_to_minor_unit(self):
        self.assertEqual(Money.of("1.005", "USD").amount, Decimal("1.00"))
        self.assertEqual(Money.of("1.015", "USD").amount, Decimal("1.02"))
        self.assertEqual(Money.of("12.5", "JPY").amount, Decimal("12"))
        self.assertEqual(Money.of("1.2345", "KWD").amount, Decimal("1.234"))

    def test_rejects_float_and_unknown_currency(self):
        with self.assertRaises(TypeError):
            Money.of(0.1, "USD")
        with self.assertRaises(ValueError):
            Money.of("1", "XYZ")

    def test_currency_is_normalised(self):
        self.assertEqual(Money.of("1", "usd").currency, "USD")

    def test_arithmetic(self):
        a, b = Money.of("10.25", "EUR"), Money.of("0.75", "EUR")
        self.assertEqual(a + b, Money.of("11", "EUR"))
        self.assertEqual(a - b, Money.of("9.50", "EUR"))
        self.assertEqual(-a, Money.of("-10.25", "EUR"))
        self.assertEqual(a * 2, Money.of("20.50", "EUR"))
        self.assertEqual(3 * b, Money.of("2.25", "EUR"))

    def test_mixing_currencies_fails(self):
        with self.assertRaises(CurrencyMismatch):
            Money.of("1", "USD") + Money.of("1", "EUR")

    def test_minor_units_round_trip(self):
        self.assertEqual(Money.of("12.34", "USD").minor_units(), 1234)
        self.assertEqual(Money.from_minor(-505, "USD"), Money.of("-5.05", "USD"))
        self.assertEqual(Money.from_minor(1500, "JPY"), Money.of("1500", "JPY"))

    def test_str(self):
        self.assertEqual(str(Money.of("1234567.8", "USD")), "1,234,567.80 USD")
        self.assertEqual(str(Money.of("-3", "JPY")), "-3 JPY")

    def test_total(self):
        self.assertEqual(total([], "USD"), Money.zero("USD"))
        amounts = [Money.of("1.10", "USD"), Money.of("2.20", "USD")]
        self.assertEqual(total(amounts, "USD"), Money.of("3.30", "USD"))


class AllocateTests(unittest.TestCase):
    def test_even_split_gives_leftover_to_first_parts(self):
        parts = Money.of("100", "USD").allocate([1, 1, 1])
        self.assertEqual([str(p) for p in parts], ["33.34 USD", "33.33 USD", "33.33 USD"])

    def test_two_cents_left_over(self):
        parts = Money.of("0.05", "USD").allocate([1, 1, 1])
        self.assertEqual([p.minor_units() for p in parts], [2, 2, 1])

    def test_weighted(self):
        parts = Money.of("10", "USD").allocate([70, 20, 10])
        self.assertEqual([str(p) for p in parts], ["7.00 USD", "2.00 USD", "1.00 USD"])

    def test_parts_always_sum_to_original(self):
        for amount in ("0.01", "1", "99.99", "1234.57", "-10"):
            for ratios in ([1], [1, 2], [3, 3, 3], [5, 0, 7, 1]):
                original = Money.of(amount, "USD")
                parts = original.allocate(ratios)
                self.assertEqual(total(parts, "USD"), original, (amount, ratios))

    def test_zero_ratio_parts_get_nothing(self):
        parts = Money.of("1.00", "USD").allocate([0, 1, 2])
        self.assertTrue(parts[0].is_zero())

    def test_negative_amounts_keep_their_sign(self):
        parts = Money.of("-1.00", "USD").allocate([1, 1, 1])
        self.assertEqual([p.minor_units() for p in parts], [-34, -33, -33])

    def test_invalid_ratios(self):
        money = Money.of("1", "USD")
        for ratios in ([], [0, 0], [1, -1], [1.5]):
            with self.assertRaises(ValueError, msg=ratios):
                money.allocate(ratios)


if __name__ == "__main__":
    unittest.main()
