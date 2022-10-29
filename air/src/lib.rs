#![feature(generic_associated_types)]

use giza_core::{
    Builtin, ExtensionOf, Felt, FieldElement, RegisterState, Word, A_RC_PRIME_FIRST,
    A_RC_PRIME_LAST, MEM_A_TRACE_OFFSET, MEM_P_TRACE_OFFSET, P_M_LAST,
};
use starknet_crypto::pedersen_hash;
use starknet_ff::FieldElement as StarknetFieldElement;
use winter_air::{
    Air, AirContext, Assertion, AuxTraceRandElements, ProofOptions as WinterProofOptions,
    TraceInfo, TransitionConstraintDegree,
};
use winter_crypto::{hashers::Blake2s_256, ElementHasher};
use winter_utils::{ByteReader, ByteWriter, Deserializable, DeserializationError, Serializable};

// EXPORTS
// ================================================================================================

pub use winter_air::{EvaluationFrame, FieldExtension, HashFunction};

mod options;
pub use options::ProofOptions;

mod constraints;
use constraints::{AuxEvaluationResult, EvaluationResult};

mod frame;
pub use frame::{AuxEvaluationFrame, MainEvaluationFrame};

// PROCESSOR AIR
// ================================================================================================

pub struct ProcessorAir {
    context: AirContext<Felt>,
    pub_inputs: PublicInputs,
}

impl Air for ProcessorAir {
    type BaseField = Felt;
    type PublicInputs = PublicInputs;
    type Frame<E: FieldElement> = MainEvaluationFrame<E>;
    type AuxFrame<E: FieldElement> = AuxEvaluationFrame<E>;

    fn new(trace_info: TraceInfo, pub_inputs: PublicInputs, options: WinterProofOptions) -> Self {
        let mut main_degrees = vec![];
        // Instruction constraints
        for _ in 0..=14 {
            main_degrees.push(TransitionConstraintDegree::new(2)); // F0-F14
        }
        main_degrees.push(TransitionConstraintDegree::new(1)); // F15

        // Operand constraints
        main_degrees.push(TransitionConstraintDegree::new(4)); // INST
        main_degrees.push(TransitionConstraintDegree::new(4)); // DST_ADDR
        main_degrees.push(TransitionConstraintDegree::new(4)); // OP0_ADDR
        main_degrees.push(TransitionConstraintDegree::new(4)); // OP1_ADDR

        // Register constraints
        main_degrees.push(TransitionConstraintDegree::new(4)); // NEXT_AP
        main_degrees.push(TransitionConstraintDegree::new(4)); // NEXT_FP
        main_degrees.push(TransitionConstraintDegree::new(4)); // NEXT_PC_1
        main_degrees.push(TransitionConstraintDegree::new(4)); // NEXT_PC_2
        main_degrees.push(TransitionConstraintDegree::new(4)); // T0
        main_degrees.push(TransitionConstraintDegree::new(4)); // T1

        // Opcode constraints
        main_degrees.push(TransitionConstraintDegree::new(4)); // MUL_1
        main_degrees.push(TransitionConstraintDegree::new(4)); // MUL_2
        main_degrees.push(TransitionConstraintDegree::new(4)); // CALL_1
        main_degrees.push(TransitionConstraintDegree::new(4)); // CALL_2
        main_degrees.push(TransitionConstraintDegree::new(4)); // ASSERT_EQ

        let aux_degrees = vec![
            // Memory constraints
            TransitionConstraintDegree::new(2), // A_M_PRIME 0
            TransitionConstraintDegree::new(2), //     "     1
            TransitionConstraintDegree::new(2), //     "     2
            TransitionConstraintDegree::new(2), //     "     3
            TransitionConstraintDegree::new(2), // V_M_PRIME 0
            TransitionConstraintDegree::new(2), //     "     1
            TransitionConstraintDegree::new(2), //     "     2
            TransitionConstraintDegree::new(2), //     "     3
            TransitionConstraintDegree::new(2), //    P_M    0
            TransitionConstraintDegree::new(2), //     "     1
            TransitionConstraintDegree::new(2), //     "     2
            TransitionConstraintDegree::new(2), //     "     3
            // Range check constraints
            TransitionConstraintDegree::new(2), // A_RC_PRIME 0
            TransitionConstraintDegree::new(2), //     "      1
            TransitionConstraintDegree::new(2), //     "      2
            TransitionConstraintDegree::new(2), //    P_RC    0
            TransitionConstraintDegree::new(2), //     "      1
            TransitionConstraintDegree::new(2), //     "      2
        ];

        let mut transition_exemptions = vec![];
        transition_exemptions.extend(vec![1; main_degrees.len()]);
        transition_exemptions.extend(vec![1; aux_degrees.len()]);

        let mut context =
            AirContext::new_multi_segment(trace_info, main_degrees, aux_degrees, 4, 3, options);
        context.set_transition_exemptions(transition_exemptions);

        Self {
            context,
            pub_inputs,
        }
    }

    fn get_assertions(&self) -> Vec<Assertion<Felt>> {
        let last_step = self.pub_inputs.num_steps - 1;
        vec![
            // Initial and final 'pc' register
            Assertion::single(MEM_A_TRACE_OFFSET, 0, self.pub_inputs.init.pc),
            Assertion::single(MEM_A_TRACE_OFFSET, last_step, self.pub_inputs.fin.pc),
            // Initial and final 'ap' register
            Assertion::single(MEM_P_TRACE_OFFSET, 0, self.pub_inputs.init.ap),
            Assertion::single(MEM_P_TRACE_OFFSET, last_step, self.pub_inputs.fin.ap),
        ]
    }

    fn get_aux_assertions<E: FieldElement + From<Self::BaseField>>(
        &self,
        aux_rand_elements: &AuxTraceRandElements<E>,
    ) -> Vec<Assertion<E>> {
        let last_step = self.trace_length() - 1;
        let random_elements = aux_rand_elements.get_segment_elements(0);
        let mem = &self.pub_inputs.mem;
        let z = random_elements[0];
        let alpha = random_elements[1];
        let num = z.exp((mem.0.len() as u64).into());

        let den = mem
            .0
            .iter()
            .zip(&mem.1)
            .map(|(a, v)| z - (E::from(*a as u64) + alpha * E::from(v.unwrap().word())))
            .reduce(|a, b| a * b)
            .unwrap();

        vec![
            // Public memory
            Assertion::single(P_M_LAST, last_step, num / den),
            // Minimum range check value
            Assertion::single(A_RC_PRIME_FIRST, 0, E::from(self.pub_inputs.rc_min)),
            // Maximum range check value
            Assertion::single(A_RC_PRIME_LAST, last_step, E::from(self.pub_inputs.rc_max)),
        ]
    }

    fn evaluate_transition<E: FieldElement + From<Felt>>(
        &self,
        frame: &MainEvaluationFrame<E>,
        _periodic_values: &[E],
        result: &mut [E],
    ) {
        result.evaluate_instr_constraints(frame);
        result.evaluate_operand_constraints(frame);
        result.evaluate_register_constraints(frame);
        result.evaluate_opcode_constraints(frame);
        result.enforce_selector(frame);
    }

    fn evaluate_aux_transition<
        E: FieldElement + From<Felt>,
        F: FieldElement + From<Felt> + ExtensionOf<E>,
    >(
        &self,
        main_frame: &MainEvaluationFrame<E>,
        aux_frame: &AuxEvaluationFrame<F>,
        _periodic_values: &[E],
        aux_rand_elements: &AuxTraceRandElements<F>,
        result: &mut [F],
    ) {
        result.evaluate_memory_constraints(main_frame, aux_frame, aux_rand_elements);
        result.evaluate_range_check_constraints(main_frame, aux_frame, aux_rand_elements);
    }

    fn context(&self) -> &AirContext<Felt> {
        &self.context
    }
}

// PUBLIC INPUTS
// ================================================================================================

pub struct PublicInputs {
    pub init: RegisterState,                // initial register state
    pub fin: RegisterState,                 // final register state
    pub rc_min: u16, // minimum range check value (0 < rc_min < rc_max < 2^16)
    pub rc_max: u16, // maximum range check value
    pub mem: (Vec<u64>, Vec<Option<Word>>), // public memory
    pub num_steps: usize, // number of execution steps
    pub builtins: Vec<Builtin>, // list of builtins
}

impl PublicInputs {
    pub fn new(
        init: RegisterState,
        fin: RegisterState,
        rc_min: u16,
        rc_max: u16,
        mem: (Vec<u64>, Vec<Option<Word>>),
        num_steps: usize,
        builtins: Vec<Builtin>,
    ) -> Self {
        Self {
            init,
            fin,
            rc_min,
            rc_max,
            mem,
            num_steps,
            builtins,
        }
    }

    /// Serializes PublicInputs into a vector of bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        // TODO: Implement Serializable/Deserializable traits in RegisterState and Memory
        // structs instead of manually managing it here
        let mut result = Vec::new();
        result.write(self.init.pc);
        result.write(self.init.ap);
        result.write(self.init.fp);
        result.write(self.fin.pc);
        result.write(self.fin.ap);
        result.write(self.fin.fp);
        result.write_u16(self.rc_min);
        result.write_u16(self.rc_max);
        result.write_u64(self.mem.1.len() as u64);
        for i in 0..self.mem.1.len() as usize {
            result.write_u64(self.mem.0[i]);
        }
        result.write(
            self.mem
                .1
                .iter()
                .map(|x| x.unwrap().word())
                .collect::<Vec<_>>(),
        );
        result.write_u64(self.num_steps as u64);
        // TODO: Use bit representation once multiple builtins are supported
        for builtin in self.builtins.iter() {
            if let Builtin::Output(_) = builtin {
                result.write_u8(1);
            } else {
                result.write_u8(0);
            }
        }
        result
    }
}

/// Serializes PublicInputs into a digest used to efficiently seed the public coin
impl Serializable for PublicInputs {
    fn write_into<W: ByteWriter>(&self, target: &mut W) {
        let mut data = Vec::new();
        let pub_mem_hash = self
            .mem
            .1
            .iter()
            .map(|x| StarknetFieldElement::from_mont(x.unwrap().word().to_raw().0))
            .reduce(|hash, item| pedersen_hash(&hash, &item))
            .unwrap()
            .to_bytes_be();

        data.push(self.init.pc);
        data.push(self.init.ap);
        data.push(self.init.fp);

        data.push(self.fin.pc);
        data.push(self.fin.ap);
        data.push(self.fin.fp);

        data.push(self.rc_min.into());
        data.push(self.rc_max.into());

        data.push(self.mem.0.len().into());
        data.push(pub_mem_hash.into());

        data.push(self.num_steps.into());

        let hash = Blake2s_256::hash_elements(&data);
        target.write(hash);
    }
}

impl Deserializable for PublicInputs {
    fn read_from<R: ByteReader>(source: &mut R) -> Result<Self, DeserializationError> {
        let init = RegisterState::new(
            Felt::read_from(source)?,
            Felt::read_from(source)?,
            Felt::read_from(source)?,
        );
        let fin = RegisterState::new(
            Felt::read_from(source)?,
            Felt::read_from(source)?,
            Felt::read_from(source)?,
        );
        let rc_min = source.read_u16()?;
        let rc_max = source.read_u16()?;
        let mem_len = source.read_u64()?;
        let mut mem_a = vec![0u64; mem_len as usize];
        for i in 0..mem_len as usize {
            mem_a[i] = source.read_u64()?;
        }
        let mem_v = Felt::read_batch_from(source, mem_len as usize)?
            .into_iter()
            .map(|x| Some(Word::new(x)))
            .collect::<Vec<_>>();
        let num_steps = source.read_u64()?;
        // TODO: Interpret as bits once multiple builtins are supported
        let builtins = match source.read_u8()? {
            1 => vec![Builtin::Output(0)],
            _ => vec![],
        };
        Ok(PublicInputs::new(
            init,
            fin,
            rc_min,
            rc_max,
            (mem_a, mem_v),
            num_steps as usize,
            builtins,
        ))
    }
}
