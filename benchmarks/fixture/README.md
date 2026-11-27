# ledgerkit

A tiny double-entry bookkeeping library: fixed-point money, a ledger that
rejects unbalanced transactions, exchange rates, a plain-text journal format,
recurring entries and a small CLI.

```bash
python -m unittest discover -s tests -t .     # run the tests (Python 3.9+, no dependencies)
python -m ledgerkit tb examples/household.journal
```

| Module | What it does |
|---|---|
| `ledgerkit/money.py` | `Money`: Decimal amount + currency, minor-unit rounding, allocation |
| `ledgerkit/ledger.py` | `Ledger`: accounts, balanced postings, balances |
| `ledgerkit/fx.py` | `RateTable`: exchange rates and conversion |
| `ledgerkit/parser.py` | Journal text format to `Ledger` |
| `ledgerkit/report.py` | Trial balance and income statement |
| `ledgerkit/recurring.py` | Monthly schedules with month-end clamping |
| `ledgerkit/__main__.py` | `python -m ledgerkit` command line |
