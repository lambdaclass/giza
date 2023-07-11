use std::fs::File;
use std::io::Read;
use std::path::PathBuf;

use super::ProofData;
use crate::utils::Cmd;
use air::{ProcessorAir, PublicInputs};
use clap::{Parser, ValueHint};
use winter_utils::{Deserializable, SliceReader};
use winterfell::StarkProof;

pub struct VerifyOutput {}

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
pub struct VerifyArgs {
    #[clap(
        help = "Path to the STARK proof",
        long,
        value_hint = ValueHint::FilePath
    )]
    pub proof: PathBuf,
}

#[derive(Debug)]
pub enum Error {}

impl Cmd for VerifyArgs {
    type Output = Result<VerifyOutput, Error>;

    fn run(self) -> Self::Output {
        // Load proof and public inputs from file
        let mut b = Vec::new();
        let mut f = File::open(self.proof).unwrap();
        f.read_to_end(&mut b).unwrap();
        let data: ProofData = bincode::deserialize(&b).unwrap();
        let pub_inputs =
            PublicInputs::read_from(&mut SliceReader::new(&data.input_bytes[..])).unwrap();
        let proof = StarkProof::from_bytes(&data.proof_bytes).unwrap();

        // Verify execution
        let _ = winterfell::verify::<ProcessorAir>(proof, pub_inputs);

        Ok(VerifyOutput {})
    }
}
