"""A small double-entry ledger.

Every transaction is a list of postings. A positive posting is a debit and a
negative posting is a credit. ``Ledger.post`` refuses any transaction whose
postings do not sum to zero in every currency it touches.
"""

from __future__ import annotations

import bisect
from collections import defaultdict
from dataclasses import dataclass, field
from datetime import date
from typing import Dict, FrozenSet, Iterable, List, Optional, Sequence, Tuple

from .money import Money

#: Account kinds, in the order reports list them.
ACCOUNT_KINDS = ("asset", "liability", "equity", "income", "expense")

#: Kinds whose balance is normally a debit (positive).
DEBIT_NORMAL = frozenset({"asset", "expense"})


class LedgerError(ValueError):
    """Base class for ledger validation errors."""


class UnbalancedTransaction(LedgerError):
    """Raised when postings do not sum to zero in some currency."""


class UnknownAccount(LedgerError):
    """Raised when a posting refers to an account that was never opened."""


class AccountClosed(LedgerError):
    """Raised when posting to an account outside the dates it is open."""


@dataclass(frozen=True)
class Account:
    name: str
    kind: str
    opened: date
    closed: Optional[date] = None

    def is_open_on(self, day: date) -> bool:
        if day < self.opened:
            return False
        return self.closed is None or day <= self.closed


@dataclass(frozen=True)
class Posting:
    account: str
    amount: Money


@dataclass(frozen=True)
class Transaction:
    date: date
    description: str
    postings: Tuple[Posting, ...]
    tags: FrozenSet[str] = field(default_factory=frozenset)

    def currencies(self) -> List[str]:
        return sorted({p.amount.currency for p in self.postings})


def _validate_name(name: str) -> str:
    parts = name.split(":")
    if not name or any(not part or part != part.strip() or " " in part for part in parts):
        raise LedgerError(f"invalid account name {name!r}")
    return name


class Ledger:
    """An ordered collection of balanced transactions over opened accounts."""

    def __init__(self) -> None:
        self._accounts: Dict[str, Account] = {}
        self._transactions: List[Transaction] = []
        self._dates: List[date] = []

    # ----------------------------------------------------------------- accounts
    def open_account(self, name: str, kind: str, opened: date = date.min) -> Account:
        """Open ``name`` as an account of ``kind`` from ``opened`` onwards."""
        _validate_name(name)
        if kind not in ACCOUNT_KINDS:
            raise LedgerError(f"unknown account kind {kind!r}; expected one of {ACCOUNT_KINDS}")
        if name in self._accounts:
            raise LedgerError(f"account {name!r} is already open")
        account = Account(name=name, kind=kind, opened=opened)
        self._accounts[name] = account
        return account

    def close_account(self, name: str, closed: date) -> Account:
        """Close ``name`` after ``closed``; its balance on that day must be zero."""
        account = self.account(name)
        leftover = [m for m in self.balance(name, as_of=closed).values() if not m.is_zero()]
        if leftover:
            shown = ", ".join(str(m) for m in leftover)
            raise LedgerError(f"cannot close {name!r} with a non-zero balance ({shown})")
        if any(t.date > closed for t in self.transactions(account=name)):
            raise LedgerError(f"{name!r} has postings after {closed.isoformat()}")
        updated = Account(account.name, account.kind, account.opened, closed)
        self._accounts[name] = updated
        return updated

    def account(self, name: str) -> Account:
        try:
            return self._accounts[name]
        except KeyError:
            raise UnknownAccount(f"unknown account {name!r}") from None

    def accounts(self) -> List[Account]:
        """All accounts, ordered by kind (as in ``ACCOUNT_KINDS``) then name."""
        order = {kind: i for i, kind in enumerate(ACCOUNT_KINDS)}
        return sorted(self._accounts.values(), key=lambda a: (order[a.kind], a.name))

    # ------------------------------------------------------------- transactions
    def post(
        self,
        day: date,
        description: str,
        postings: Sequence[Posting],
        tags: Iterable[str] = (),
    ) -> Transaction:
        """Validate and record a transaction.

        Rules enforced:

        * at least two postings, none of them zero;
        * every account exists and is open on ``day``;
        * postings sum to zero separately in each currency.

        Transactions are kept sorted by date; same-day transactions keep the
        order they were posted in.
        """
        postings = tuple(postings)
        if len(postings) < 2:
            raise UnbalancedTransaction("a transaction needs at least two postings")
        sums: Dict[str, Money] = {}
        for posting in postings:
            account = self.account(posting.account)
            if not account.is_open_on(day):
                raise AccountClosed(f"{account.name!r} is not open on {day.isoformat()}")
            if posting.amount.is_zero():
                raise LedgerError(f"zero posting to {posting.account!r}")
            currency = posting.amount.currency
            sums[currency] = sums.get(currency, Money.zero(currency)) + posting.amount
        unbalanced = [m for m in sums.values() if not m.is_zero()]
        if unbalanced:
            shown = ", ".join(str(m) for m in unbalanced)
            raise UnbalancedTransaction(f"{description!r} does not balance: off by {shown}")

        transaction = Transaction(
            date=day,
            description=description.strip(),
            postings=postings,
            tags=frozenset(t.lower() for t in tags),
        )
        index = bisect.bisect_right(self._dates, day)
        self._dates.insert(index, day)
        self._transactions.insert(index, transaction)
        return transaction

    def transactions(
        self,
        account: Optional[str] = None,
        start: Optional[date] = None,
        end: Optional[date] = None,
        tag: Optional[str] = None,
    ) -> List[Transaction]:
        """Transactions in date order, optionally filtered (``end`` is inclusive)."""
        if account is not None:
            self.account(account)
        lo = 0 if start is None else bisect.bisect_left(self._dates, start)
        hi = len(self._dates) if end is None else bisect.bisect_right(self._dates, end)
        selected = self._transactions[lo:hi]
        if account is not None:
            selected = [t for t in selected if any(p.account == account for p in t.postings)]
        if tag is not None:
            selected = [t for t in selected if tag.lower() in t.tags]
        return selected

    # ----------------------------------------------------------------- balances
    def balance(self, name: str, as_of: Optional[date] = None) -> Dict[str, Money]:
        """Balance of one account per currency, including ``as_of`` if given.

        Sub-accounts are included: ``balance("expenses")`` covers
        ``expenses:food`` and ``expenses:rent``.
        """
        if name not in self._accounts and not any(
            a.startswith(name + ":") for a in self._accounts
        ):
            raise UnknownAccount(f"unknown account {name!r}")
        result: Dict[str, Money] = {}
        for transaction in self.transactions(end=as_of):
            for posting in transaction.postings:
                if posting.account == name or posting.account.startswith(name + ":"):
                    currency = posting.amount.currency
                    result[currency] = result.get(currency, Money.zero(currency)) + posting.amount
        return dict(sorted(result.items()))

    def balances(self, as_of: Optional[date] = None) -> Dict[str, Dict[str, Money]]:
        """Balances of every opened account (exact name, no roll-up), per currency."""
        result: Dict[str, Dict[str, Money]] = defaultdict(dict)
        for transaction in self.transactions(end=as_of):
            for posting in transaction.postings:
                per_currency = result[posting.account]
                currency = posting.amount.currency
                per_currency[currency] = (
                    per_currency.get(currency, Money.zero(currency)) + posting.amount
                )
        return {name: dict(sorted(result[name].items())) for name in sorted(result)}

    def __len__(self) -> int:
        return len(self._transactions)
