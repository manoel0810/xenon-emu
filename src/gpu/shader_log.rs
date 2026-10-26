//! XENON_SHADER_DISASM / XENON_SHADER_DUMP: every distinct shader the guest loads (IM_LOAD / IM_LOAD_IMMEDIATE) is
//! disassembled and printed once (gpu/ucode_disasm.rs) and/or written as raw big-endian dwords to
//! `<dir>/{vs,ps}_<hash>.bin`. The dumps are guest data: keep the directory outside the repository.

use std::collections::HashSet;
use std::path::PathBuf;

use crate::gpu::ucode::ShaderType;
use crate::gpu::ucode_disasm::disassemble;

pub struct ShaderLog {
    print: bool,
    dump_dir: Option<PathBuf>,
    seen: HashSet<(u32, u64)>,
}

/// FNV-1a over the dwords: the shader's identity in the log and the dump file name.
pub fn shader_hash(words: &[u32]) -> u64 {
    words.iter().flat_map(|w| w.to_be_bytes()).fold(0xCBF2_9CE4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x0100_0000_01B3))
}

impl ShaderLog {
    /// None when neither variable is set.
    pub fn from_config() -> Option<Self> {
        let cfg = crate::diag::config();
        let dump_dir = cfg.shader_dump.as_ref().map(PathBuf::from);
        (cfg.shader_disasm || dump_dir.is_some()).then(|| Self { print: cfg.shader_disasm, dump_dir, seen: HashSet::new() })
    }

    pub fn loaded(&mut self, shader_type: u32, words: &[u32]) {
        let hash = shader_hash(words);
        if words.is_empty() || !self.seen.insert((shader_type, hash)) {
            return;
        }
        let (kind, ty) = if shader_type == 0 { ("vs", ShaderType::Vertex) } else { ("ps", ShaderType::Pixel) };
        if self.print {
            let d = disassemble(words, ty);
            println!("  [SHADER] {} {:016x}: {} dwords, {} ALU, {} fetch, {} issue(s)", kind, hash, words.len(), d.alu_count, d.fetch_count, d.issues.len());
            for issue in &d.issues {
                println!("    !! {}", issue);
            }
            print!("{}", d.text);
        }
        if let Some(dir) = &self.dump_dir {
            let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_be_bytes()).collect();
            let _ = std::fs::create_dir_all(dir);
            let _ = std::fs::write(dir.join(format!("{}_{:016x}.bin", kind, hash)), bytes);
        }
    }
}
