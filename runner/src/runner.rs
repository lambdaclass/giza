// Modified from https://github.com/o1-labs/proof-systems

use crate::errors::ExecutionError;
use crate::memory::Memory;
use crate::trace::ExecutionTrace;
use giza_core::{flags::*, *};

#[cfg(feature = "hints")]
use crate::hints::{ExecutionEffect as HintExecutionEffect, HintManager};

/// A data structure to store a current step of computation
pub struct Step<'a> {
    pub mem: &'a Memory,
    pub curr: RegisterState,
    pub next: Option<RegisterState>,
    #[cfg(feature = "hints")]
    hints: Option<&'a HintManager>,
}

impl<'a> Step<'a> {
    /// Creates a new execution step from a step index, a word, and current pointers
    pub fn new(mem: &'a Memory, ptrs: RegisterState) -> Step<'a> {
        Step {
            mem,
            curr: ptrs,
            next: None,
        }
    }

    /// Executes a step from the current registers and returns the instruction state
    pub fn execute(&mut self, write: bool) -> InstructionState {
        // Execute hints and apply changes
        #[cfg(feature = "hints")]
        self.execute_hints();

        // Execute instruction
        let (op0_addr, mut op0) = self.set_op0();
        let (op1_addr, mut op1, size) = self.set_op1(op0);
        let (dst_addr, mut dst) = self.set_dst();
        let mut res = self.set_res(op0, op1, dst);
        let next_pc = self.next_pc(size, res, dst, op1);
        let (next_ap, next_fp, op0_update, op1_update, res_update, dst_update) =
            self.next_apfp(size, res, dst, dst_addr, op1_addr, write);

        println!("OP0: {:?}", op0);
        if op0_update.is_some() {
            op0 = op0_update;
        }
        if op1_update.is_some() {
            op1 = op1_update;
        }
        if res_update.is_some() {
            res = res_update;
        }
        if dst_update.is_some() {
            dst = dst_update;
        }
        self.next = Some(RegisterState::new(
            next_pc.expect("Empty next program counter"),
            next_ap.expect("Empty next allocation pointer"),
            next_fp.expect("Empty next frame pointer"),
        ));
        InstructionState::new(
            self.inst(),
            size,
            dst,
            op0,
            op1,
            res,
            dst_addr,
            op0_addr,
            op1_addr,
        )
    }

    #[cfg(feature = "hints")]
    fn set_hint_manager(&mut self, hints: &'a HintManager) {
        self.hints = hints;
    }

    #[cfg(feature = "hints")]
    fn execute_hints(&mut self) {
        if let Some(manager) = self.hints {
            for hint in manager.get_hints(self.curr.pc).into_iter().flatten() {
                let changes = hint.exec(&self).unwrap();
                self.apply_hint_effects(changes);
            }
        }
    }

    #[cfg(feature = "hints")]
    fn apply_hint_effects(&mut self, res: HintExecutionEffect) {
        self.curr.pc = res.pc;
        self.curr.ap = res.ap;
        self.curr.fp = res.fp;
        if let Some(updates) = res.mem_updates {
            for (addr, elem) in updates.0.iter() {
                self.mem.write(Felt::from(*addr), elem.word());
            }
        }
    }

    /// This function returns the current word instruction being executed
    fn inst(&mut self) -> Word {
        Word::new(self.mem.read(self.curr.pc).expect("pc points to None cell"))
    }

    /// This function computes the first operand address.
    /// Outputs: `(op0_addr, op0)`
    fn set_op0(&mut self) -> (Felt, Option<Felt>) {
        let reg = match self.inst().op0_reg() {
            /*0*/ OP0_AP => self.curr.ap, // reads first word from allocated memory
            /*1*/ _ => self.curr.fp, // reads first word from input stack
        };
        let op0_addr = reg + self.inst().off_op0();
        let op0 = self.mem.read(op0_addr);
        (op0_addr, op0)
    }

    /// This function computes the second operand address and content and the instruction size
    /// Panics if the flagset `OP1_SRC` has more than 1 nonzero bit
    /// Inputs: `op0`
    /// Outputs: `(op1_addr, op1, size)`
    fn set_op1(&mut self, op0: Option<Felt>) -> (Felt, Option<Felt>, Felt) {
        let (reg, size) = match self.inst().op1_src() {
            /*0*/
            OP1_DBL => (op0.expect("None op0 for OP1_DBL"), Felt::ONE), // double indexing, op0 should be positive for address
            /*1*/
            OP1_VAL => (self.curr.pc, Felt::TWO), // off_op1 will be 1 and then op1 contains an immediate value
            /*2*/ OP1_FP => (self.curr.fp, Felt::ONE),
            /*4*/ OP1_AP => (self.curr.ap, Felt::ONE),
            _ => panic!("Invalid op1_src flagset"),
        };
        let op1_addr = reg + self.inst().off_op1(); // apply second offset to corresponding register
        let op1 = self.mem.read(op1_addr);
        (op1_addr, op1, size)
    }

    /// This function computes the value of the result of the arithmetic operation
    /// Panics if a `jnz` instruction is used with an invalid format
    ///     or if the flagset `RES_LOG` has more than 1 nonzero bit
    /// Inputs: `op0`, `op1`
    /// Outputs: `res`
    fn set_res(&mut self, op0: Option<Felt>, op1: Option<Felt>, dst: Option<Felt>) -> Option<Felt> {
        let res;
        if self.inst().pc_up() == PC_JNZ {
            /*4*/
            // jnz instruction
            if self.inst().res_log() == RES_ONE /*0*/
                && self.inst().opcode() == OPC_JMP_INC /*0*/
                && self.inst().ap_up() != AP_ADD
            /* not 1*/
            {
                // in the context of a jnz instruction, the res register is unused, so we repurpose
                // it to hold the value v = dst^(-1), which is used in the pc update constraint
                let dst = dst.expect("None dst after JNZ");
                if dst == Felt::ZERO {
                    res = Some(dst)
                } else {
                    res = Some(dst.inv());
                }
            } else {
                panic!("Invalid JNZ instruction");
            }
        } else if self.inst().pc_up() == PC_SIZ /*0*/
            || self.inst().pc_up() == PC_ABS /*1*/
            || self.inst().pc_up() == PC_REL
        /*2*/
        {
            // rest of types of updates
            // common increase || absolute jump || relative jump
            res = {
                match self.inst().res_log() {
                    /*0*/
                    RES_ONE => op1, // right part is single operand
                    /*1*/
                    RES_ADD => Some(
                        op0.expect("None op0 after RES_ADD") + op1.expect("None op1 after RES_ADD"),
                    ), // right part is addition
                    /*2*/
                    RES_MUL => Some(
                        op0.expect("None op0 after RES_MUL") * op1.expect("None op1 after RES_MUL"),
                    ), // right part is multiplication
                    _ => panic!("Invalid res_log flagset"),
                }
            };
        } else {
            // multiple bits take value 1
            panic!("Invalid pc_up flagset");
        }
        res
    }

    /// This function computes the destination address
    /// Outputs: `(dst_addr, dst)`
    fn set_dst(&mut self) -> (Felt, Option<Felt>) {
        let reg = match self.inst().dst_reg() {
            /*0*/ DST_AP => self.curr.ap, // read from stack
            /*1*/ _ => self.curr.fp, // read from parameters
        };
        let dst_addr = reg + self.inst().off_dst();
        let dst = self.mem.read(dst_addr);
        (dst_addr, dst)
    }

    /// This function computes the next program counter
    /// Panics if the flagset `PC_UP` has more than 1 nonzero bit
    /// Inputs: `size`, `res`, `dst`, `op1`,
    /// Outputs: `next_pc`
    fn next_pc(
        &mut self,
        size: Felt,
        res: Option<Felt>,
        dst: Option<Felt>,
        op1: Option<Felt>,
    ) -> Option<Felt> {
        match self.inst().pc_up() {
            /*0*/
            PC_SIZ => Some(self.curr.pc + size), // common case, next instruction is right after the current one
            /*1*/
            PC_ABS => Some(res.expect("None res after PC_ABS")), // absolute jump, next instruction is in res,
            /*2*/
            PC_REL => Some(self.curr.pc + res.expect("None res after PC_REL")), // relative jump, go to some address relative to pc
            /*4*/
            PC_JNZ => {
                // conditional relative jump (jnz)
                if dst == Some(Felt::ZERO) {
                    // if condition false, common case
                    Some(self.curr.pc + size)
                } else {
                    // if condition true, relative jump with second operand
                    Some(self.curr.pc + op1.expect("None op1 after PC_JNZ"))
                }
            }
            _ => panic!("Invalid pc_up flagset"),
        }
    }

    /// This function computes the next values of the allocation and frame pointers
    /// Panics if in a `call` instruction the flagset [AP_UP] is incorrect
    ///     or if in any other instruction the flagset AP_UP has more than 1 nonzero bit
    ///     or if the flagset `OPCODE` has more than 1 nonzero bit
    /// Inputs: `size`, `res`, `dst`, `dst_addr`, `op1_addr`
    /// Outputs: `(next_ap, next_fp, op0_update, op1_update, res_update, dst_update)`
    fn next_apfp(
        &mut self,
        size: Felt,
        res: Option<Felt>,
        dst: Option<Felt>,
        dst_addr: Felt,
        op1_addr: Felt,
        write: bool,
    ) -> (
        Option<Felt>,
        Option<Felt>,
        Option<Felt>,
        Option<Felt>,
        Option<Felt>,
        Option<Felt>,
    ) {
        let (next_ap, next_fp);
        let mut op0_update = None;
        let mut op1_update = None;
        let mut res_update = None;
        let mut dst_update = None;
        if self.inst().opcode() == OPC_CALL {
            /*1*/
            // "call" instruction
            if write {
                //self.mem.write(self.curr.ap, self.curr.fp);
                //self.mem
                //    .write(self.curr.ap + Felt::ONE, self.curr.pc + size);
            } else {
                let expected_a = self.mem.read(self.curr.ap).unwrap();
                let expected_b = self.mem.read(self.curr.ap + Felt::ONE).unwrap();
                assert_eq!(expected_a, self.curr.fp);
                assert_eq!(expected_b, self.curr.pc + size);
            }

            println!("DST: {:?}", dst);
            dst_update = self.mem.read(self.curr.ap);
            println!("DST_UPDATE: {:?}", dst_update);
            op0_update = self.mem.read(self.curr.ap + Felt::ONE);
            println!("OP0_UPDATE: {:?}", op0_update);

            // Update fp
            // pointer for next frame is after current fp and instruction after call
            next_fp = Some(self.curr.ap + Felt::TWO);

            // Update ap
            match self.inst().ap_up() {
                /*0*/
                AP_Z2 => next_ap = Some(self.curr.ap + Felt::TWO), // two words were written so advance 2 positions
                _ => panic!("ap increment in call instruction"),
            };
        } else if self.inst().opcode() == OPC_JMP_INC /*0*/
            || self.inst().opcode() == OPC_RET /*2*/
            || self.inst().opcode() == OPC_AEQ
        /*4*/
        {
            // rest of types of instruction
            // jumps and increments || return || assert equal
            match self.inst().ap_up() {
                /*0*/ AP_Z2 => next_ap = Some(self.curr.ap), // no modification on ap
                /*1*/
                AP_ADD => {
                    // ap += <op> should be larger than current ap
                    next_ap = Some(self.curr.ap + res.expect("None res after AP_ADD"))
                }
                /*2*/ AP_ONE => next_ap = Some(self.curr.ap + Felt::ONE), // ap++
                _ => panic!("Invalid ap_up flagset"),
            }

            match self.inst().opcode() {
                /*0*/
                OPC_JMP_INC => next_fp = Some(self.curr.fp), // no modification on fp
                /*2*/
                OPC_RET => next_fp = Some(dst.expect("None dst after OPC_RET")), // ret sets fp to previous fp that was in [ap-2]
                /*4*/
                OPC_AEQ => {
                    // The following conditional is a fix that is not explained in the whitepaper
                    // The goal is to distinguish two types of ASSERT_EQUAL where one checks that
                    // dst = res , but in order for this to be true, one sometimes needs to write
                    // the res in mem(dst_addr) and sometimes write dst in mem(res_dir). The only
                    // case where res can be None is when res = op1 and thus res_dir = op1_addr
                    if res.is_none() {
                        // res = dst
                        if write {
                            //self.mem
                            //    .write(op1_addr, dst.expect("None dst after OPC_AEQ"));
                        } else {
                            let expected_a = self.mem.read(op1_addr).unwrap();
                            assert_eq!(expected_a, dst.unwrap());
                        }
                        op1_update = self.mem.read(op1_addr);
                        res_update = self.mem.read(op1_addr);
                    } else {
                        // dst = res
                        if write {
                            //self.mem
                            //    .write(dst_addr, res.expect("None res after OPC_AEQ"));
                        } else {
                            let expected_a = self.mem.read(dst_addr).unwrap();
                            assert_eq!(expected_a, res.unwrap());
                        }
                        dst_update = self.mem.read(dst_addr);
                    }
                    next_fp = Some(self.curr.fp); // no modification on fp
                }
                _ => {
                    panic!("This case must never happen")
                }
            }
        } else {
            panic!("Invalid opcode flagset");
        }
        (
            next_ap, next_fp, op0_update, op1_update, res_update, dst_update,
        )
    }
}

/// Trace-friendly record of registers and instruction state across
/// all program execution steps
pub struct State {
    pub flags: [Vec<Felt>; FLAG_TRACE_WIDTH],
    pub res: [Vec<Felt>; RES_TRACE_WIDTH],
    pub mem_p: [Vec<Felt>; MEM_P_TRACE_WIDTH],
    pub mem_a: [Vec<Felt>; MEM_A_TRACE_WIDTH],
    pub mem_v: [Vec<Felt>; MEM_V_TRACE_WIDTH],
    pub offsets: [Vec<Felt>; OFF_X_TRACE_WIDTH],
}

impl State {
    pub fn new(init_trace_len: usize) -> Self {
        let mut flags: Vec<Vec<Felt>> = Vec::with_capacity(FLAG_TRACE_WIDTH);
        let mut res: Vec<Vec<Felt>> = Vec::with_capacity(RES_TRACE_WIDTH);
        let mut mem_p: Vec<Vec<Felt>> = Vec::with_capacity(MEM_P_TRACE_WIDTH);
        let mut mem_a: Vec<Vec<Felt>> = Vec::with_capacity(MEM_A_TRACE_WIDTH);
        let mut mem_v: Vec<Vec<Felt>> = Vec::with_capacity(MEM_V_TRACE_WIDTH);
        let mut offsets: Vec<Vec<Felt>> = Vec::with_capacity(OFF_X_TRACE_WIDTH);
        for _ in 0..FLAG_TRACE_WIDTH {
            let column = Felt::zeroed_vector(init_trace_len);
            flags.push(column);
        }
        for _ in 0..RES_TRACE_WIDTH {
            let column = Felt::zeroed_vector(init_trace_len);
            res.push(column);
        }
        for _ in 0..MEM_P_TRACE_WIDTH {
            let column = Felt::zeroed_vector(init_trace_len);
            mem_p.push(column);
        }
        for _ in 0..MEM_A_TRACE_WIDTH {
            let column = Felt::zeroed_vector(init_trace_len);
            mem_a.push(column);
        }
        for _ in 0..MEM_V_TRACE_WIDTH {
            let column = Felt::zeroed_vector(init_trace_len);
            mem_v.push(column);
        }
        for _ in 0..OFF_X_TRACE_WIDTH {
            let column = Felt::zeroed_vector(init_trace_len);
            offsets.push(column);
        }
        State {
            flags: flags.try_into().unwrap(),
            res: res.try_into().unwrap(),
            mem_p: mem_p.try_into().unwrap(),
            mem_a: mem_a.try_into().unwrap(),
            mem_v: mem_v.try_into().unwrap(),
            offsets: offsets.try_into().unwrap(),
        }
    }

    pub fn set_register_state(&mut self, step: usize, s: RegisterState) {
        self.mem_a[0][step] = s.pc;
        self.mem_p[0][step] = s.ap;
        self.mem_p[1][step] = s.fp;
    }

    pub fn set_instruction_state(&mut self, step: usize, s: InstructionState) {
        // Flags
        let flags = s.inst.flags();
        for i in 0..=15 {
            self.flags[i][step] = flags[i];
        }

        // Result
        self.res[0][step] = s.res.unwrap_or(Felt::ZERO);

        // Instruction
        self.mem_v[0][step] = s.inst.word();

        // Auxiliary values
        self.mem_v[1][step] = s.dst.unwrap_or(Felt::ZERO);
        self.mem_v[2][step] = s.op0.unwrap_or(Felt::ZERO);
        self.mem_v[3][step] = s.op1.unwrap_or(Felt::ZERO);

        // Operands
        self.mem_a[1][step] = s.dst_addr;
        self.mem_a[2][step] = s.op0_addr;
        self.mem_a[3][step] = s.op1_addr;

        // Offsets
        self.offsets[0][step] = s.inst.off_dst();
        self.offsets[1][step] = s.inst.off_op0();
        self.offsets[2][step] = s.inst.off_op1();
    }
}

/// Stores all information needed to run a program
pub struct Program<'a> {
    /// total number of steps
    steps: usize,
    /// full execution memory
    mem: &'a mut Memory,
    /// initial register state
    init: RegisterState,
    /// final register state
    fin: RegisterState,
    /// requested builtins
    builtins: Vec<Builtin>,
    /// hints
    #[cfg(feature = "hints")]
    hints: Option<HintManager>,
}

impl<'a> Program<'a> {
    /// Creates an execution from the public information (memory and initial pointers)
    #[cfg(feature = "hints")]
    pub fn new(mem: &mut Memory, pc: u64, ap: u64, hints: Option<HintManager>) -> Program {
        Program {
            steps: 0,
            mem,
            init: RegisterState::new(Felt::from(pc), Felt::from(ap), Felt::from(ap)),
            fin: RegisterState::new(Felt::ZERO, Felt::ZERO, Felt::ZERO),
            builtins: vec![],
            hints,
        }
    }

    #[cfg(not(feature = "hints"))]
    pub fn new(mem: &mut Memory, pc: u64, ap: u64) -> Program {
        Program {
            steps: 0,
            mem,
            init: RegisterState::new(Felt::from(pc), Felt::from(ap), Felt::from(ap)),
            fin: RegisterState::new(Felt::ZERO, Felt::ZERO, Felt::ZERO),
            builtins: vec![],
        }
    }

    /// Outputs the total number of steps of the execution carried out by the runner
    pub fn get_steps(&self) -> usize {
        self.steps
    }

    /// Outputs the final value of the pointers after the execution carried out by the runner
    pub fn get_final(&self) -> RegisterState {
        self.fin
    }

    /// This function simulates an execution of the program received as input
    /// and returns an execution trace
    pub fn execute(&mut self) -> Result<ExecutionTrace, ExecutionError> {
        let mut state = State::new(self.mem.size() as usize);
        let mut n: usize = 0;
        let mut end = false;
        let mut curr = self.init;
        let mut next = curr;

        // keep executing steps until the end is reached
        while !end {
            // create current step of computation
            let mut step = Step::new(self.mem, next);
            curr = step.curr;

            #[cfg(feature = "hints")]
            step.set_hint_manager(self.hints.as_ref());

            // execute current step and save state
            let inst_state = step.execute(true);
            state.set_register_state(n, curr);
            state.set_instruction_state(n, inst_state);

            n += 1;
            match step.next {
                None => end = true,
                _ => {
                    next = step.next.expect("Empty next pointers");
                    if curr.ap.as_int() <= next.pc.as_int() {
                        // if reading from unallocated memory, end
                        end = true;
                    }
                }
            }
        }
        self.fin = curr;
        self.steps = n;

        Ok(ExecutionTrace::new(
            n,
            &mut state,
            &self.mem,
            self.builtins.clone(),
        ))
    }
}
