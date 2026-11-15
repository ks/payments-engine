//! Stream-level behaviour of the input reader that the CLI tests cannot reach:
//! what happens when the byte source itself fails, as a dropped TCP connection would.

use std::io::{self, Read};

use payments_engine::input::{read_transactions, reader};

/// Serves `data`, then fails every read, like a connection reset mid-stream.
struct FailsAfter {
    data: &'static [u8],
    pos: usize,
}

impl Read for FailsAfter {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.pos == self.data.len() {
            return Err(io::Error::new(io::ErrorKind::ConnectionReset, "reset"));
        }
        let n = buf.len().min(self.data.len() - self.pos);
        buf[..n].copy_from_slice(&self.data[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

#[test]
fn io_error_is_yielded_and_ends_the_stream() {
    let source = FailsAfter {
        data: b"type,client,tx,amount\ndeposit,1,1,1.0\nnot,a,valid,row\n",
        pos: 0,
    };
    let items: Vec<_> = read_transactions(reader(source)).collect();

    assert_eq!(
        items.len(),
        2,
        "one transaction, then the I/O error: {items:?}"
    );
    assert!(items[0].is_ok());
    assert!(items[1].as_ref().is_err_and(|e| e.is_io_error()));
}

#[test]
fn malformed_rows_are_skipped_not_yielded() {
    let source: &[u8] =
        b"type,client,tx,amount\ndeposit,1,1,1.0\nnot,a,valid,row\ndeposit,1,2,2.0\n";
    let items: Vec<_> = read_transactions(reader(source)).collect();

    assert_eq!(items.len(), 2, "{items:?}");
    assert!(items.iter().all(Result::is_ok));
}
