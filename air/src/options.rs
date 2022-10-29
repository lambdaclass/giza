use core::ops::Deref;
use winter_air::{FieldExtension, HashFunction, ProofOptions as WinterProofOptions};

/// TODO: add docs
#[derive(Clone)]
pub struct ProofOptions(WinterProofOptions);

impl ProofOptions {
    pub fn new(
        num_queries: usize,
        blowup_factor: usize,
        grinding_factor: u32,
        hash_fn: HashFunction,
        field_extension: FieldExtension,
        fri_folding_factor: usize,
        fri_max_remainder_size: usize,
    ) -> Self {
        Self(WinterProofOptions::new(
            num_queries,
            blowup_factor,
            grinding_factor,
            hash_fn,
            field_extension,
            fri_folding_factor,
            fri_max_remainder_size,
        ))
    }

    pub fn with_proof_options(
        num_queries: Option<usize>,
        blowup_factor: Option<usize>,
        grinding_factor: Option<u32>,
        fri_folding_factor: Option<usize>,
        fri_max_remainder_size: Option<usize>,
    ) -> Self {
        Self(WinterProofOptions::new(
            num_queries.unwrap_or(54),  // 27
            blowup_factor.unwrap_or(4), //8,
            grinding_factor.unwrap_or(16),
            HashFunction::Blake2s_256,
            FieldExtension::None,
            fri_folding_factor.unwrap_or(8),
            fri_max_remainder_size.unwrap_or(256),
        ))
    }

    pub fn into_inner(self) -> WinterProofOptions {
        self.0
    }
}

impl Default for ProofOptions {
    fn default() -> Self {
        Self::with_proof_options(None, None, None, None, None)
    }
}

impl Deref for ProofOptions {
    type Target = WinterProofOptions;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
