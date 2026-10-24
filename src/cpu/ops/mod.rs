//! Instruction semantics grouped by kind. `Cpu::step` decodes the opcode and calls the matching `exec_*`.

mod int;
mod branch;
mod loadstore;
mod float;
mod vmx;
mod system;
