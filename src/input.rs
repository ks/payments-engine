use std::fs::File;
use std::io::Read;
use std::path::Path;

use csv::{Position, Reader, ReaderBuilder, StringRecord, Trim};

use crate::types::{Amount, Balance, Dispute, Tx, TxId, TxOp};

#[derive(Debug)]
pub enum ParseError {
    Empty,
    TxOp(String),
    Csv(csv::Error),
}

impl From<csv::Error> for ParseError {
    fn from(value: csv::Error) -> Self {
        Self::Csv(value)
    }
}

impl TryFrom<(&str, TxId, Amount)> for TxOp {
    type Error = ParseError;

    fn try_from((tag, id, amount): (&str, TxId, Amount)) -> Result<Self, Self::Error> {
        match tag {
            "deposit" => Ok(TxOp::Balance {
                id,
                change: Balance::Deposit(amount),
            }),
            "withdrawal" => Ok(TxOp::Balance {
                id,
                change: Balance::Withdrawal(amount),
            }),
            _ => Err(ParseError::TxOp(tag.to_string())),
        }
    }
}

impl TryFrom<(&str, TxId)> for TxOp {
    type Error = ParseError;

    fn try_from((tag, ref_id): (&str, TxId)) -> Result<Self, Self::Error> {
        match tag {
            "dispute" => Ok(TxOp::Dispute {
                ref_id,
                change: Dispute::Open,
            }),
            "resolve" => Ok(TxOp::Dispute {
                ref_id,
                change: Dispute::Resolve,
            }),
            "chargeback" => Ok(TxOp::Dispute {
                ref_id,
                change: Dispute::Chargeback,
            }),
            _ => Err(ParseError::TxOp(tag.to_string())),
        }
    }
}

impl TryFrom<&StringRecord> for Tx {
    type Error = ParseError;

    fn try_from(value: &StringRecord) -> Result<Self, Self::Error> {
        let tag = value.get(0).ok_or(ParseError::Empty)?;
        let fields = StringRecord::from(value.iter().skip(1).collect::<Vec<_>>());

        match tag {
            "deposit" | "withdrawal" => {
                let (client, id, amount) = fields.deserialize(None)?;
                let op = TxOp::try_from((tag, id, amount))?;
                Ok(Tx { client, op })
            }
            "dispute" | "resolve" | "chargeback" => {
                let (client, ref_id) = fields.deserialize(None)?;
                let op = TxOp::try_from((tag, ref_id))?;
                Ok(Tx { client, op })
            }
            tag => Err(ParseError::TxOp(tag.to_string()))?,
        }
    }
}

fn log_skipped<E: std::fmt::Debug>(position: Option<&Position>, err: &E) {
    if cfg!(feature = "verbose") {
        match position {
            Some(pos) => eprintln!("skipped line {}: {err:?}", pos.line()),
            None => eprintln!("skipped row: {err:?}"),
        }
    }
}

/// Malformed rows are skipped (logged with `--features verbose`), an I/O error
/// is passed on: the rest of the stream cannot be trusted.
fn parse_record(record: Result<StringRecord, csv::Error>) -> Option<Result<Tx, csv::Error>> {
    match record {
        Err(err) if err.is_io_error() => Some(Err(err)),
        Err(err) => {
            log_skipped(err.position(), &err);
            None
        }
        Ok(record) => match Tx::try_from(&record) {
            Ok(tx) => Some(Ok(tx)),
            Err(err) => {
                log_skipped(record.position(), &err);
                None
            }
        },
    }
}

/// CSV reader configured for the transaction format, over any byte source
/// (a file, stdin, a TCP stream).
pub fn reader<R: Read>(source: R) -> Reader<R> {
    ReaderBuilder::new()
        .has_headers(true)
        .trim(Trim::All)
        .flexible(true)
        .from_reader(source)
}

pub fn file_reader(input_file: &Path) -> Result<Reader<File>, csv::Error> {
    Ok(reader(File::open(input_file)?))
}

/// Streams transactions row by row. Malformed rows are skipped; an I/O error
/// is yielded as `Err` and ends the stream.
pub fn read_transactions<R: Read>(
    reader: Reader<R>,
) -> impl Iterator<Item = Result<Tx, csv::Error>> {
    reader.into_records().filter_map(parse_record)
}
