"""Parser for the plain-text journal format.

Example::

    ; comments start with a semicolon
    open 2026-01-01 assets:bank asset
    open 2026-01-01 income:salary income

    2026-01-05 January salary #payroll
      assets:bank      3000.00 USD
      income:salary   -3000.00 USD

    2026-01-07 Coffee
      expenses:food       4.50 USD
      assets:bank

A transaction header is a date, a description and optional ``#tags``. Each
indented line is a posting: an account and an amount with its currency. At
most one posting may leave the amount out; it receives whatever balances the
transaction (only when the other postings share one currency).
"""

from __future__ import annotations

import re
from dataclasses import dataclass, field
from datetime import date
from decimal import Decimal, InvalidOperation
from typing import List, Optional, Tuple

from .ledger import Ledger, LedgerError, Posting
from .money import Money, minor_digits

_DATE = r"\d{4}-\d{2}-\d{2}"
_OPEN_RE = re.compile(rf"^open\s+({_DATE})\s+(\S+)\s+(\w+)\s*$")
_HEADER_RE = re.compile(rf"^({_DATE})\s+(.*?)\s*$")
_POSTING_RE = re.compile(r"^\s+(\S+)(?:\s+(-?[\d,]+(?:\.\d+)?)\s+([A-Za-z]{3}))?\s*$")
_TAG_RE = re.compile(r"(?:^|\s)#([\w-]+)")
_RATE_RE = re.compile(r"^rate(?:\s|$)")


class ParseError(ValueError):
    """A journal syntax or validation error, with its 1-based line number."""

    def __init__(self, line: int, message: str) -> None:
        super().__init__(f"line {line}: {message}")
        self.line = line
        self.message = message


@dataclass
class RawTransaction:
    line: int
    date: date
    description: str
    tags: List[str]
    postings: List[Tuple[str, Optional[Money]]] = field(default_factory=list)


@dataclass
class ParsedJournal:
    accounts: List[Tuple[date, str, str]] = field(default_factory=list)
    transactions: List[RawTransaction] = field(default_factory=list)
    rates: List[Tuple[str, str, Decimal]] = field(default_factory=list)


def _parse_rate(line: str, number: int) -> Tuple[str, str, Decimal]:
    fields = line.split()
    if len(fields) != 4:
        raise ParseError(number, "rate needs: rate <BASE> <QUOTE> <RATE>")
    _, base, quote, text = fields
    base, quote = base.upper(), quote.upper()
    for currency in (base, quote):
        try:
            minor_digits(currency)
        except ValueError as exc:
            raise ParseError(number, str(exc)) from None
    try:
        rate = Decimal(text)
    except InvalidOperation:
        raise ParseError(number, f"invalid rate {text!r}") from None
    if not rate.is_finite() or rate <= 0:
        raise ParseError(number, f"rate must be a positive number, got {text!r}")
    return base, quote, rate


def _parse_date(text: str, line: int) -> date:
    try:
        return date.fromisoformat(text)
    except ValueError:
        raise ParseError(line, f"invalid date {text!r}") from None


def _parse_amount(text: str, currency: str, line: int) -> Money:
    try:
        return Money.of(Decimal(text.replace(",", "")), currency.upper())
    except (InvalidOperation, ValueError) as exc:
        raise ParseError(line, f"invalid amount {text} {currency}: {exc}") from None


def _strip_comment(raw: str) -> str:
    index = raw.find(";")
    return raw if index < 0 else raw[:index]


def parse_journal(text: str) -> ParsedJournal:
    """Parse journal text into accounts and raw transactions (no validation)."""
    journal = ParsedJournal()
    current: Optional[RawTransaction] = None

    for number, raw in enumerate(text.splitlines(), start=1):
        line = _strip_comment(raw).rstrip()
        if not line.strip():
            current = None
            continue

        if line[0].isspace():
            if current is None:
                raise ParseError(number, "posting outside a transaction")
            match = _POSTING_RE.match(line)
            if not match:
                raise ParseError(number, f"cannot read posting {line.strip()!r}")
            account, amount, currency = match.groups()
            money = _parse_amount(amount, currency, number) if amount else None
            current.postings.append((account, money))
            continue

        if _RATE_RE.match(line):
            journal.rates.append(_parse_rate(line, number))
            current = None
            continue

        match = _OPEN_RE.match(line)
        if match:
            opened, account, kind = match.groups()
            journal.accounts.append((_parse_date(opened, number), account, kind))
            current = None
            continue

        match = _HEADER_RE.match(line)
        if match:
            day = _parse_date(match.group(1), number)
            rest = match.group(2)
            tags = _TAG_RE.findall(rest)
            description = _TAG_RE.sub("", rest).strip()
            if not description:
                raise ParseError(number, "transaction needs a description")
            current = RawTransaction(number, day, description, tags)
            journal.transactions.append(current)
            continue

        raise ParseError(number, f"unrecognised line {line.strip()!r}")

    return journal


def _fill_elided(raw: RawTransaction) -> List[Posting]:
    missing = [i for i, (_, money) in enumerate(raw.postings) if money is None]
    if len(missing) > 1:
        raise ParseError(raw.line, "only one posting may leave out its amount")
    known = [money for _, money in raw.postings if money is not None]
    postings = []
    for account, money in raw.postings:
        if money is None:
            currencies = {m.currency for m in known}
            if len(currencies) != 1:
                raise ParseError(
                    raw.line, "an elided amount needs the other postings in one currency"
                )
            (currency,) = currencies
            balance = Money.zero(currency)
            for m in known:
                balance = balance + m
            money = -balance
        postings.append(Posting(account, money))
    return postings


def load_ledger(text: str) -> Ledger:
    """Parse ``text`` and post everything into a new ``Ledger``."""
    journal = parse_journal(text)
    ledger = Ledger()
    for opened, account, kind in journal.accounts:
        try:
            ledger.open_account(account, kind, opened)
        except LedgerError as exc:
            raise ParseError(0, str(exc)) from None
    for raw in journal.transactions:
        postings = _fill_elided(raw)
        try:
            ledger.post(raw.date, raw.description, postings, raw.tags)
        except LedgerError as exc:
            raise ParseError(raw.line, str(exc)) from None
    return ledger
