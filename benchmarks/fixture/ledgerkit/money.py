"""Fixed-point money values that always carry their currency.

Amounts are stored as ``Decimal`` and quantized to the currency's minor unit
(cents for USD, none for JPY, fils for KWD) with banker's rounding, so two
``Money`` values that print the same always compare equal.
"""

from __future__ import annotations

from dataclasses import dataclass
from decimal import ROUND_HALF_EVEN, Decimal, InvalidOperation
from typing import Iterable, List, Sequence, Union

#: Number of digits after the decimal point for each supported currency.
MINOR_UNITS = {
    "USD": 2,
    "EUR": 2,
    "GBP": 2,
    "INR": 2,
    "JPY": 0,
    "KWD": 3,
}

Number = Union[int, str, Decimal]


class CurrencyMismatch(ValueError):
    """Raised when arithmetic mixes two different currencies."""


def minor_digits(currency: str) -> int:
    """Return how many decimal places ``currency`` uses."""
    try:
        return MINOR_UNITS[currency]
    except KeyError:
        raise ValueError(f"unknown currency {currency!r}") from None


def _quantum(currency: str) -> Decimal:
    return Decimal(1).scaleb(-minor_digits(currency))


@dataclass(frozen=True)
class Money:
    """An amount in a single currency, rounded to that currency's minor unit."""

    amount: Decimal
    currency: str

    def __post_init__(self) -> None:
        currency = self.currency.upper()
        amount = self.amount
        if not isinstance(amount, Decimal):
            try:
                amount = Decimal(str(amount))
            except InvalidOperation:
                raise ValueError(f"not a number: {self.amount!r}") from None
        if not amount.is_finite():
            raise ValueError(f"amount must be finite, got {amount}")
        object.__setattr__(self, "currency", currency)
        object.__setattr__(
            self, "amount", amount.quantize(_quantum(currency), rounding=ROUND_HALF_EVEN)
        )

    # ----------------------------------------------------------------- builders
    @classmethod
    def of(cls, amount: Number, currency: str) -> "Money":
        """Build from an int, a string such as ``"12.50"``, or a ``Decimal``.

        Floats are rejected because they cannot represent most decimal amounts.
        """
        if isinstance(amount, float):
            raise TypeError("use a str or Decimal for money, not float")
        return cls(Decimal(str(amount)), currency)

    @classmethod
    def zero(cls, currency: str) -> "Money":
        return cls(Decimal(0), currency)

    @classmethod
    def from_minor(cls, units: int, currency: str) -> "Money":
        """Build from an integer count of minor units (e.g. cents)."""
        return cls(Decimal(units).scaleb(-minor_digits(currency)), currency)

    # --------------------------------------------------------------- inspection
    def minor_units(self) -> int:
        """Return the amount as an integer count of minor units."""
        return int(self.amount.scaleb(minor_digits(self.currency)))

    def is_zero(self) -> bool:
        return self.amount == 0

    def is_negative(self) -> bool:
        return self.amount < 0

    # --------------------------------------------------------------- arithmetic
    def _same_currency(self, other: object) -> "Money":
        if not isinstance(other, Money):
            raise TypeError(f"expected Money, got {type(other).__name__}")
        if other.currency != self.currency:
            raise CurrencyMismatch(f"cannot combine {self.currency} with {other.currency}")
        return other

    def __add__(self, other: "Money") -> "Money":
        other = self._same_currency(other)
        return Money(self.amount + other.amount, self.currency)

    def __sub__(self, other: "Money") -> "Money":
        other = self._same_currency(other)
        return Money(self.amount - other.amount, self.currency)

    def __neg__(self) -> "Money":
        return Money(-self.amount, self.currency)

    def __abs__(self) -> "Money":
        return Money(abs(self.amount), self.currency)

    def __mul__(self, factor: Number) -> "Money":
        if isinstance(factor, (float, Money)):
            raise TypeError("multiply money by an int, str or Decimal")
        return Money(self.amount * Decimal(str(factor)), self.currency)

    __rmul__ = __mul__

    def __lt__(self, other: "Money") -> bool:
        return self.amount < self._same_currency(other).amount

    def __le__(self, other: "Money") -> bool:
        return self.amount <= self._same_currency(other).amount

    # --------------------------------------------------------------- allocation
    def allocate(self, ratios: Sequence[int]) -> List["Money"]:
        """Split this amount into parts proportional to integer ``ratios``.

        Each part first receives the floor of its exact share, counted in minor
        units. The minor units left over by flooring (always fewer than the
        number of parts) are then handed out one at a time to the parts with a
        non-zero ratio, in the order the ratios were given, so the earliest
        parts absorb the rounding. The parts always add up to the original
        amount exactly and the sign of the amount is preserved.

        >>> [str(m) for m in Money.of("100.00", "USD").allocate([1, 1, 1])]
        ['33.34 USD', '33.33 USD', '33.33 USD']
        """
        if not ratios:
            raise ValueError("allocate needs at least one ratio")
        if any(not isinstance(r, int) or r < 0 for r in ratios):
            raise ValueError("ratios must be non-negative integers")
        total_ratio = sum(ratios)
        if total_ratio == 0:
            raise ValueError("at least one ratio must be positive")

        units = self.minor_units()
        sign = -1 if units < 0 else 1
        units = abs(units)

        shares = [units * r // total_ratio for r in ratios]
        remainder = units - sum(shares)
        receivers = [i for i, r in enumerate(ratios) if r > 0]
        for i in receivers[:remainder]:
            shares[i] += 1

        return [Money.from_minor(sign * s, self.currency) for s in shares]

    # ------------------------------------------------------------------ display
    def __str__(self) -> str:
        digits = minor_digits(self.currency)
        return f"{self.amount:,.{digits}f} {self.currency}"


def total(amounts: Iterable[Money], currency: str) -> Money:
    """Sum ``amounts`` (all in ``currency``); an empty iterable gives zero."""
    result = Money.zero(currency)
    for amount in amounts:
        result = result + amount
    return result
