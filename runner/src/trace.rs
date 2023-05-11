use crate::cairo_interop::{read_builtins, read_memory_bin, read_trace_bin};
use crate::memory::Memory;
use crate::runner::{State, Step};
use giza_core::{
    Builtin, Felt, FieldElement, StarkField, Word, AP, A_M_PRIME_WIDTH, A_RC_PRIME_WIDTH,
    MEM_A_TRACE_RANGE, MEM_A_TRACE_WIDTH, MEM_V_TRACE_RANGE, OFF_X_TRACE_RANGE, OFF_X_TRACE_WIDTH,
    P_M_WIDTH, P_RC_WIDTH, TRACE_WIDTH, V_M_PRIME_WIDTH,
};
use winterfell::{Matrix, Trace, TraceLayout};

use indicatif::ParallelProgressIterator;
use indicatif::ProgressIterator;
use itertools::Itertools;
use rayon::prelude::*;
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct ExecutionTrace {
    layout: TraceLayout,
    meta: Vec<u8>,
    pub trace: Matrix<Felt>,
    pub memory: Memory,
    pub rc_min: u16,
    pub rc_max: u16,
    pub num_steps: usize,
    pub builtins: Vec<Builtin>,
}

/// A virtual column is composed of one or more subcolumns.
#[derive(Debug)]
struct VirtualColumn<'a, E: FieldElement> {
    subcols: &'a [Vec<E>],
}

impl<'a, E: FieldElement> VirtualColumn<'a, E> {
    fn new(subcols: &'a [Vec<E>]) -> Self {
        Self { subcols }
    }

    /// Pack subcolumns into a single output column: cycle through each subcolumn, appending
    /// a single value to the output column for each iteration step until exhausted.
    fn to_column(&self) -> Vec<E> {
        let mut col: Vec<E> = vec![];
        for n in 0..self.subcols[0].len() {
            for subcol in self.subcols {
                col.push(subcol[n]);
            }
        }
        col
    }

    /// Split subcolumns into multiple output columns: for each subcolumn, output a single
    /// value to each output column, cycling through each output column until exhuasted.
    fn to_columns(&self, num_rows: &[usize]) -> Vec<Vec<E>> {
        let mut n = 0;
        let mut cols: Vec<Vec<E>> = vec![vec![]; num_rows.iter().sum()];
        for (subcol, width) in self.subcols.iter().zip(num_rows) {
            for (elem, idx) in subcol.iter().zip((0..*width).cycle()) {
                cols[idx + n].push(*elem);
            }
            n += width;
        }
        cols
    }
}

struct Layouter<'a, E: FieldElement> {
    columns: &'a mut Vec<Vec<E>>,
    frame_len: usize,
}

impl<'a, E: FieldElement> Layouter<'a, E> {
    fn new(columns: &'a mut Vec<Vec<E>>, frame_len: usize) -> Self {
        Self { columns, frame_len }
    }

    /// Add one or more columns to the trace. The chunk size determines the number
    /// of subcolumn elements to place within each frame chunk (defaults to 1)
    /// starting from the top most row of the chunk.
    fn add_columns(&mut self, subcols: &[Vec<E>], chunk_size: Option<usize>) {
        for subcol in subcols.iter() {
            let mut col = E::zeroed_vector(subcol.len());
            for (col_chunk, subcol_chunk) in col
                .chunks_mut(self.frame_len)
                .zip(subcol.chunks(chunk_size.unwrap_or(1)))
            {
                for (n, elem) in subcol_chunk.iter().enumerate() {
                    col_chunk[n] = *elem
                }
            }
            self.columns.push(col);
        }
    }

    /// Resize columns to next power of two
    fn resize_all(&mut self) {
        resize_to_pow2(&mut self.columns);
    }
}

impl ExecutionTrace {
    /// Builds an execution trace
    pub(super) fn new(
        num_steps: usize,
        state: &mut State,
        memory: &Memory,
        builtins: Vec<Builtin>,
    ) -> Self {
        // Compute the derived ("auxiliary") trace values: t0, t1, and mul.
        // Note that in a conditional jump instruction we substitute res with dst^{-1}
        // (see page 53 of the whitepaper).
        let mut t0 = vec![];
        let mut t1 = vec![];
        let mut mul = vec![];
        for step in 0..num_steps {
            // TODO: Don't hardcode index values
            let f_pc_jnz = state.flags[9][step];
            let dst = state.mem_v[1][step];
            let res = state.res[0][step];
            t0.push(f_pc_jnz * dst); // f_pc_jnz * dst
            t1.push(t0[step] * res); // t_0 * res
            mul.push(state.mem_v[2][step] * state.mem_v[3][step]); // op0 * op1
                                                                   // println!("MUL: {:?}", state.mem_v[2][step] * state.mem_v[3][step]);
        }

        // 1. Append dummy artificial accesses to mem_a and mem_v to fill memory holes.
        //    These gaps are due to interaction with builtins, and they still need to be handled
        //    elsewhere in the code for soundness.
        // 2. Append dummy (0,0) public memory values to mem_a and mem_v.
        //    Note that we don't need to worry about precise placement (i.e. ensuring that they are
        //    the final n entries in the columns), because these dummy values will extend into the
        //    resized column cells.
        //    TODO: We should also append dummy output (not just program) public memory, in case the
        //    trace length is not already long enough to contain these values.
        let mut col_extension = memory.get_holes(VirtualColumn::new(&state.mem_a).to_column());
        // println!("MEMORY HOLES: {:?}", col_extension);
        col_extension.extend(vec![Felt::ZERO; memory.get_codelen()]);
        for (n, col) in VirtualColumn::new(&[col_extension])
            .to_columns(&[MEM_A_TRACE_WIDTH])
            .iter()
            .enumerate()
        {
            state.mem_a[n].extend(col);

            state.mem_v[n].extend(Felt::zeroed_vector(col.len()));
        }

        // 1. Convert offsets into an unbiased representation by adding 2^15, so that values are
        //    within [0, 2^16].
        // 2. Fill gaps between sorted offsets so that we can compute the proper permutation
        //    product column in the range check auxiliary segment (if we implemented Ord for Felt
        //    we could achieve a speedup here)
        // println!("OFFSETS BEFORE: {:?}", state.offsets);
        let b15 = Felt::from(2u8).exp(15u32.into());
        let mut rc_column: Vec<Felt> = VirtualColumn::new(&state.offsets)
            .to_column()
            .into_iter()
            .map(|x| x + b15)
            .collect();
        // println!("OFFSETS AFTER: {:?}", rc_column);
        let mut rc_sorted: Vec<u16> = rc_column
            .iter()
            .map(|x| x.as_int().try_into().unwrap())
            .collect();
        rc_sorted.sort_unstable();
        let rc_min = rc_sorted.first().unwrap().clone();
        let rc_max = rc_sorted.last().unwrap().clone();
        for s in rc_sorted.windows(2).progress() {
            match s[1] - s[0] {
                0 | 1 => {}
                _ => {
                    // println!("LALA");
                    rc_column.extend((s[0] + 1..s[1]).map(|x| Felt::from(x)).collect::<Vec<_>>());
                }
            }
        }
        let offsets = VirtualColumn::new(&[rc_column]).to_columns(&[3]);
        // println!("OFFSETS: {:?}", offsets);

        // This is hacky... We're adding a selector to the main trace to disable the Cairo
        // transition constraints for public memory (and any extended trace cells that were added
        // to ensure that that length is a power of two). If we instead used transition
        // exemptions, then proving/verifying time would be too slow for programs with a large
        // number of instructions.
        //
        // There are two methods that can be combined to avoid selectors:
        // - Transform traces so that they end in an inifite loop (use the instruction
        //   0x10780017fff7fffu64).
        // - Use a short bootloader program so that the number of transition exemptions is small
        //   and doesn't harm performance. This bootloader will compute and expose a hash of the
        //   private memory containing the program instructions to be run.
        let mut selector = vec![Felt::ONE; num_steps];
        selector[num_steps - 1] = Felt::ZERO;

        // Layout the trace
        let mut columns: Vec<Vec<Felt>> = Vec::with_capacity(TRACE_WIDTH);
        let mut layouter = Layouter::new(&mut columns, 1);
        layouter.add_columns(&state.flags, None);
        layouter.add_columns(&state.res, None);
        layouter.add_columns(&state.mem_p, None);
        layouter.add_columns(&state.mem_a, None);
        layouter.add_columns(&state.mem_v, None);
        layouter.add_columns(&offsets, None);
        layouter.add_columns(&[t0, t1, mul], None);
        layouter.add_columns(&[selector], None);

        layouter.resize_all();

        Self {
            layout: TraceLayout::new(
                TRACE_WIDTH,
                &[12, 6], // aux_segment widths
                &[2, 1],  // aux_segment rands
            ),
            meta: Vec::new(),
            trace: Matrix::new(columns),
            memory: memory.clone(),
            rc_min,
            rc_max,
            num_steps,
            builtins,
        }
    }

    /// Reconstructs the execution trace from file
    pub fn from_file(
        program_path: PathBuf,
        trace_path: PathBuf,
        memory_path: PathBuf,
        output_len: Option<u64>,
    ) -> ExecutionTrace {
        let mem = read_memory_bin(&memory_path, &program_path);
        let registers = read_trace_bin(&trace_path);

        // println!("REGISTERS: {:?}", registers);

        let builtins = read_builtins(&program_path, output_len);
        let num_steps = registers.len();

        // println!("MEMORY: {:?}", mem);

        let inst_states = registers
            .par_iter()
            .progress()
            .map(|ptrs| {
                let mut step = Step::new(&mem, *ptrs);
                step.execute(false)
            })
            .collect::<Vec<_>>();

        // println!("INSTRUCTION STATES: {:?}", inst_states);

        // Va llenando el state
        let mut state = State::new(registers.len() + 1);
        for (n, (reg_state, inst_state)) in registers.iter().zip(inst_states).enumerate() {
            state.set_register_state(n, *reg_state);
            state.set_instruction_state(n, inst_state);
        }

        Self::new(num_steps, &mut state, &mem, builtins)
    }

    /// Return the program public memory
    pub fn get_program_mem(&self) -> (Vec<u64>, Vec<Option<Word>>) {
        let addrs = (0..self.memory.get_codelen() as u64).collect::<Vec<_>>();
        let vals = self.memory.data[..self.memory.get_codelen()].to_vec();
        (addrs, vals)
    }

    /// Return the output public memory
    pub fn get_output_mem(&self) -> (Vec<u64>, Vec<Option<Word>>) {
        for builtin in self.builtins.iter() {
            if let Builtin::Output(len) = builtin {
                let ptr_start: u64 = self.main_segment().get_column(AP)[self.num_steps - 1]
                    .as_int()
                    .try_into()
                    .unwrap();
                let ptr_end = ptr_start + len;
                let addrs = (ptr_start..ptr_end).collect::<Vec<_>>();
                let vals = addrs
                    .iter()
                    .map(|i| self.memory.data[*i as usize])
                    .collect::<Vec<_>>();
                return (addrs, vals);
            }
        }
        return (vec![], vec![]);
    }

    /// Return the combined public memory
    pub fn get_public_mem(&self) -> (Vec<u64>, Vec<Option<Word>>) {
        let (mut a, mut v) = self.get_program_mem();
        let (out_a, out_v) = self.get_output_mem();
        a.extend(out_a);
        v.extend(out_v);
        (a, v)
    }
}

impl Trace for ExecutionTrace {
    type BaseField = Felt;

    fn layout(&self) -> &TraceLayout {
        &self.layout
    }

    fn length(&self) -> usize {
        self.trace.num_rows()
    }

    fn meta(&self) -> &[u8] {
        &self.meta
    }

    fn main_segment(&self) -> &Matrix<Felt> {
        &self.trace
    }

    fn build_aux_segment<E>(
        &mut self,
        aux_segments: &[Matrix<E>],
        rand_elements: &[E],
    ) -> Option<Matrix<E>>
    where
        E: FieldElement<BaseField = Self::BaseField>,
    {
        match aux_segments.len() {
            0 => build_aux_segment_mem(self, rand_elements),
            1 => build_aux_segment_rc(self, rand_elements),
            _ => None,
        }
    }
}

/// Write documentation
fn build_aux_segment_mem<E>(trace: &ExecutionTrace, rand_elements: &[E]) -> Option<Matrix<E>>
where
    E: FieldElement + From<Felt>,
{
    let z = rand_elements[0];
    let alpha = rand_elements[1];

    // Pack main memory access trace columns into two virtual columns
    let main = trace.main_segment();
    let (a, v) = [MEM_A_TRACE_RANGE, MEM_V_TRACE_RANGE]
        .iter()
        .map(|range| {
            VirtualColumn::new(
                &range
                    .clone()
                    .map(|i| main.get_column(i).to_vec())
                    .collect::<Vec<_>>()[..],
            )
            .to_column()
        })
        .collect_tuple()
        .unwrap();

    // Construct duplicate virtual columns sorted by memory access, with dummy public
    // memory addresses/values replaced by their true values
    let mut a_prime = vec![E::ZERO; a.len()];
    let mut v_prime = vec![E::ZERO; a.len()];
    let mut a_replaced = a.clone();
    let mut v_replaced = v.clone();

    let (pub_a, pub_v) = trace.get_public_mem();
    let l = a.len() - pub_a.len() - 1;
    for (i, (n, x)) in pub_a.iter().copied().zip(pub_v).enumerate() {
        a_replaced[l + i] = Felt::from(n);
        v_replaced[l + i] = x.unwrap().word().into();
    }
    let mut indices = (0..a.len()).collect::<Vec<_>>();
    indices.sort_by_key(|&i| a_replaced[i].as_int());
    for (i, j) in indices.iter().copied().enumerate() {
        a_prime[i] = a_replaced[j].into();
        v_prime[i] = v_replaced[j].into();
    }

    // Construct virtual column of computed permutation products
    let mut p = vec![E::ZERO; trace.length() * MEM_A_TRACE_WIDTH];
    let a_0: E = a[0].into();
    let v_0: E = v[0].into();
    p[0] = (z - (a_0 + alpha * v_0).into()) / (z - (a_prime[0] + alpha * v_prime[0]).into());
    for i in (1..p.len()).progress() {
        let a_i: E = a[i].into();
        let v_i: E = v[i].into();
        p[i] = (z - (a_i + alpha * v_i).into()) * p[i - 1]
            / (z - (a_prime[i] + alpha * v_prime[i]).into());
    }

    println!("# RANDOM ELEMENTS - AUXILIARY SEGMENTS");
    println!("Z: {:?}", z);
    println!("ALPHA: {:?}", alpha);

    println!("#######################################");

    println!("A PRIME: {:?}", a_prime);
    println!("V PRIME: {:?}", v_prime);
    println!("P: {:?}", p);

    // Split virtual columns into separate auxiliary columns
    let mut aux_columns = VirtualColumn::new(&[a_prime, v_prime, p]).to_columns(&[
        A_M_PRIME_WIDTH,
        V_M_PRIME_WIDTH,
        P_M_WIDTH,
    ]);
    resize_to_pow2(&mut aux_columns);

    Some(Matrix::new(aux_columns))
}

/// Write documentation
fn build_aux_segment_rc<E>(trace: &ExecutionTrace, rand_elements: &[E]) -> Option<Matrix<E>>
where
    E: FieldElement + From<Felt>,
{
    let z = rand_elements[0];

    // Pack main offset trace columns into a single virtual column
    let main = trace.main_segment();
    let a = VirtualColumn::new(
        &OFF_X_TRACE_RANGE
            .map(|i| main.get_column(i).to_vec())
            .collect::<Vec<_>>()[..],
    )
    .to_column();

    // Construct duplicate virtual column sorted by offset value
    let mut indices = (0..a.len()).collect::<Vec<_>>();
    indices.sort_by_key(|&i| a[i].as_int());
    let a_prime = indices.iter().map(|x| a[*x].into()).collect::<Vec<E>>();

    // Construct virtual column of computed permutation products
    let mut p = vec![E::ZERO; trace.length() * OFF_X_TRACE_WIDTH];
    let a_0: E = a[0].into();
    p[0] = (z - a_0) / (z - a_prime[0]);
    for i in (1..p.len()).progress() {
        let a_i: E = a[i].into();
        p[i] = (z - a_i) * p[i - 1] / (z - a_prime[i]);
    }

    // Split virtual columns into separate auxiliary columns
    let mut aux_columns =
        VirtualColumn::new(&[a_prime, p]).to_columns(&[A_RC_PRIME_WIDTH, P_RC_WIDTH]);
    resize_to_pow2(&mut aux_columns);

    Some(Matrix::new(aux_columns))
}

/// Resize columns to next power of two
fn resize_to_pow2<E: FieldElement>(columns: &mut [Vec<E>]) {
    let trace_len_pow2 = columns
        .iter()
        .map(|x| x.len().next_power_of_two())
        .max()
        .unwrap();
    for column in columns.iter_mut() {
        // println!("TRACE COLUMNS BEFORE RESIZE POWER OF TWO: {:?}", column);
        let last_value = column.last().copied().unwrap();
        column.resize(trace_len_pow2, last_value);
        // println!("TRACE COLUMNS AFTER RESIZE POWER OF TWO: {:?}", column);
    }
}

#[cfg(test)]
mod test {
    use winterfell::math::fields::f128::BaseElement;

    use super::*;

    #[test]
    fn virtual_col_to_colums() {
        let mut inner = vec![BaseElement::ZERO; 2];
        inner.extend_from_slice(&[BaseElement::ONE; 3]);

        let col = vec![inner];

        let virtual_col = VirtualColumn::new(&col);

        println!("Virtual Col: {:?}", virtual_col);
        println!("Col: {:?}", virtual_col.to_columns(&[4]));
    }

    #[test]
    fn sort_test() {
        let a = vec![0, 3, 1, 5, 2, 4];
        let mut indices = (0..a.len()).collect::<Vec<_>>();
        indices.sort_by_key(|&i| a[i]);

        println!("{:?}", indices);
    }
}
