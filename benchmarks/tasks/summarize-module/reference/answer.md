`ledgerkit/ledger.py` is a small double-entry ledger.

Public classes: `Account` (name, kind, opened/closed dates), `Posting`
(account + `Money`; positive is a debit, negative a credit), `Transaction`
(date, description, postings, tags), `Ledger`, and the errors `LedgerError`,
`UnbalancedTransaction`, `UnknownAccount` and `AccountClosed`.

Main `Ledger` methods: `open_account`, `close_account` (only at a zero
balance), `account`, `accounts`, `post`, `transactions` (filters by account,
dates and tag), `balance` (one account plus sub-accounts, per currency) and
`balances` (every account).

`Ledger.post` enforces:

* at least two postings;
* no zero postings;
* every account exists (`UnknownAccount`) and is open on the transaction
  date (`AccountClosed`);
* postings sum to zero separately in each currency (`UnbalancedTransaction`).

Recorded transactions stay sorted by date, same-day ones in posting order.
