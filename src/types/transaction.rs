use crate::types::*;

#[derive(Debug, Copy, Clone)]
pub struct Tx {
    pub client: ClientId,
    pub op: TxOp,
}

#[derive(Debug, Copy, Clone)]
pub enum Balance {
    Deposit(Amount),
    Withdrawal(Amount),
}

#[derive(Debug, Copy, Clone)]
pub enum Dispute {
    Open,
    Resolve,
    Chargeback,
}

#[derive(Debug, Copy, Clone)]
pub enum TxOp {
    Balance { id: TxId, change: Balance },
    Dispute { ref_id: TxId, change: Dispute },
}
