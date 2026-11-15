use csv::WriterBuilder;
use serde::{Serialize, Serializer};
use std::io::Write;

use crate::engine::Account;
use crate::types::{Amount, ClientId};

pub fn serialize_display<T, S>(val: &T, serializer: S) -> Result<S::Ok, S::Error>
where
    T: std::fmt::Display,
    S: Serializer,
{
    serializer.collect_str(val)
}

/// Column names, in the field order of [`AccountRow`].
const HEADER: [&str; 5] = ["client", "available", "held", "total", "locked"];

#[derive(Debug, Serialize)]
pub struct AccountRow {
    client: ClientId,
    #[serde(serialize_with = "serialize_display")]
    available: Amount,
    #[serde(serialize_with = "serialize_display")]
    held: Amount,
    #[serde(serialize_with = "serialize_display")]
    total: Amount,
    locked: bool,
}

impl From<(&ClientId, &Account)> for AccountRow {
    fn from((client, account): (&ClientId, &Account)) -> Self {
        let (balance, locked) = match account {
            Account::Normal(balance) => (balance, false),
            Account::Locked(balance) => (balance, true),
        };
        let (available, held, total) = balance.amounts();
        Self {
            client: *client,
            available,
            held,
            total,
            locked,
        }
    }
}

pub fn write_accounts<'a, W, I>(writer: W, rows: I) -> csv::Result<()>
where
    W: Write,
    I: Iterator<Item = (&'a ClientId, &'a Account)>,
{
    // csv writes headers only with the first serialized row, so an input with
    // no accounts would print nothing at all. Write the header explicitly.
    let mut writer = WriterBuilder::new().has_headers(false).from_writer(writer);
    writer.write_record(HEADER)?;

    for row in rows {
        writer.serialize::<AccountRow>(row.into())?;
    }

    writer.flush()?;
    Ok(())
}
