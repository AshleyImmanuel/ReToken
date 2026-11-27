"""Reports over a ``Ledger``: trial balance and income statement."""

from __future__ import annotations

from dataclasses import dataclass
from datetime import date
from typing import Dict, List, Optional

from .fx import RateTable
from .ledger import Ledger
from .money import Money, total


@dataclass(frozen=True)
class TrialBalanceRow:
    account: str
    kind: str
    currency: str
    debit: Money
    credit: Money

    @property
    def net(self) -> Money:
        return self.debit - self.credit


def _row(account: str, kind: str, amount: Money) -> TrialBalanceRow:
    zero = Money.zero(amount.currency)
    if amount.is_negative():
        return TrialBalanceRow(account, kind, amount.currency, zero, -amount)
    return TrialBalanceRow(account, kind, amount.currency, amount, zero)


def trial_balance(
    ledger: Ledger,
    as_of: Optional[date] = None,
    base_currency: Optional[str] = None,
    rates: Optional[RateTable] = None,
) -> List[TrialBalanceRow]:
    """One row per account and currency with a non-zero balance.

    A positive balance is shown as a debit, a negative one as a credit. Rows
    follow ``Ledger.accounts()`` order, then currency code. With
    ``base_currency`` every balance is converted with ``rates`` and each
    account gets a single row in the base currency.
    """
    if base_currency is not None and rates is None:
        raise ValueError("base_currency needs a RateTable")
    balances = ledger.balances(as_of=as_of)
    rows: List[TrialBalanceRow] = []
    for account in ledger.accounts():
        per_currency = balances.get(account.name, {})
        if base_currency is not None:
            converted = total(
                (rates.convert(m, base_currency) for m in per_currency.values()),
                base_currency.upper(),
            )
            if not converted.is_zero():
                rows.append(_row(account.name, account.kind, converted))
            continue
        for amount in per_currency.values():
            if not amount.is_zero():
                rows.append(_row(account.name, account.kind, amount))
    return rows


def totals(rows: List[TrialBalanceRow]) -> Dict[str, Dict[str, Money]]:
    """Debit and credit totals per currency: ``{"USD": {"debit": ..., "credit": ...}}``."""
    result: Dict[str, Dict[str, Money]] = {}
    for row in rows:
        bucket = result.setdefault(
            row.currency,
            {"debit": Money.zero(row.currency), "credit": Money.zero(row.currency)},
        )
        bucket["debit"] = bucket["debit"] + row.debit
        bucket["credit"] = bucket["credit"] + row.credit
    return dict(sorted(result.items()))


def format_trial_balance(rows: List[TrialBalanceRow]) -> str:
    """Render rows as an aligned text table with per-currency totals."""
    if not rows:
        return "(no balances)\n"
    width = max(len("Account"), *(len(r.account) for r in rows))

    def amount(m: Money) -> str:
        return "" if m.is_zero() else str(m)

    lines = [f"{'Account':<{width}}  {'Debit':>16}  {'Credit':>16}"]
    lines.append("-" * len(lines[0]))
    for row in rows:
        lines.append(f"{row.account:<{width}}  {amount(row.debit):>16}  {amount(row.credit):>16}")
    lines.append("-" * len(lines[0]))
    for currency, bucket in totals(rows).items():
        label = f"Total {currency}"
        lines.append(f"{label:<{width}}  {str(bucket['debit']):>16}  {str(bucket['credit']):>16}")
    return "\n".join(lines) + "\n"


def income_statement(ledger: Ledger, start: date, end: date) -> Dict[str, Money]:
    """Net income per currency between ``start`` and ``end`` inclusive.

    Income postings are credits (negative), so net income is
    ``-(income + expenses)``: positive means a profit.
    """
    kinds = {a.name: a.kind for a in ledger.accounts()}
    result: Dict[str, Money] = {}
    for transaction in ledger.transactions(start=start, end=end):
        for posting in transaction.postings:
            if kinds[posting.account] in ("income", "expense"):
                currency = posting.amount.currency
                result[currency] = result.get(currency, Money.zero(currency)) - posting.amount
    return dict(sorted(result.items()))
