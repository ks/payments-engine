//! End-to-end tests: each test writes a CSV, runs the real binary on it and
//! checks the accounts it prints. This is the same interface the spec scores.
//!
//! Amounts are compared as decimals, so `1.5`, `1.50` and `1.5000` are equal,
//! and row order does not matter (both as allowed by the spec).

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::str::FromStr;
use std::sync::atomic::{AtomicUsize, Ordering};

use rust_decimal::Decimal;

// ---------------------------------------------------------------- harness

#[derive(Debug, PartialEq)]
struct Account {
    available: Decimal,
    held: Decimal,
    total: Decimal,
    locked: bool,
}

type Accounts = BTreeMap<u16, Account>;

const HEADER: &str = "type, client, tx, amount";

fn d(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

fn input_file(contents: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("cli-{}-{n}.csv", std::process::id()));
    std::fs::write(&path, contents).unwrap();
    path
}

fn run_raw(contents: &str) -> Output {
    let path = input_file(contents);
    let out = Command::new(env!("CARGO_BIN_EXE_payments-engine"))
        .arg(&path)
        .output()
        .unwrap();
    std::fs::remove_file(&path).ok();
    out
}

/// Runs the engine on `HEADER` + `rows` and returns the parsed accounts.
/// Fails the test if the process does not exit cleanly, and checks the
/// `available + held == total` invariant on every printed account.
fn run(rows: &[&str]) -> Accounts {
    let mut csv = String::from(HEADER);
    for row in rows {
        csv.push('\n');
        csv.push_str(row);
    }
    csv.push('\n');
    run_csv(&csv)
}

fn run_csv(contents: &str) -> Accounts {
    let out = run_raw(contents);
    assert!(
        out.status.success(),
        "engine exited with {}\nstderr:\n{}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    parse_output(&String::from_utf8(out.stdout).unwrap())
}

fn parse_output(stdout: &str) -> Accounts {
    let mut reader = csv::ReaderBuilder::new()
        .trim(csv::Trim::All)
        .from_reader(stdout.as_bytes());

    let header: Vec<String> = reader.headers().unwrap().iter().map(String::from).collect();
    if !stdout.trim().is_empty() {
        assert_eq!(header, ["client", "available", "held", "total", "locked"]);
    }

    let mut accounts = Accounts::new();
    for record in reader.records() {
        let r = record.unwrap();
        let client: u16 = r[0].parse().unwrap();
        let account = Account {
            available: d(&r[1]),
            held: d(&r[2]),
            total: d(&r[3]),
            locked: r[4].parse().unwrap(),
        };
        assert_eq!(
            account.available + account.held,
            account.total,
            "available + held != total for client {client}: {account:?}"
        );
        assert!(
            account.available.scale() <= 4
                && account.held.scale() <= 4
                && account.total.scale() <= 4,
            "more than 4 decimal places in output for client {client}: {account:?}"
        );
        let previous = accounts.insert(client, account);
        assert!(previous.is_none(), "client {client} printed twice");
    }
    accounts
}

#[track_caller]
fn assert_account(
    accounts: &Accounts,
    client: u16,
    available: &str,
    held: &str,
    total: &str,
    locked: bool,
) {
    let expected = Account {
        available: d(available),
        held: d(held),
        total: d(total),
        locked,
    };
    match accounts.get(&client) {
        Some(actual) => assert_eq!(actual, &expected, "client {client}"),
        None => panic!("client {client} missing from output: {accounts:?}"),
    }
}

// ---------------------------------------------------------------- basics

mod basics {
    use super::*;

    #[test]
    fn deposits_and_withdrawals() {
        let out = run(&[
            "deposit, 1, 1, 4.0",
            "deposit, 2, 2, 3.0",
            "deposit, 1, 3, 1.5",
            "withdrawal, 1, 4, 2.25",
            "withdrawal, 2, 5, 3.5",
        ]);
        assert_eq!(out.len(), 2);
        assert_account(&out, 1, "3.25", "0", "3.25", false);
        assert_account(&out, 2, "3", "0", "3", false);
    }

    #[test]
    fn sample_file_in_repo() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/transactions.csv");
        let out = run_csv(&std::fs::read_to_string(path).unwrap());
        assert_eq!(out.len(), 3);
        assert_account(&out, 1, "89.75", "0", "89.75", false);
        assert_account(&out, 2, "50.5", "0", "50.5", true);
        assert_account(&out, 3, "1.2345", "0", "1.2345", false);
    }

    #[test]
    fn output_has_four_decimal_places() {
        let out = run_raw(&format!("{HEADER}\ndeposit, 1, 1, 1.5\n"));
        let stdout = String::from_utf8(out.stdout).unwrap();
        assert_eq!(
            stdout,
            "client,available,held,total,locked\n1,1.5000,0.0000,1.5000,false\n"
        );
    }

    #[test]
    fn no_whitespace_is_accepted() {
        let out = run_csv("type,client,tx,amount\ndeposit,1,1,1.0\nwithdrawal,1,2,0.25\n");
        assert_account(&out, 1, "0.75", "0", "0.75", false);
    }

    #[test]
    fn irregular_whitespace_is_accepted() {
        let out = run_csv(
            "type ,  client,tx ,amount\n  deposit ,1,   1,   3.5  \n\twithdrawal,\t1,\t2,\t1\n",
        );
        assert_account(&out, 1, "2.5", "0", "2.5", false);
    }

    #[test]
    fn four_decimal_places_are_exact() {
        let out = run(&[
            "deposit, 1, 1, 0.1",
            "deposit, 1, 2, 0.2",
            "deposit, 1, 3, 0.0001",
            "withdrawal, 1, 4, 0.0002",
        ]);
        assert_account(&out, 1, "0.2999", "0", "0.2999", false);
    }

    #[test]
    fn many_small_deposits_do_not_drift() {
        let rows: Vec<String> = (1..=10_000)
            .map(|tx| format!("deposit, 1, {tx}, 0.0001"))
            .collect();
        let rows: Vec<&str> = rows.iter().map(String::as_str).collect();
        let out = run(&rows);
        assert_account(&out, 1, "1", "0", "1", false);
    }

    #[test]
    fn integer_amounts_are_accepted() {
        let out = run(&["deposit, 1, 1, 5", "withdrawal, 1, 2, 2"]);
        assert_account(&out, 1, "3", "0", "3", false);
    }

    #[test]
    fn clients_are_independent_and_unordered() {
        let out = run(&[
            "deposit, 3, 1, 3",
            "deposit, 1, 2, 1",
            "deposit, 2, 3, 2",
            "withdrawal, 3, 4, 1",
        ]);
        assert_eq!(out.len(), 3);
        assert_account(&out, 1, "1", "0", "1", false);
        assert_account(&out, 2, "2", "0", "2", false);
        assert_account(&out, 3, "2", "0", "2", false);
    }

    #[test]
    fn tx_ids_need_not_be_ordered() {
        let out = run(&[
            "deposit, 1, 100, 5",
            "deposit, 1, 7, 5",
            "withdrawal, 1, 50, 3",
        ]);
        assert_account(&out, 1, "7", "0", "7", false);
    }

    #[test]
    fn extreme_ids_are_accepted() {
        let out = run(&["deposit, 65535, 4294967295, 1", "deposit, 0, 0, 2"]);
        assert_account(&out, 65535, "1", "0", "1", false);
        assert_account(&out, 0, "2", "0", "2", false);
    }

    #[test]
    fn header_only_input_prints_header_only() {
        let out = run_raw(&format!("{HEADER}\n"));
        assert!(out.status.success());
        assert_eq!(
            String::from_utf8(out.stdout).unwrap(),
            "client,available,held,total,locked\n"
        );
    }
}

// ---------------------------------------------------------------- withdrawals

mod withdrawals {
    use super::*;

    #[test]
    fn withdrawing_exact_balance_succeeds() {
        let out = run(&["deposit, 1, 1, 2.5", "withdrawal, 1, 2, 2.5"]);
        assert_account(&out, 1, "0", "0", "0", false);
    }

    #[test]
    fn insufficient_funds_fails_and_balance_is_unchanged() {
        let out = run(&["deposit, 1, 1, 2.5", "withdrawal, 1, 2, 2.5001"]);
        assert_account(&out, 1, "2.5", "0", "2.5", false);
    }

    #[test]
    fn failed_withdrawal_does_not_block_later_ones() {
        let out = run(&[
            "deposit, 1, 1, 2",
            "withdrawal, 1, 2, 3",
            "withdrawal, 1, 3, 1",
        ]);
        assert_account(&out, 1, "1", "0", "1", false);
    }

    #[test]
    fn withdrawal_by_unknown_client_does_not_create_money() {
        // Whether the client then appears (with zeros) or not is a policy
        // choice; either way it must not have a non-zero balance.
        let out = run(&["withdrawal, 9, 1, 1", "deposit, 1, 2, 1"]);
        if out.contains_key(&9) {
            assert_account(&out, 9, "0", "0", "0", false);
        }
        assert_account(&out, 1, "1", "0", "1", false);
    }

    #[test]
    fn held_funds_cannot_be_withdrawn() {
        let out = run(&[
            "deposit, 1, 1, 10",
            "deposit, 1, 2, 5",
            "dispute, 1, 1",
            "withdrawal, 1, 3, 6",
        ]);
        assert_account(&out, 1, "5", "10", "15", false);
    }

    #[test]
    fn withdrawal_up_to_available_while_disputed() {
        let out = run(&[
            "deposit, 1, 1, 10",
            "deposit, 1, 2, 5",
            "dispute, 1, 1",
            "withdrawal, 1, 3, 5",
        ]);
        assert_account(&out, 1, "0", "10", "10", false);
    }
}

// ---------------------------------------------------------------- disputes

mod disputes {
    use super::*;

    #[test]
    fn dispute_moves_funds_to_held() {
        let out = run(&["deposit, 1, 1, 10", "deposit, 1, 2, 3", "dispute, 1, 1"]);
        assert_account(&out, 1, "3", "10", "13", false);
    }

    #[test]
    fn dispute_row_with_trailing_empty_amount() {
        // Common real-world shape: the amount column is present but empty.
        let out = run(&["deposit, 1, 1, 10", "dispute, 1, 1,"]);
        assert_account(&out, 1, "0", "10", "10", false);
    }

    #[test]
    fn dispute_unknown_tx_is_ignored() {
        let out = run(&["deposit, 1, 1, 10", "dispute, 1, 99"]);
        assert_account(&out, 1, "10", "0", "10", false);
    }

    #[test]
    fn dispute_by_other_client_is_ignored() {
        let out = run(&["deposit, 1, 1, 10", "deposit, 2, 2, 1", "dispute, 2, 1"]);
        assert_account(&out, 1, "10", "0", "10", false);
        assert_account(&out, 2, "1", "0", "1", false);
    }

    #[test]
    fn dispute_by_nonexistent_client_is_ignored() {
        let out = run(&["deposit, 1, 1, 10", "dispute, 7, 1"]);
        assert_account(&out, 1, "10", "0", "10", false);
        assert!(
            !out.contains_key(&7)
                || out[&7]
                    == Account {
                        available: d("0"),
                        held: d("0"),
                        total: d("0"),
                        locked: false
                    }
        );
    }

    #[test]
    fn dispute_of_withdrawal_is_ignored() {
        // Design decision (see README): only deposits are disputable.
        let out = run(&["deposit, 1, 1, 10", "withdrawal, 1, 2, 4", "dispute, 1, 2"]);
        assert_account(&out, 1, "6", "0", "6", false);
    }

    #[test]
    fn dispute_of_failed_withdrawal_is_ignored() {
        let out = run(&["deposit, 1, 1, 1", "withdrawal, 1, 2, 4", "dispute, 1, 2"]);
        assert_account(&out, 1, "1", "0", "1", false);
    }

    #[test]
    fn repeated_dispute_does_not_double_hold() {
        let out = run(&[
            "deposit, 1, 1, 10",
            "deposit, 1, 2, 5",
            "dispute, 1, 1",
            "dispute, 1, 1",
            "dispute, 1, 1",
        ]);
        assert_account(&out, 1, "5", "10", "15", false);
    }

    #[test]
    fn two_disputes_on_different_txs_both_hold() {
        let out = run(&[
            "deposit, 1, 1, 10",
            "deposit, 1, 2, 5",
            "deposit, 1, 3, 1",
            "dispute, 1, 1",
            "dispute, 1, 2",
        ]);
        assert_account(&out, 1, "1", "15", "16", false);
    }

    #[test]
    fn dispute_after_funds_withdrawn_makes_available_negative() {
        // The money already left; the hold is still the full disputed amount.
        let out = run(&["deposit, 1, 1, 10", "withdrawal, 1, 2, 8", "dispute, 1, 1"]);
        assert_account(&out, 1, "-8", "10", "2", false);
    }

    #[test]
    fn negative_available_blocks_withdrawals() {
        let out = run(&[
            "deposit, 1, 1, 10",
            "withdrawal, 1, 2, 8",
            "dispute, 1, 1",
            "withdrawal, 1, 3, 0.0001",
        ]);
        assert_account(&out, 1, "-8", "10", "2", false);
    }

    #[test]
    fn deposit_while_disputed_adds_to_available() {
        let out = run(&["deposit, 1, 1, 10", "dispute, 1, 1", "deposit, 1, 2, 3"]);
        assert_account(&out, 1, "3", "10", "13", false);
    }

    #[test]
    fn disputes_do_not_touch_other_clients() {
        let out = run(&["deposit, 1, 1, 10", "deposit, 2, 2, 10", "dispute, 1, 1"]);
        assert_account(&out, 1, "0", "10", "10", false);
        assert_account(&out, 2, "10", "0", "10", false);
    }
}

// ---------------------------------------------------------------- resolves

mod resolves {
    use super::*;

    #[test]
    fn resolve_releases_held_funds() {
        let out = run(&["deposit, 1, 1, 10", "dispute, 1, 1", "resolve, 1, 1"]);
        assert_account(&out, 1, "10", "0", "10", false);
    }

    #[test]
    fn resolve_with_trailing_empty_amount() {
        let out = run(&["deposit, 1, 1, 10", "dispute, 1, 1,", "resolve, 1, 1,"]);
        assert_account(&out, 1, "10", "0", "10", false);
    }

    #[test]
    fn resolve_without_dispute_is_ignored() {
        let out = run(&["deposit, 1, 1, 10", "resolve, 1, 1"]);
        assert_account(&out, 1, "10", "0", "10", false);
    }

    #[test]
    fn resolve_unknown_tx_is_ignored() {
        let out = run(&["deposit, 1, 1, 10", "dispute, 1, 1", "resolve, 1, 99"]);
        assert_account(&out, 1, "0", "10", "10", false);
    }

    #[test]
    fn resolve_by_other_client_is_ignored() {
        let out = run(&[
            "deposit, 1, 1, 10",
            "deposit, 2, 2, 1",
            "dispute, 1, 1",
            "resolve, 2, 1",
        ]);
        assert_account(&out, 1, "0", "10", "10", false);
        assert_account(&out, 2, "1", "0", "1", false);
    }

    #[test]
    fn resolve_twice_releases_once() {
        let out = run(&[
            "deposit, 1, 1, 10",
            "deposit, 1, 2, 5",
            "dispute, 1, 1",
            "dispute, 1, 2",
            "resolve, 1, 1",
            "resolve, 1, 1",
        ]);
        assert_account(&out, 1, "10", "5", "15", false);
    }

    #[test]
    fn resolve_only_releases_the_referenced_tx() {
        let out = run(&[
            "deposit, 1, 1, 10",
            "deposit, 1, 2, 5",
            "dispute, 1, 1",
            "dispute, 1, 2",
            "resolve, 1, 2",
        ]);
        assert_account(&out, 1, "5", "10", "15", false);
    }

    #[test]
    fn resolve_after_withdrawal_restores_available() {
        let out = run(&[
            "deposit, 1, 1, 10",
            "withdrawal, 1, 2, 8",
            "dispute, 1, 1",
            "resolve, 1, 1",
        ]);
        assert_account(&out, 1, "2", "0", "2", false);
    }

    #[test]
    fn resolved_tx_can_be_disputed_again() {
        // Policy pinned by this test: a resolved deposit is disputable again.
        // If you decide on a terminal Resolved state, flip the expectation.
        let out = run(&[
            "deposit, 1, 1, 10",
            "dispute, 1, 1",
            "resolve, 1, 1",
            "dispute, 1, 1",
        ]);
        assert_account(&out, 1, "0", "10", "10", false);
    }

    #[test]
    fn resolve_after_chargeback_is_ignored() {
        let out = run(&[
            "deposit, 1, 1, 10",
            "deposit, 1, 2, 1",
            "dispute, 1, 1",
            "chargeback, 1, 1",
            "resolve, 1, 1",
        ]);
        assert_account(&out, 1, "1", "0", "1", true);
    }
}

// ---------------------------------------------------------------- chargebacks

mod chargebacks {
    use super::*;

    #[test]
    fn chargeback_removes_held_funds_and_locks() {
        let out = run(&[
            "deposit, 1, 1, 10",
            "deposit, 1, 2, 3",
            "dispute, 1, 1",
            "chargeback, 1, 1",
        ]);
        assert_account(&out, 1, "3", "0", "3", true);
    }

    #[test]
    fn chargeback_with_trailing_empty_amount() {
        let out = run(&["deposit, 1, 1, 10", "dispute, 1, 1,", "chargeback, 1, 1,"]);
        assert_account(&out, 1, "0", "0", "0", true);
    }

    #[test]
    fn chargeback_without_dispute_is_ignored() {
        let out = run(&["deposit, 1, 1, 10", "chargeback, 1, 1"]);
        assert_account(&out, 1, "10", "0", "10", false);
    }

    #[test]
    fn chargeback_unknown_tx_is_ignored() {
        let out = run(&["deposit, 1, 1, 10", "dispute, 1, 1", "chargeback, 1, 99"]);
        assert_account(&out, 1, "0", "10", "10", false);
    }

    #[test]
    fn chargeback_by_other_client_is_ignored() {
        let out = run(&[
            "deposit, 1, 1, 10",
            "deposit, 2, 2, 1",
            "dispute, 1, 1",
            "chargeback, 2, 1",
        ]);
        assert_account(&out, 1, "0", "10", "10", false);
        assert_account(&out, 2, "1", "0", "1", false);
    }

    #[test]
    fn chargeback_after_resolve_is_ignored() {
        let out = run(&[
            "deposit, 1, 1, 10",
            "dispute, 1, 1",
            "resolve, 1, 1",
            "chargeback, 1, 1",
        ]);
        assert_account(&out, 1, "10", "0", "10", false);
    }

    #[test]
    fn chargeback_twice_reverses_once() {
        let out = run(&[
            "deposit, 1, 1, 10",
            "deposit, 1, 2, 5",
            "dispute, 1, 1",
            "chargeback, 1, 1",
            "chargeback, 1, 1",
        ]);
        assert_account(&out, 1, "5", "0", "5", true);
    }

    #[test]
    fn dispute_resolve_dispute_chargeback_reverses_once() {
        let out = run(&[
            "deposit, 1, 1, 10",
            "deposit, 1, 2, 5",
            "dispute, 1, 1",
            "resolve, 1, 1",
            "dispute, 1, 1",
            "chargeback, 1, 1",
        ]);
        assert_account(&out, 1, "5", "0", "5", true);
    }

    #[test]
    fn chargeback_after_withdrawal_makes_total_negative() {
        // The fraud case from the spec: deposit, withdraw, reverse the deposit.
        let out = run(&[
            "deposit, 1, 1, 10",
            "withdrawal, 1, 2, 10",
            "dispute, 1, 1",
            "chargeback, 1, 1",
        ]);
        assert_account(&out, 1, "-10", "0", "-10", true);
    }

    #[test]
    fn chargeback_does_not_lock_other_clients() {
        let out = run(&[
            "deposit, 1, 1, 10",
            "deposit, 2, 2, 10",
            "dispute, 1, 1",
            "chargeback, 1, 1",
            "deposit, 2, 3, 1",
            "withdrawal, 2, 4, 2",
        ]);
        assert_account(&out, 1, "0", "0", "0", true);
        assert_account(&out, 2, "9", "0", "9", false);
    }
}

// ---------------------------------------------------------------- locked accounts

mod locked {
    use super::*;

    const LOCK_CLIENT_1: [&str; 4] = [
        "deposit, 1, 1, 10",
        "deposit, 1, 2, 5",
        "dispute, 1, 1",
        "chargeback, 1, 1",
    ];

    fn run_after_lock(rows: &[&str]) -> Accounts {
        let all: Vec<&str> = LOCK_CLIENT_1.iter().chain(rows).copied().collect();
        run(&all)
    }

    #[test]
    fn deposit_to_locked_account_is_rejected() {
        let out = run_after_lock(&["deposit, 1, 10, 100"]);
        assert_account(&out, 1, "5", "0", "5", true);
    }

    #[test]
    fn withdrawal_from_locked_account_is_rejected() {
        let out = run_after_lock(&["withdrawal, 1, 10, 1"]);
        assert_account(&out, 1, "5", "0", "5", true);
    }

    #[test]
    fn new_dispute_on_locked_account_is_rejected() {
        let out = run_after_lock(&["dispute, 1, 2"]);
        assert_account(&out, 1, "5", "0", "5", true);
    }

    #[test]
    fn rejected_deposit_on_locked_account_cannot_be_disputed_later() {
        let out = run_after_lock(&["deposit, 1, 10, 100", "dispute, 1, 10", "chargeback, 1, 10"]);
        assert_account(&out, 1, "5", "0", "5", true);
    }

    #[test]
    fn rejected_deposit_id_is_free_for_another_client() {
        // tx 10 was never applied, so another client using it is not a duplicate.
        let out = run_after_lock(&["deposit, 1, 10, 100", "deposit, 2, 10, 7"]);
        assert_account(&out, 1, "5", "0", "5", true);
        assert_account(&out, 2, "7", "0", "7", false);
    }

    #[test]
    fn open_dispute_stays_held_after_lock() {
        // Documented policy (README, Assumptions): a locked account rejects
        // everything, so a dispute still open at the lock stays held until
        // someone reviews the frozen account; the engine does not settle it.
        let out = run(&[
            "deposit, 1, 1, 10",
            "deposit, 1, 2, 5",
            "dispute, 1, 1",
            "dispute, 1, 2",
            "chargeback, 1, 1",
            "resolve, 1, 2",
            "chargeback, 1, 2",
        ]);
        assert_account(&out, 1, "0", "5", "5", true);
    }
}

// ---------------------------------------------------------------- duplicate tx ids

mod duplicates {
    use super::*;

    #[test]
    fn duplicate_deposit_id_is_ignored() {
        let out = run(&["deposit, 1, 1, 10", "deposit, 1, 1, 10"]);
        assert_account(&out, 1, "10", "0", "10", false);
    }

    #[test]
    fn duplicate_id_from_other_client_is_ignored() {
        let out = run(&["deposit, 1, 1, 10", "deposit, 2, 1, 99"]);
        assert_account(&out, 1, "10", "0", "10", false);
        assert!(!out.contains_key(&2) || out[&2].total == d("0"));
    }

    #[test]
    fn withdrawal_reusing_deposit_id_is_ignored() {
        let out = run(&["deposit, 1, 1, 10", "withdrawal, 1, 1, 4"]);
        assert_account(&out, 1, "10", "0", "10", false);
    }

    #[test]
    fn dispute_of_duplicated_id_holds_original_amount() {
        let out = run(&["deposit, 1, 1, 10", "deposit, 1, 1, 99", "dispute, 1, 1"]);
        assert_account(&out, 1, "0", "10", "10", false);
    }
}

// ---------------------------------------------------------------- malformed input

mod malformed {
    use super::*;

    #[test]
    fn more_than_four_decimal_places_is_rejected() {
        let out = run(&[
            "deposit, 1, 1, 1",
            "deposit, 1, 2, 0.00001",
            "withdrawal, 1, 3, 0.12345",
        ]);
        assert_account(&out, 1, "1", "0", "1", false);
    }

    #[test]
    fn trailing_zeros_beyond_four_places_are_accepted() {
        let out = run(&["deposit, 1, 1, 1.500000"]);
        assert_account(&out, 1, "1.5", "0", "1.5", false);
    }

    #[test]
    fn negative_deposit_is_rejected() {
        // Otherwise a negative deposit is a withdrawal without a funds check.
        let out = run(&["deposit, 1, 1, 10", "deposit, 1, 2, -50"]);
        assert_account(&out, 1, "10", "0", "10", false);
    }

    #[test]
    fn negative_withdrawal_is_rejected() {
        // Otherwise a negative withdrawal is a deposit.
        let out = run(&["deposit, 1, 1, 10", "withdrawal, 1, 2, -50"]);
        assert_account(&out, 1, "10", "0", "10", false);
    }

    #[test]
    fn zero_amount_is_rejected() {
        // A zero deposit would otherwise create an empty account for an unknown client.
        let out = run(&[
            "deposit, 1, 1, 10",
            "withdrawal, 1, 2, 0.0000",
            "deposit, 2, 3, 0",
        ]);
        assert_account(&out, 1, "10", "0", "10", false);
        assert!(
            !out.contains_key(&2),
            "zero deposit must not create client 2: {out:?}"
        );
    }

    #[test]
    fn deposit_without_amount_is_skipped() {
        let out = run(&["deposit, 1, 1, 10", "deposit, 1, 2,", "deposit, 1, 3"]);
        assert_account(&out, 1, "10", "0", "10", false);
    }

    #[test]
    fn withdrawal_without_amount_is_skipped() {
        let out = run(&["deposit, 1, 1, 10", "withdrawal, 1, 2,"]);
        assert_account(&out, 1, "10", "0", "10", false);
    }

    #[test]
    fn non_numeric_amount_is_skipped() {
        let out = run(&[
            "deposit, 1, 1, ten",
            "deposit, 1, 2, 1.2.3",
            "deposit, 1, 3, 2",
        ]);
        assert_account(&out, 1, "2", "0", "2", false);
    }

    #[test]
    fn scientific_notation_is_rejected() {
        // Policy: the spec describes plain decimals; `1e3` is not one.
        let out = run(&[
            "deposit, 1, 1, 1e3",
            "deposit, 1, 2, 1E-2",
            "deposit, 1, 3, 2",
        ]);
        assert_account(&out, 1, "2", "0", "2", false);
    }

    #[test]
    fn large_amounts_keep_full_precision() {
        // Amounts must never pass through f64: 17 significant digits do not survive it.
        let out = run(&[
            "deposit, 1, 1, 1234567890123.4567",
            "deposit, 2, 2, 99999999999999.9999",
            "deposit, 3, 3, 999999999999999.9999",
        ]);
        assert_account(
            &out,
            1,
            "1234567890123.4567",
            "0",
            "1234567890123.4567",
            false,
        );
        assert_account(
            &out,
            2,
            "99999999999999.9999",
            "0",
            "99999999999999.9999",
            false,
        );
        assert_account(
            &out,
            3,
            "999999999999999.9999",
            "0",
            "999999999999999.9999",
            false,
        );
    }

    #[test]
    fn unknown_type_is_skipped_and_processing_continues() {
        let out = run(&[
            "deposit, 1, 1, 10",
            "transfer, 1, 2, 5",
            "Deposit, 1, 3, 5",
            "withdrawal, 1, 4, 1",
        ]);
        assert_account(&out, 1, "9", "0", "9", false);
    }

    #[test]
    fn out_of_range_ids_are_skipped() {
        let out = run(&[
            "deposit, 65536, 1, 10",
            "deposit, -1, 2, 10",
            "deposit, 1, 4294967296, 10",
            "deposit, 1, 3, 1",
        ]);
        assert_eq!(out.len(), 1);
        assert_account(&out, 1, "1", "0", "1", false);
    }

    #[test]
    fn short_and_empty_rows_are_skipped() {
        let out = run(&[
            "deposit, 1, 1, 10",
            "",
            "deposit",
            "deposit, 1",
            "dispute, 1",
            ",,,",
            "deposit, 1, 2, 1",
        ]);
        assert_account(&out, 1, "11", "0", "11", false);
    }

    #[test]
    fn overflow_does_not_crash() {
        // Decimal::MAX (~7.9e28) is far above the per-transaction maximum, so
        // both rows are rejected as invalid amounts. The run must not panic
        // and must still output the valid rows.
        let max = "79228162514264337593543950335";
        let out = run(&[
            &format!("deposit, 1, 1, {max}"),
            &format!("deposit, 1, 2, {max}"),
            "deposit, 2, 3, 1",
        ]);
        assert!(
            !out.contains_key(&1),
            "client 1 should have no valid rows: {out:?}"
        );
        assert_account(&out, 2, "1", "0", "1", false);
    }

    #[test]
    fn maximum_amount_is_accepted() {
        let out = run(&[
            "deposit, 1, 1, 1000000000000000",
            "deposit, 1, 2, 1000000000000000.0000",
        ]);
        assert_account(&out, 1, "2000000000000000", "0", "2000000000000000", false);
    }

    #[test]
    fn above_maximum_amount_is_rejected() {
        let out = run(&[
            "deposit, 1, 1, 1",
            "deposit, 1, 2, 1000000000000000.0001",
            "withdrawal, 1, 3, 1000000000000000.0001",
        ]);
        assert_account(&out, 1, "1", "0", "1", false);
    }

    #[test]
    fn missing_input_file_fails_cleanly() {
        let out = Command::new(env!("CARGO_BIN_EXE_payments-engine"))
            .arg("/nonexistent/transactions.csv")
            .output()
            .unwrap();
        assert!(!out.status.success());
        assert!(out.stdout.is_empty());
    }

    #[test]
    fn missing_argument_prints_usage() {
        let out = Command::new(env!("CARGO_BIN_EXE_payments-engine"))
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(2));
        assert!(out.stdout.is_empty());
        assert!(String::from_utf8_lossy(&out.stderr).contains("usage"));
    }
}
