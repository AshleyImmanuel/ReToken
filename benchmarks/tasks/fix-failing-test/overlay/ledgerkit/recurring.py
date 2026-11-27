"""Recurring transactions (rent, subscriptions, salaries).

Dates are always computed from the original start date, never from the
previous occurrence, so a schedule that starts on the 31st returns to the 31st
in every month that has one instead of drifting to the 28th after February.
"""

from __future__ import annotations

import calendar
from dataclasses import dataclass
from datetime import date
from typing import List, Optional, Sequence

from .ledger import Ledger, Posting, Transaction


def add_months(day: date, months: int) -> date:
    """Move ``day`` by ``months``, clamping to the last day of short months."""
    index = day.year * 12 + (day.month - 1) + months
    year, month = divmod(index, 12)
    month += 1
    last = calendar.monthrange(year, month)[1]
    return date(year, month, min(day.day, last))


@dataclass(frozen=True)
class Recurrence:
    """Every ``every_months`` months from ``start``, bounded by ``count`` or ``until``."""

    start: date
    every_months: int = 1
    count: Optional[int] = None
    until: Optional[date] = None

    def __post_init__(self) -> None:
        if self.every_months < 1:
            raise ValueError("every_months must be at least 1")
        if self.count is None and self.until is None:
            raise ValueError("a recurrence needs a count or an until date")
        if self.count is not None and self.count < 1:
            raise ValueError("count must be at least 1")
        if self.until is not None and self.until < self.start:
            raise ValueError("until is before start")

    def dates(self) -> List[date]:
        result: List[date] = []
        day = self.start
        while True:
            if self.until is not None and day > self.until:
                break
            result.append(day)
            if self.count is not None and len(result) >= self.count:
                break
            day = add_months(day, self.every_months)
        return result


def post_recurring(
    ledger: Ledger,
    recurrence: Recurrence,
    description: str,
    postings: Sequence[Posting],
    tags: Sequence[str] = (),
) -> List[Transaction]:
    """Post one copy of ``postings`` on every date of ``recurrence``."""
    return [
        ledger.post(day, description, postings, tags=tuple(tags) + ("recurring",))
        for day in recurrence.dates()
    ]
