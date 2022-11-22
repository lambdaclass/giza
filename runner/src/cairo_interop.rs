/// Code for parsing the outputs of Starkware's cairo-runner.
/// Note the following:
/// - Field elements are encoded in little-endian byte order.
/// - Cairo serializes field elements as 32 bytes (the program
///   prime is assumed to be equal to the 252-bit Starkware prime).
///
use crate::memory::Memory;
use giza_core::{Builtin, Felt, RegisterState};
use serde::{Deserialize, Serialize};
use std::fs::{metadata, File};
use std::io::{BufReader, Read};
use std::path::PathBuf;

#[derive(Serialize, Deserialize)]
struct CompiledProgram {
    builtins: Vec<String>,
    data: Vec<String>,
    prime: String,
}

/// Parses an execution trace outputted by the cairo-runner.
/// e.g. cairo-runner --trace_file out/trace.bin
pub fn read_trace_bin(path: &PathBuf) -> Vec<RegisterState> {
    let mut f = File::open(&path).expect("no file found");
    let metadata = metadata(&path).expect("unable to read metadata");
    let length = metadata.len() as usize;

    // Buffer for register values
    let mut pc: [u8; 8] = Default::default();
    let mut ap: [u8; 8] = Default::default();
    let mut fp: [u8; 8] = Default::default();

    let mut ptrs: Vec<RegisterState> = vec![];
    let mut bytes_read = 0;
    while bytes_read < length {
        bytes_read += f.read(&mut ap).unwrap();
        bytes_read += f.read(&mut fp).unwrap();
        bytes_read += f.read(&mut pc).unwrap();
        let reg = RegisterState::new(
            u64::from_le_bytes(pc),
            u64::from_le_bytes(ap),
            u64::from_le_bytes(fp),
        );
        ptrs.push(reg);
    }

    ptrs
}

/// Parses a memory dump outputted by the cairo-runner.
/// e.g. cairo-runner --memory_file out/memory.bin
pub fn read_memory_bin(mem_path: &PathBuf, program_path: &PathBuf) -> Memory {
    // Read memory trace
    let mut f = File::open(&mem_path).expect("Memory trace file not found");
    let metadata = metadata(&mem_path).expect("Unable to read metadata");
    let length = metadata.len() as usize;

    // Buffer for memory accesses
    let mut address: [u8; 8] = Default::default();
    let mut value: [u8; 32] = Default::default();

    let mut mem = Memory::new(vec![]).clone();
    let mut bytes_read = 0;
    while bytes_read < length {
        bytes_read += f.read(&mut address).unwrap();
        bytes_read += f.read(&mut value).unwrap();
        mem.write(
            Felt::try_from(u64::from_le_bytes(address)).unwrap(),
            Felt::try_from(value).unwrap(),
        );
    }

    // Read compiled program and set memory codelen (the length of the public memory)
    let file = File::open(&program_path).expect("Compiled program file not found");
    let reader = BufReader::new(file);
    let p: CompiledProgram = serde_json::from_reader(reader).unwrap();
    mem.set_codelen(p.data.len() + 1);

    mem
}

pub fn read_builtins(program_path: &PathBuf, output_len: Option<u64>) -> Vec<Builtin> {
    // Read compiled program and set memory codelen (the length of the public memory)
    let file = File::open(&program_path).expect("Compiled program file not found");
    let reader = BufReader::new(file);
    let p: CompiledProgram = serde_json::from_reader(reader).unwrap();
    let builtins = p
        .builtins
        .iter()
        .filter_map(|b| match b.as_str() {
            "output" => Some(Builtin::Output(output_len.unwrap())),
            _ => None,
        })
        .collect::<Vec<_>>();
    builtins
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trace_bin() {
        let trace = read_trace_bin(&PathBuf::from("../tmp/trace.bin"));
        println!("{:?}", trace);
    }

    #[test]
    fn test_memory_bin() {
        let mem = read_memory_bin(
            &PathBuf::from("../tmp/memory.bin"),
            &PathBuf::from("../tmp/program.json"),
        );
        println!("{:?}", mem.data);
    }
}
