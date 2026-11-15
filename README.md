# payments-engine

A toy payments engine. It reads a CSV of transactions (deposits, withdrawals,
disputes, resolves, chargebacks), applies them to client accounts and prints the
final state of every account as CSV.

## Usage

```sh
cargo run -- transactions.csv > accounts.csv
```

The input file is the only argument; the accounts are written to stdout.
Builds with stable Rust (edition 2024).

```sh
cargo run --features verbose -- transactions.csv   # log every row and its outcome to stderr
cargo test                                          # run all tests
```

## Design

| Module | Role |
|---|---|
| `types` | `Amount`, the transaction types (`Tx`, `TxOp`, `Balance`, `Dispute`), id aliases |
| `input` | CSV reader over any byte source, row parsing, skipping malformed rows |
| `engine` | accounts and stored transactions; applies one `Tx` at a time |
| `output` | writes the accounts as CSV to any writer |

The engine knows nothing about CSV or I/O: it takes parsed transactions and
returns `Ok` or the reason a transaction was rejected. Parsing and output are
separate modules on either side of it (see [Streaming](#streaming-and-concurrency)).

**Accounts** are an enum, `Normal` or `Locked`, so every operation has to say
what it does with a locked account; the compiler does not let one be forgotten.
Each account keeps `total`, a running `held` sum and the deposits currently under
dispute; `available` is derived as `total - held`, so the three can never drift
apart.

**Amounts** wrap `rust_decimal::Decimal`, never `f64`. Parsing is strict and lives
in `Amount::from_str`:

- only digits and a decimal point (no sign, exponent, separators): `-5`, `1e3` and `1_000` are rejected;
- greater than zero, at most 4 decimal places;
- at most 10^15 (`MAX_AMOUNT`).

Fields are deserialized as `&str` on purpose. csv's serde deserializer otherwise
guesses the field type, so a decimal would pass through `f64` and lose precision
(`1234567890123.4567` became `…4568`).

The cap makes overflow impossible. A repeated transaction id is ignored, so at
most 2^32 transactions ever change a balance, and no balance can exceed
2^32 × 10^15 ≈ 4.3e24, far below `Decimal::MAX` ≈ 7.9e28. Balance arithmetic
therefore uses plain operators instead of `checked_*`. The bound relies on
transaction ids never being reused; if that ever broke, `Decimal` panics on
overflow instead of wrapping, so a balance is never silently wrong. The details
are in the `MAX_AMOUNT` doc comment.

Output always has 4 decimal places (`1.5000`) and a header row, even when there
are no accounts. Rows are in no particular order.

## Error handling

| Situation | Behaviour |
|---|---|
| Malformed row (unknown type, missing or invalid field, invalid amount) | skipped; processing continues |
| Valid row the engine rejects (insufficient funds, unknown tx, wrong client, not under dispute, locked account) | ignored, as the spec says these are partner errors; `Engine::process` returns the reason as a `TxError` |
| I/O error while reading | fatal: non-zero exit and no output, because balances from part of the input would be wrong |
| Missing argument | usage message on stderr, exit code 2 |

Skipped and rejected rows are silent by default: output is checked by automated
scoring, so stderr stays quiet. Build with `--features verbose` to log every
skipped row with its line number, and every transaction with its result.

## Assumptions

Where the spec is silent, I chose what a bank would do:

- **Only deposits can be disputed.** In real banking, disputes are more often
  raised against debits (card payments, withdrawals). But the spec defines a
  dispute as moving funds from available to held, which only makes sense for
  money that came in: a withdrawal's money has already left, so there is nothing
  to hold. Disputes of withdrawals are ignored.
- **A dispute, resolve or chargeback must come from the client who owns the
  transaction**; otherwise it is ignored.
- **Disputing a transaction that is already under dispute changes nothing.** A
  resolved transaction can be disputed again.
- **A dispute can make `available` negative** when the deposit was already
  withdrawn: the full disputed amount is held anyway. A chargeback then leaves
  `total` negative, which is the fraud case from the spec (deposit, withdraw,
  reverse the deposit): the client owes that money.
- **A locked account rejects everything**: deposits, withdrawals, disputes,
  resolves and chargebacks. A chargeback means fraud is likely, and a frozen
  account goes to manual review; the engine does not keep settling it on its
  own. So a dispute still open when the account locked keeps its funds held,
  which is the cautious outcome, until someone reviews the account. Letting the
  partner still resolve or charge back disputes that were already open would be
  a reasonable alternative.
- **A withdrawal by an unknown client is rejected** and does not create an
  account.
- **A transaction id that was already applied is ignored**, so replaying input
  is idempotent. Only applied transactions claim their id; a rejected one (say, a
  withdrawal with insufficient funds) does not.
- **Columns are positional**: `type, client, tx, amount`, after a header row.
  Whitespace around fields is trimmed. The amount column may be missing or empty
  on dispute, resolve and chargeback rows.

## Streaming and concurrency

The input is never loaded whole. The pieces are split so the engine can be fed
by any stream, not only a file:

- `input::reader<R: Read>` sets up the CSV parsing rules over any byte source
  (a file, stdin, a TCP socket). `input::file_reader` is just that, applied to a file.
- `input::read_transactions` is a lazy iterator: one row is parsed, applied and
  dropped before the next is read.
- `Engine::process(&Tx)` takes one parsed transaction and has no I/O of its own.
- `output::write_accounts<W: Write>` writes to any writer.

With thousands of concurrent TCP streams, each connection gets its own reader,
and parsed transactions are routed to worker threads by client id
(`client % workers`), each worker owning the accounts of its clients. That works
without locks because a dispute only ever refers to a deposit of the same
client, so every worker has all the state it needs. Rows from one stream for one
client go through one channel to one worker, so they stay in order. The only
state shared across workers is transaction id uniqueness, which a shared atomic
bitmap would cover (see below). The csv crate is blocking, so in an async server
parsing would run on a blocking thread pool (or use `csv-async`), with bounded
channels for backpressure.

## Memory

Accounts are small: at most 65,536 of them. What grows is the transaction store:
every applied deposit and withdrawal is kept, so later disputes can be checked
and duplicate ids detected. Measured on 5 million rows, peak memory was 489 MB,
about 100 bytes per stored transaction. With the full u32 id range that would be
hundreds of GB.

An improvement I left out as over-engineering for this exercise: withdrawals
cannot be disputed, so nothing ever reads their amount or client again; they
are stored only to reject duplicate ids. A bitmap of all 2^32 ids is a fixed
512 MB and covers duplicate detection for every transaction, so only deposits
would need an entry in the map. Beyond that, a real system would keep disputable
deposits in an on-disk store, or only for a dispute window, as card networks do.

## Testing

`cargo test` runs 88 tests:

- **79 end-to-end tests** (`tests/cli.rs`) run the real binary on a CSV and check
  the accounts it prints, grouped as basics, withdrawals, disputes, resolves,
  chargebacks, locked accounts, duplicate ids and malformed input. Every printed
  account is also checked for `available + held == total` and at most 4 decimal
  places.
- **2 stream tests** (`tests/input.rs`): a source that fails partway through
  yields the I/O error and ends; malformed rows are skipped, not yielded.
- **7 unit tests** for `Amount` parsing and display.

`transactions.csv` is sample data covering every transaction type; the test
`sample_file_in_repo` checks its output.

## AI use

This solution was written with AI assistance (Claude Code). The session
transcript is in [TRANSCRIPT.md](TRANSCRIPT.md), with the challenge document,
the company name and personal details redacted. I designed and wrote the types, the engine and
the input and output modules. The AI reviewed the design and code, challenged
decisions, and wrote the test suites, which found several bugs (amounts parsed
through `f64`, negative amounts accepted, a panic on overflow). On my request it
applied specific changes I had decided on (the amount cap, small cleanups,
removing checked arithmetic) and drafted this README.
