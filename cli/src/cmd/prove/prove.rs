use std::fs::File;
use std::io::Write;

use super::ProveArgs;
use crate::{cmd::ProofData, utils::Cmd};
use air::ProofOptions;
use runner::ExecutionTrace;

pub struct ProveOutput {}

#[derive(Debug)]
pub enum Error {}

impl Cmd for ProveArgs {
    type Output = Result<ProveOutput, Error>;

    fn run(self) -> Self::Output {
        // Load trace from file
        let trace =
            ExecutionTrace::from_file(self.program, self.trace, self.memory, self.num_outputs);

        // Generate proof
        let proof_options = ProofOptions::with_proof_options(
            self.num_queries,
            self.blowup_factor,
            self.grinding_factor,
            self.fri_folding_factor,
            self.fri_max_remainder_size,
        );
        let (proof, pub_inputs) = prover::prove_trace(trace, &proof_options).unwrap();
        let input_bytes = pub_inputs.to_bytes();
        let proof_bytes = proof.to_bytes();
        println!("Proof size: {:.1} KB", proof_bytes.len() as f64 / 1024f64);

        // Write proof to disk
        let data = ProofData {
            input_bytes,
            proof_bytes,
        };
        let b = bincode::serialize(&data).unwrap();
        let mut f = File::create(self.output).unwrap();
        f.write_all(&b).unwrap();

        Ok(ProveOutput {})
    }
}
