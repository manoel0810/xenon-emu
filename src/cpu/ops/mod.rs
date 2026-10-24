//! Instruction semantics grouped by kind. `Cpu::step` decodes the opcode and calls the matching `exec_*`.

mod int;
mod branch;
mod d3d_pack;
mod loadstore;
mod float;
mod vmx;
mod system;
