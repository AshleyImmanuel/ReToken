"""Exchange rates between currencies.

A rate ``(base, quote) -> r`` means one unit of ``base`` buys ``r`` units of
``quote``. The inverse direction is derived automatically. Rates are never
chained through a third currency: a missing pair is an error, not a guess.
"""

from __future__ import annotations

from decimal import Decimal
from typing import Dict, Iterator, Tuple

from .money import Money, minor_digits


class MissingRate(LookupError):
    """Raised when no rate is known between two currencies."""


class RateTable:
    """A set of exchange rates, each usable in both directions."""

    def __init__(self) -> None:
        self._rates: Dict[Tuple[str, str], Decimal] = {}

    def set(self, base: str, quote: str, rate: Decimal) -> None:
        """Record that one ``base`` buys ``rate`` units of ``quote``."""
        base, quote = base.upper(), quote.upper()
        minor_digits(base)
        minor_digits(quote)
        if base == quote:
            raise ValueError("a rate needs two different currencies")
        rate = Decimal(rate)
        if rate <= 0:
            raise ValueError(f"rate must be positive, got {rate}")
        self._rates[(base, quote)] = rate
        self._rates.pop((quote, base), None)

    def rate(self, base: str, quote: str) -> Decimal:
        """Units of ``quote`` per one ``base``."""
        base, quote = base.upper(), quote.upper()
        if base == quote:
            return Decimal(1)
        if (base, quote) in self._rates:
            return self._rates[(base, quote)]
        if (quote, base) in self._rates:
            return Decimal(1) / self._rates[(quote, base)]
        raise MissingRate(f"no exchange rate between {base} and {quote}")

    def convert(self, money: Money, to_currency: str) -> Money:
        """Convert ``money``, rounding half-even to the target's minor unit."""
        return Money(money.amount * self.rate(money.currency, to_currency), to_currency)

    def pairs(self) -> Iterator[Tuple[str, str, Decimal]]:
        for (base, quote), rate in sorted(self._rates.items()):
            yield base, quote, rate

    def __len__(self) -> int:
        return len(self._rates)
