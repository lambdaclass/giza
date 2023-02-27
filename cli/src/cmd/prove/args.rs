use clap::{Error, ErrorKind, Parser, ValueHint};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
pub struct ProveArgs {
    #[clap(
        help = "Path to the compiled Cairo program JSON file",
        long,
        value_hint = ValueHint::FilePath
    )]
    pub program: PathBuf,

    #[clap(
        help = "Path to the execution trace output file",
        long,
        value_hint = ValueHint::FilePath
    )]
    pub trace: PathBuf,

    #[clap(
        help = "Path to the memory output file",
        long,
        value_hint = ValueHint::FilePath
    )]
    pub memory: PathBuf,

    #[clap(
        help = "Path to write the STARK proof",
        long,
        value_hint = ValueHint::FilePath
    )]
    pub output: PathBuf,

    #[clap(help = "Number of serialized outputs", long)]
    pub num_outputs: Option<u64>,

    #[clap(
        help = "Number of queries for a STARK proof",
        long,
        value_parser(clap::builder::ValueParser::new(parse_num_queries))
    )]
    pub num_queries: Option<usize>,

    #[clap(
        help = "Blowup factor for a STARK proof",
        long,
        value_parser(clap::builder::ValueParser::new(parse_blowup_factor))
    )]
    pub blowup_factor: Option<usize>,

    #[clap(
        help = "Query seed grinding factor for a STARK proof",
        long,
        value_parser(clap::value_parser!(u32).range(..33))
    )]
    pub grinding_factor: Option<u32>,

    #[clap(
        help = "Factor by which the degree of a polynomial is reduced with each FRI layer",
        long,
        value_parser(clap::builder::ValueParser::new(parse_fri_folding_factor))
    )]
    pub fri_folding_factor: Option<usize>,

    #[clap(
        help = "Maximum allowed remainder (last FRI layer) size",
        long,
        value_parser(clap::builder::ValueParser::new(parse_fri_max_remainder_size))
    )]
    pub fri_max_remainder_size: Option<usize>,
}

fn parse_num_queries(value: &str) -> Result<usize, Error> {
    let value = value
        .parse::<usize>()
        .map_err(|e| Error::raw(ErrorKind::InvalidValue, format!("{}", e)))?;

    match value {
        0 => Err(Error::raw(ErrorKind::ValueValidation, "cannot be 0")),
        129.. => Err(Error::raw(
            ErrorKind::ValueValidation,
            "cannot be more than 128",
        )),
        _ => Ok(value),
    }
}

fn parse_blowup_factor(value: &str) -> Result<usize, Error> {
    let value = value
        .parse::<usize>()
        .map_err(|e| Error::raw(ErrorKind::InvalidValue, format!("{}", e)))?;

    if !value.is_power_of_two() {
        return Err(Error::raw(
            ErrorKind::ValueValidation,
            "must be a power of two",
        ));
    }

    match value {
        0..=3 => Err(Error::raw(
            ErrorKind::ValueValidation,
            "cannot be smaller than 4",
        )),
        257.. => Err(Error::raw(
            ErrorKind::ValueValidation,
            "cannot be more than 256",
        )),
        _ => Ok(value),
    }
}

fn parse_fri_folding_factor(value: &str) -> Result<usize, Error> {
    let value = value
        .parse::<usize>()
        .map_err(|e| Error::raw(ErrorKind::InvalidValue, format!("{}", e)))?;

    if value != 2 && value != 4 && value != 8 && value != 16 {
        Err(Error::raw(ErrorKind::ValueValidation, "must be 4, 8 or 16"))
    } else {
        Ok(value)
    }
}

fn parse_fri_max_remainder_size(value: &str) -> Result<usize, Error> {
    let value = value
        .parse::<usize>()
        .map_err(|e| Error::raw(ErrorKind::InvalidValue, format!("{}", e)))?;

    if !value.is_power_of_two() {
        return Err(Error::raw(
            ErrorKind::ValueValidation,
            "must be a power of two",
        ));
    }

    match value {
        0..=31 => Err(Error::raw(
            ErrorKind::ValueValidation,
            "cannot be smaller than 32",
        )),
        1025.. => Err(Error::raw(
            ErrorKind::ValueValidation,
            "cannot be more than 1024",
        )),
        _ => Ok(value),
    }
}
