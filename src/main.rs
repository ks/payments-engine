use std::error::Error;
use std::path::PathBuf;
use std::process::ExitCode;

use payments_engine::{Engine, input, output};

fn main() -> Result<ExitCode, Box<dyn Error>> {
    let Some(input_file) = std::env::args_os().nth(1).map(PathBuf::from) else {
        eprintln!("usage: payments-engine <transactions.csv>");
        return Ok(ExitCode::from(2));
    };

    let reader = input::file_reader(input_file.as_path())?;
    let transactions = input::read_transactions(reader);

    let mut engine = Engine::default();

    for tx in transactions {
        let tx = tx?;
        let res = engine.process(&tx);

        if cfg!(feature = "verbose") {
            eprintln!("processing {tx:?} -> {res:?}");
        }
    }

    output::write_accounts(std::io::stdout(), engine.accounts_iter())?;

    Ok(ExitCode::SUCCESS)
}
