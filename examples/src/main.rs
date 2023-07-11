use air::{ProcessorAir, ProofOptions};
use giza_core::Felt;
use runner::{Memory, Program};

fn main() {
    //  %builtins output
    //  from starkware.cairo.common.serialize import serialize_word
    //  func main{output_ptr : felt*}():
    //      tempvar x = 10
    //      tempvar y = x + x
    //      tempvar z = y * y + x
    //      serialize_word(x)
    //      serialize_word(y)
    //      serialize_word(z)
    //      return ()
    //  end
    //  */
    let instrs: Vec<Felt> = vec![
        Felt::from(0x400380007ffc7ffdu64),
        Felt::from(0x482680017ffc8000u64),
        Felt::from(1u64),
        Felt::from(0x208b7fff7fff7ffeu64),
        Felt::from(0x480680017fff8000u64),
        Felt::from(10u64),
        Felt::from(0x48307fff7fff8000u64),
        Felt::from(0x48507fff7fff8000u64),
        Felt::from(0x48307ffd7fff8000u64),
        Felt::from(0x480a7ffd7fff8000u64),
        Felt::from(0x48127ffb7fff8000u64),
        Felt::from(0x1104800180018000u64),
        -Felt::from(11u64),
        Felt::from(0x48127ff87fff8000u64),
        Felt::from(0x1104800180018000u64),
        -Felt::from(14u64),
        Felt::from(0x48127ff67fff8000u64),
        Felt::from(0x1104800180018000u64),
        -Felt::from(17u64),
        //Felt::from(0x208b7fff7fff7ffeu64),
        Felt::from(0x10780017fff7fffu64), // infinite loop
    ];
    let mut mem = Memory::new(instrs);
    mem.write_pub(Felt::from(21u32), Felt::from(41u32)); // beginning of output
    mem.write_pub(Felt::from(22u32), Felt::from(44u32)); // end of output
    mem.write_pub(Felt::from(23u32), Felt::from(44u32)); // end of program

    // run the program to create an execution trace
    let mut program = Program::new(&mut mem, 5, 24);
    let trace = program.execute().unwrap();

    // generate the proof of execution
    let proof_options = ProofOptions::with_proof_options(None, None, None, None, None);
    let (proof, pub_inputs) = prover::prove_trace(trace, &proof_options).unwrap();

    // verify correct program execution
    match winterfell::verify::<ProcessorAir>(proof, pub_inputs) {
        Ok(_) => println!("Execution verified"),
        Err(err) => println!("Failed to verify execution: {}", err),
    }
}
