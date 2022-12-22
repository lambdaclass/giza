use super::FieldElement;
use giza_core::{flags::*, *};
use winter_air::{Air, EvaluationFrame, Table};
use winter_utils::TableReader;

// MAIN FRAME
// --------------------------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct MainEvaluationFrame<E: FieldElement> {
    table: Table<E>, // row-major indexing
}

impl<E: FieldElement> EvaluationFrame<E> for MainEvaluationFrame<E> {
    // CONSTRUCTORS
    // --------------------------------------------------------------------------------------------

    fn new<A: Air>(air: &A) -> Self {
        let num_cols = air.trace_layout().main_trace_width();
        let num_rows = Self::num_rows();
        MainEvaluationFrame {
            table: Table::new(num_rows, num_cols),
        }
    }

    fn from_table(table: Table<E>) -> Self {
        Self { table }
    }

    // ROW MUTATORS
    // --------------------------------------------------------------------------------------------

    fn read_from<R: TableReader<E>>(
        &mut self,
        data: R,
        step: usize,
        _offset: usize,
        blowup: usize,
    ) {
        let trace_len = data.num_rows();
        for (row, row_idx) in self.table.rows_mut().zip(Self::offsets().into_iter()) {
            for col_idx in 0..data.num_cols() {
                row[col_idx] = data.get(col_idx, (step + row_idx * blowup) % trace_len);
            }
        }
    }

    // ROW ACCESSORS
    // --------------------------------------------------------------------------------------------

    fn row<'a>(&'a self, row_idx: usize) -> &'a [E] {
        &self.table.get_row(row_idx)
    }

    fn to_table(&self) -> Table<E> {
        self.table.clone()
    }

    fn offsets() -> &'static [usize] {
        &[0, 1]
    }
}

impl<'a, E: FieldElement> MainEvaluationFrame<E> {
    pub fn current(&'a self) -> MainFrameSegment<'a, E> {
        MainFrameSegment::new(&self.table, 0)
    }
    pub fn next(&'a self) -> MainFrameSegment<'a, E> {
        MainFrameSegment::new(&self.table, 1)
    }
    pub fn segment(&'a self) -> MainFrameSegment<'a, E> {
        MainFrameSegment::new(&self.table, 0)
    }
}

pub struct MainFrameSegment<'a, E: FieldElement> {
    table: &'a Table<E>,
    row_start: usize,
}

enum DataSegment {
    Flags,
    ResValue,
    TempMemoryPointer,
    MemoryAddress,
    MemoryValues,
    Offsets,
    TempValues,
    Selector,
}

impl<'a, E: FieldElement> MainFrameSegment<'a, E> {
    fn new(table: &'a Table<E>, row_start: usize) -> Self {
        Self { table, row_start }
    }

    fn get(&self, pos: usize, data_type: DataSegment) -> E {
        // Should this function be inlined?
        let offset = match data_type {
            DataSegment::Flags => FLAG_TRACE_OFFSET,
            DataSegment::ResValue => RES_TRACE_OFFSET,
            DataSegment::TempMemoryPointer => MEM_P_TRACE_OFFSET,
            DataSegment::MemoryAddress => MEM_A_TRACE_OFFSET,
            DataSegment::MemoryValues => MEM_V_TRACE_OFFSET,
            DataSegment::Offsets => OFF_X_TRACE_OFFSET,
            DataSegment::TempValues => DERIVED_TRACE_OFFSET,
            DataSegment::Selector => SELECTOR_TRACE_OFFSET,
        };
        self.table.get_row(self.row_start)[offset + pos]
    }

    fn get_virtual(&self, idx: usize, offset: usize, width: usize) -> E {
        if (0..width).contains(&idx) {
            self.table.get_row(0)[offset + idx]
        } else if (width..width * 2).contains(&idx) {
            self.table.get_row(1)[offset + idx - width]
        } else {
            panic!()
        }
    }
}

impl<'a, E: FieldElement + From<Felt>> MainFrameSegment<'a, E> {
    /// Result
    pub fn res(&self) -> E {
        self.get(0, DataSegment::ResValue)
    }
    /// Registers
    pub fn pc(&self) -> E {
        self.get(0, DataSegment::MemoryAddress)
    }
    pub fn ap(&self) -> E {
        self.get(0, DataSegment::TempMemoryPointer)
    }
    pub fn fp(&self) -> E {
        self.get(1, DataSegment::TempMemoryPointer)
    }
    /// Memory addresses
    pub fn dst_addr(&self) -> E {
        self.get(1, DataSegment::MemoryAddress)
    }
    pub fn op0_addr(&self) -> E {
        self.get(2, DataSegment::MemoryAddress)
    }
    pub fn op1_addr(&self) -> E {
        self.get(3, DataSegment::MemoryAddress)
    }
    /// Memory values
    pub fn inst(&self) -> E {
        self.get(0, DataSegment::MemoryValues)
    }
    pub fn dst(&self) -> E {
        self.get(1, DataSegment::MemoryValues)
    }
    pub fn op0(&self) -> E {
        self.get(2, DataSegment::MemoryValues)
    }
    pub fn op1(&self) -> E {
        self.get(3, DataSegment::MemoryValues)
    }
    /// Instruction size
    pub fn inst_size(&self) -> E {
        self.f_op1_val() + Felt::ONE.into()
    }
    /// Derived trace values
    pub fn t0(&self) -> E {
        self.get(0, DataSegment::TempValues)
    }
    pub fn t1(&self) -> E {
        self.get(1, DataSegment::TempValues)
    }
    pub fn mul(&self) -> E {
        self.get(2, DataSegment::TempValues)
    }
    /// Virtual columns of memory addreses and values
    pub fn a_m(&self, idx: usize) -> E {
        self.get_virtual(idx, MEM_A_TRACE_OFFSET, MEM_A_TRACE_WIDTH)
    }
    pub fn v_m(&self, idx: usize) -> E {
        self.get_virtual(idx, MEM_V_TRACE_OFFSET, MEM_V_TRACE_WIDTH)
    }
    /// Virtual columns of offsets
    pub fn a_rc(&self, idx: usize) -> E {
        self.get_virtual(idx, OFF_X_TRACE_OFFSET, OFF_X_TRACE_WIDTH)
    }
    /// Selector
    pub fn selector(&self) -> E {
        self.get(0, DataSegment::Selector)
    }
}

impl<'a, E: FieldElement + From<Felt>> OffsetDecomposition<E> for MainFrameSegment<'a, E> {
    fn off_dst(&self) -> E {
        bias(self.get(0, DataSegment::Offsets))
    }

    fn off_op0(&self) -> E {
        bias(self.get(1, DataSegment::Offsets))
    }

    fn off_op1(&self) -> E {
        bias(self.get(2, DataSegment::Offsets))
    }
}

impl<'a, E: FieldElement + From<Felt>> FlagDecomposition<E> for MainFrameSegment<'a, E> {
    fn flags(&self) -> Vec<E> {
        let mut flags = Vec::with_capacity(NUM_FLAGS);
        for i in 0..NUM_FLAGS {
            flags.push(self.flag_at(i));
        }
        flags
    }

    fn flag_at(&self, pos: usize) -> E {
        self.get(pos, DataSegment::Flags)
    }
}

// AUX FRAME
// --------------------------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct AuxEvaluationFrame<E: FieldElement> {
    pub table: Table<E>, // row-major indexing
}

impl<E: FieldElement> EvaluationFrame<E> for AuxEvaluationFrame<E> {
    // CONSTRUCTORS
    // --------------------------------------------------------------------------------------------

    fn new<A: Air>(air: &A) -> Self {
        let num_rows = Self::num_rows();
        let num_cols = air.trace_layout().aux_trace_width();
        AuxEvaluationFrame {
            table: Table::new(num_rows, num_cols),
        }
    }

    fn from_table(table: Table<E>) -> Self {
        Self { table }
    }

    // ROW MUTATORS
    // --------------------------------------------------------------------------------------------

    fn read_from<R: TableReader<E>>(&mut self, data: R, step: usize, offset: usize, blowup: usize) {
        let trace_len = data.num_rows();
        for (row, row_idx) in self.table.rows_mut().zip(Self::offsets().into_iter()) {
            for col_idx in 0..data.num_cols() {
                row[col_idx + offset] = data.get(col_idx, (step + row_idx * blowup) % trace_len);
            }
        }
    }

    // ROW ACCESSORS
    // --------------------------------------------------------------------------------------------

    fn row<'a>(&'a self, row_idx: usize) -> &'a [E] {
        &self.table.get_row(row_idx)
    }

    fn to_table(&self) -> Table<E> {
        self.table.clone()
    }

    fn offsets() -> &'static [usize] {
        &[0, 1]
    }
}

impl<'a, E: FieldElement> AuxEvaluationFrame<E> {
    pub fn segment(&'a self) -> AuxFrameSegment<'a, E> {
        AuxFrameSegment::new(&self.table, 0)
    }
}

pub struct AuxFrameSegment<'a, E: FieldElement> {
    curr_row: &'a [E],
    next_row: &'a [E],
}

impl<'a, E: FieldElement> AuxFrameSegment<'a, E> {
    fn new(table: &'a Table<E>, row_idx: usize) -> Self {
        let curr_row = table.get_row(row_idx);
        let next_row = table.get_row(row_idx + 1);
        Self { curr_row, next_row }
    }

    fn get_virtual(&self, idx: usize, offset: usize, width: usize) -> E {
        if (0..width).contains(&idx) {
            self.curr_row[offset + idx]
        } else if (width..width * 2).contains(&idx) {
            self.next_row[offset + idx - width]
        } else {
            panic!()
        }
    }

    /// Memory
    pub fn a_m_prime(&self, idx: usize) -> E {
        self.get_virtual(idx, A_M_PRIME_OFFSET, A_M_PRIME_WIDTH)
    }
    pub fn v_m_prime(&self, idx: usize) -> E {
        self.get_virtual(idx, V_M_PRIME_OFFSET, V_M_PRIME_WIDTH)
    }
    pub fn p_m(&self, idx: usize) -> E {
        self.get_virtual(idx, P_M_OFFSET, P_M_WIDTH)
    }

    /// Permutation range check
    pub fn a_rc_prime(&self, idx: usize) -> E {
        self.get_virtual(idx, A_RC_PRIME_OFFSET, A_RC_PRIME_WIDTH)
    }
    pub fn p_rc(&self, idx: usize) -> E {
        self.get_virtual(idx, P_RC_OFFSET, P_RC_WIDTH)
    }
}
