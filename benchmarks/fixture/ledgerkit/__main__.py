"""Command line: ``python -m ledgerkit <command> <journal> [options]``.

Commands:

* ``tb``      trial balance (``--as-of YYYY-MM-DD``)
* ``balance`` balance of one account and its sub-accounts
* ``income``  net income between ``--start`` and ``--end``
"""

from __future__ import annotations

import argparse
import sys
from datetime import date
from pathlib import Path
from typing import List, Optional

from .parser import ParseError, load_ledger
from .report import format_trial_balance, income_statement, trial_balance


def _date(text: str) -> date:
    try:
        return date.fromisoformat(text)
    except ValueError:
        raise argparse.ArgumentTypeError(f"not a YYYY-MM-DD date: {text!r}") from None


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(prog="ledgerkit")
    commands = parser.add_subparsers(dest="command", required=True)

    tb = commands.add_parser("tb", help="trial balance")
    tb.add_argument("journal", type=Path)
    tb.add_argument("--as-of", type=_date)

    balance = commands.add_parser("balance", help="balance of one account")
    balance.add_argument("journal", type=Path)
    balance.add_argument("account")
    balance.add_argument("--as-of", type=_date)

    income = commands.add_parser("income", help="net income for a period")
    income.add_argument("journal", type=Path)
    income.add_argument("--start", type=_date, required=True)
    income.add_argument("--end", type=_date, required=True)
    return parser


def main(argv: Optional[List[str]] = None) -> int:
    args = build_parser().parse_args(argv)
    try:
        ledger = load_ledger(args.journal.read_text(encoding="utf-8"))
    except OSError as exc:
        print(f"ledgerkit: cannot read {args.journal}: {exc.strerror}", file=sys.stderr)
        return 2
    except ParseError as exc:
        print(f"ledgerkit: {args.journal}: {exc}", file=sys.stderr)
        return 1

    if args.command == "tb":
        sys.stdout.write(format_trial_balance(trial_balance(ledger, as_of=args.as_of)))
    elif args.command == "balance":
        amounts = ledger.balance(args.account, as_of=args.as_of)
        for money in amounts.values():
            print(money)
        if not amounts:
            print("0")
    elif args.command == "income":
        for money in income_statement(ledger, args.start, args.end).values():
            print(money)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
