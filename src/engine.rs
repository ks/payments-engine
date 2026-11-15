use std::collections::{BTreeMap, HashMap, hash_map};

use crate::types::*;

#[derive(Debug, Default)]
pub struct AccountBalance {
    total: Amount,
    held: Amount,
    disputable: BTreeMap<TxId, Amount>,
}

impl AccountBalance {
    /// May be negative: a deposit disputed after its funds were withdrawn.
    pub fn available(&self) -> Amount {
        self.total - self.held
    }
    pub fn amounts(&self) -> (Amount, Amount, Amount) {
        (self.available(), self.held, self.total)
    }
    pub fn deposit(&mut self, amount: Amount) {
        self.total += amount;
    }
    pub fn withdraw(&mut self, amount: Amount, client: &ClientId) -> TxResult {
        if self.available() >= amount {
            self.total -= amount;
            Ok(())
        } else {
            Err(TxError::AccountBalance(*client))
        }
    }
    /// Disputing an already disputed transaction changes nothing.
    pub fn open_dispute(&mut self, ref_id: TxId, amount: Amount) {
        if self.disputable.contains_key(&ref_id) {
            return;
        }
        self.held += amount;
        self.disputable.insert(ref_id, amount);
    }
    pub fn resolve_dispute(&mut self, ref_id: TxId, client: &ClientId) -> TxResult {
        match self.disputable.remove(&ref_id) {
            Some(amount) => {
                self.held -= amount;
                Ok(())
            }
            None => Err(TxError::DisputeNotFound(*client, ref_id)),
        }
    }
}

#[derive(Debug)]
pub enum Account {
    Normal(AccountBalance),
    Locked(AccountBalance),
}

impl Account {
    pub fn chargeback(&mut self, ref_id: TxId, client: &ClientId) -> TxResult {
        match self {
            Account::Locked(_) => Err(TxError::AccountLocked(*client)),
            Account::Normal(acc_balance) => {
                let Some(amount) = acc_balance.disputable.remove(&ref_id) else {
                    return Err(TxError::DisputeNotFound(*client, ref_id));
                };

                let mut acc_balance = std::mem::take(acc_balance);
                acc_balance.total -= amount;
                acc_balance.held -= amount;
                *self = Account::Locked(acc_balance);
                Ok(())
            }
        }
    }
}

#[derive(Debug, Default)]
pub struct Accounts(HashMap<ClientId, Account>);

impl Accounts {
    fn change_account(
        &mut self,
        client: &ClientId,
        mut action: impl FnMut(&mut Account, &ClientId) -> TxResult,
    ) -> TxResult {
        match self.0.get_mut(client) {
            Some(account) => action(account, client),
            None => Err(TxError::AccountNotFound(*client)),
        }
    }

    fn change_balance(
        &mut self,
        client: &ClientId,
        mut action: impl FnMut(&mut AccountBalance, &ClientId) -> TxResult,
    ) -> TxResult {
        self.change_account(client, |account, client| match account {
            Account::Normal(balance) => action(balance, client),
            Account::Locked(_) => Err(TxError::AccountLocked(*client)),
        })
    }

    pub(crate) fn deposit(&mut self, client: &ClientId, amount: Amount) -> TxResult {
        match self.0.entry(*client) {
            hash_map::Entry::Occupied(mut e) => {
                if let Account::Normal(balance) = e.get_mut() {
                    balance.deposit(amount);
                    Ok(())
                } else {
                    Err(TxError::AccountLocked(*client))
                }
            }
            hash_map::Entry::Vacant(e) => {
                let mut balance = AccountBalance::default();
                balance.deposit(amount);
                e.insert(Account::Normal(balance));
                Ok(())
            }
        }
    }

    pub(crate) fn withdrawal(&mut self, client: &ClientId, amount: Amount) -> TxResult {
        let action = |acc_balance: &mut AccountBalance, client: &ClientId| {
            acc_balance.withdraw(amount, client)
        };
        self.change_balance(client, action)
    }

    pub(crate) fn dispute(
        &mut self,
        client: &ClientId,
        amount: Amount,
        ref_id: TxId,
        change: Dispute,
    ) -> TxResult {
        let action = |account: &mut Account, client: &ClientId| match (account, change) {
            (Account::Locked(_), _) => Err(TxError::AccountLocked(*client)),
            (Account::Normal(acc_balance), Dispute::Open) => {
                acc_balance.open_dispute(ref_id, amount);
                Ok(())
            }
            (Account::Normal(acc_balance), Dispute::Resolve) => {
                acc_balance.resolve_dispute(ref_id, client)
            }
            (account, Dispute::Chargeback) => account.chargeback(ref_id, client),
        };
        self.change_account(client, action)
    }
}

#[derive(Debug)]
pub struct DbTx {
    client: ClientId,
    change: Balance,
}

#[derive(Debug, Default)]
pub struct Transactions(HashMap<TxId, DbTx>);

impl Transactions {
    pub fn get(&self, id: &TxId) -> Option<&DbTx> {
        self.0.get(id)
    }

    pub fn insert(&mut self, id: TxId, tx: DbTx) -> Option<DbTx> {
        self.0.insert(id, tx)
    }
}

#[derive(Debug, Default)]
pub struct Engine {
    accounts: Accounts,
    transactions: Transactions,
}

#[derive(Debug, Copy, Clone)]
pub enum TxError {
    AccountNotFound(ClientId),
    AccountLocked(ClientId),
    AccountBalance(ClientId),
    TransactionNotFound(TxId),
    TransactionClient(TxId, ClientId),
    DisputeNotFound(ClientId, TxId),
}

pub type TxResult = Result<(), TxError>;

impl Engine {
    fn handle_balance(&mut self, client: ClientId, id: TxId, change: Balance) -> TxResult {
        let Self {
            accounts,
            transactions,
        } = self;

        if transactions.get(&id).is_some() {
            return Ok(());
        }

        let res = match change {
            Balance::Deposit(amount) => accounts.deposit(&client, amount),
            Balance::Withdrawal(amount) => accounts.withdrawal(&client, amount),
        };

        if res.is_ok() {
            transactions.insert(id, DbTx { client, change });
        }

        res
    }

    fn handle_dispute(&mut self, ref_client: ClientId, ref_id: TxId, change: Dispute) -> TxResult {
        let Self {
            accounts,
            transactions,
        } = self;

        let Some(DbTx {
            client,
            change: Balance::Deposit(amount),
        }) = transactions.get(&ref_id)
        else {
            return Err(TxError::TransactionNotFound(ref_id));
        };

        if *client != ref_client {
            return Err(TxError::TransactionClient(ref_id, ref_client));
        }

        accounts.dispute(client, *amount, ref_id, change)
    }

    pub fn process(&mut self, tx: &Tx) -> TxResult {
        match tx.op {
            TxOp::Balance { id, change } => self.handle_balance(tx.client, id, change),
            TxOp::Dispute { ref_id, change } => self.handle_dispute(tx.client, ref_id, change),
        }
    }

    pub fn accounts_iter(&self) -> impl Iterator<Item = (&ClientId, &Account)> {
        self.accounts.0.iter()
    }
}
