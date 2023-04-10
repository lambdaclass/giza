// Modified from https://github.com/o1-labs/proof-systems

use std::convert::TryInto;
use std::fmt::{Display, Formatter, Result};
use std::ops::{Index, IndexMut};

use core::iter::repeat;
use giza_core::{Felt, FieldHelpers, StarkField, Word};

/// This data structure stores the memory of the program
#[derive(Debug, Clone)]
pub struct Memory {
    /// length of the public memory
    codelen: usize,
    /// full memory vector, None if non initialized
    pub data: Vec<Option<Word>>,
}

impl Index<Felt> for Memory {
    type Output = Option<Word>;
    fn index(&self, idx: Felt) -> &Self::Output {
        let addr: u64 = idx.to_u64();
        &self.data[addr as usize]
    }
}

impl IndexMut<Felt> for Memory {
    fn index_mut(&mut self, idx: Felt) -> &mut Self::Output {
        let addr: u64 = idx.to_u64();
        self.resize(addr);
        &mut self.data[addr as usize]
    }
}

impl Display for Memory {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        for i in 1..self.size() {
            // Visualize content of memory
            if let Some(elem) = self[Felt::from(i as u64)] {
                if writeln!(f, "{0:>6}: 0x{1:}", i, elem.word().to_hex_le()).is_err() {
                    println!("Error while writing")
                }
            } else if writeln!(f, "{0:>6}: None", i).is_err() {
                println!("Error while writing")
            }
        }
        Ok(())
    }
}

impl Memory {
    /// Create a new memory structure from a vector of field elements
    pub fn new(input: Vec<Felt>) -> Memory {
        // Initialized with the public memory (compiled instructions only)
        let mut aux = vec![Felt::from(0u8)];
        aux.extend(input);
        Memory {
            codelen: aux.len(),
            data: aux.into_iter().map(|i| Some(Word::new(i))).collect(),
        }
    }

    /// Get size of the public memory
    pub fn get_codelen(&self) -> usize {
        self.codelen
    }

    /// Set size of the public memory
    pub fn set_codelen(&mut self, len: usize) {
        self.codelen = len;
    }

    /// Get size of the full memory
    pub fn size(&self) -> u64 {
        self.data.len() as u64
    }

    /// Resizes memory with enough additional None slots if necessary before writing or reading
    fn resize(&mut self, addr: u64) {
        if let Some(additional) = addr.checked_sub(self.size() - 1) {
            self.data.extend(repeat(None).take(additional as usize));
        }
    }

    /// Write u64 element in memory address
    pub fn write(&mut self, addr: Felt, elem: Felt) {
        self[addr] = Some(Word::new(elem));
    }

    /// Write u64 element in memory address
    pub fn write_pub(&mut self, addr: Felt, elem: Felt) {
        self.write(addr, elem);
        self.codelen += 1;
    }

    /// Read element in memory address
    pub fn read(&self, addr: Felt) -> Option<Felt> {
        //self.resize(addr.to_u64()); // Resize if necessary
        self[addr].map(|x| x.word())
    }

    /// Returns a list of all memory holes (defined as missing private memory
    /// accesses from the provided trace vec)
    /// TODO: Memory should be stored as a BTreeMap in data, not a Vec.
    pub fn get_holes(&self, vec: Vec<Felt>) -> Vec<Felt> {
        let mut accesses = vec
            .iter()
            .map(|x| TryInto::<u64>::try_into(x.as_int()).unwrap())
            .collect::<Vec<_>>();
        accesses.sort_unstable();

        let mut holes = vec![];
        for s in accesses.windows(2) {
            match s[1] - s[0] {
                0 | 1 => {}
                _ => {
                    if s[0] > self.codelen as u64 {
                        holes.extend((s[0] + 1..s[1]).map(|x| Felt::from(x)).collect::<Vec<_>>());
                    }
                }
            }
        }
        holes
    }
}

#[cfg(test)]
mod tests {
    use super::Felt as F;
    use super::*;
    use giza_core::{Felt, FieldHelpers, Word};

    #[test]
    fn test_cairo_bytecode() {
        // This test starts with the public memory corresponding to a simple  program
        // func main{}():
        //    tempvar x = 10;
        //    return()
        // end
        // And checks that memory writing and reading works as expected by completing
        // the total memory of executing the program
        let instrs = vec![
            F::from(0x480680017fff8000u64),
            F::from(10u64),
            F::from(0x208b7fff7fff7ffeu64),
        ];
        let mut memory = Memory::new(instrs);
        memory.write(F::from(memory.size() as u64), F::from(7u64));
        memory.write(F::from(memory.size() as u64), F::from(7u64));
        memory.write(F::from(memory.size() as u64), F::from(10u64));
        println!("{}", memory);
        // Check content of an address
        assert_eq!(
            memory.read(F::from(1u32)).unwrap(),
            F::from(0x480680017fff8000u64)
        );
        // Check that the program contained 3 words
        assert_eq!(3, memory.get_codelen());
        // Check we have 6 words, excluding the dummy entry
        assert_eq!(6, memory.size() - 1);
        memory.read(F::from(10u32));
    }
}
