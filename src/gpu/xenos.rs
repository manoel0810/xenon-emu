use crate::gpu::draw_log::{DrawInitiator, DrawLog};
use crate::gpu::edram::{Edram, RenderState, EDRAM_MODE_COPY, RB_MODECONTROL};
use crate::gpu::pm4::*;
use crate::gpu::primitive::{assemble_primitives, clip_position, clip_triangle, project, ScreenVertex, Vertex};
use crate::gpu::rasterizer::rasterize_triangle;
use crate::gpu::ring_buffer::RingBuffer;
use crate::gpu::shader_env::GpuShaderEnv;
use crate::gpu::shader_exec::draw_indices;
use crate::gpu::shader_interp::{CompiledShader, ShaderState};
use crate::gpu::texture::GpuTextureSampler;
use crate::gpu::ucode::ShaderType;
use crate::memory::MemoryBus;
use std::collections::HashMap;

pub const XENOS_WIDTH: usize = 1280;
pub const XENOS_HEIGHT: usize = 720;

/// Whether the command processor can go on after a packet (Xenia's CP thread instead sleeps inside WAIT_REG_MEM).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CpFlow {
    Continue,
    /// A WAIT_REG_MEM condition is not met: the packet is executed again the next time the CP runs.
    Blocked,
}

/// An indirect buffer being executed: guest address, length and position in dwords. The words are read from memory
/// as they are executed (like Xenia's ring reader), so a CP suspended inside an IB sees later CPU writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IbFrame {
    pub addr: u64,
    pub len: usize,
    pub pos: usize,
}

/// ATI Xenos (Xbox 360) Command Processor and Graphics Engine
pub struct XenosGpu {
    /// Host-side color buffer the display falls back to when no frontbuffer is known; the GPU never writes it (the
    /// guest's own resolves fill the frontbuffer in guest memory).
    pub color_buffer: Vec<u32>,
    /// The EDRAM (render targets and resolves, Phase 4.5).
    pub edram: Edram,
    /// Execution counters
    pub packets_executed: u64,
    /// Real Xenos opcodes (as emitted by the Direct3D driver in the dash) seen so far, by opcode value
    pub real_opcode_counts: HashMap<u8, u64>,
    /// GPU counter (Xenia `counter_`): written by EVENT_WRITE_SHD when bit 31 of the initiator is set;
    /// incremented once per vblank (`mark_vblank`) and per XE_SWAP.
    pub gpu_counter: u32,
    /// Draws submitted with the real DRAW_INDX_2 / DRAW_INDX opcodes (not rasterised yet: the dash shaders are Xenos microcode)
    pub real_draws: u64,
    /// CPU numbers of CP interrupts raised by PM4 INTERRUPT packets, not yet delivered to the guest's graphics
    /// interrupt callback (delivered by the frame loop with source = 1).
    pub pending_cp_interrupts: Vec<u32>,
    /// XE_SWAP packets executed (written by VdSwap into the guest command buffer).
    pub swaps: u64,
    /// Last XE_SWAP: (frontbuffer physical address, width, height), as passed to Xenia's IssueSwap.
    pub last_swap: Option<(u32, u32, u32)>,
    /// Bin mask/select for predicated Type-3 packets (Xenia CommandProcessor bin_mask_/bin_select_, both start at
    /// 0xFFFFFFFF).
    pub bin_mask: u64,
    pub bin_select: u64,
    /// Active vertex / pixel shader microcode, as loaded by IM_LOAD / IM_LOAD_IMMEDIATE (Xenia active_*_shader_).
    pub vertex_shader: Vec<u32>,
    pub pixel_shader: Vec<u32>,
    /// Times the CP suspended on an unmet WAIT_REG_MEM condition.
    pub wait_blocks: u64,
    /// Indirect buffers in execution, innermost last; kept across calls while the CP is blocked inside one.
    pub ib_stack: Vec<IbFrame>,
    /// The last `execute_ring` stopped at an unmet WAIT_REG_MEM.
    pub waiting: bool,
    /// Indirect-buffer words skipped because the packet starting there did not fit in the buffer.
    pub ib_decode_errors: u64,
    /// XENON_DRAW_LOG state (None when off).
    pub draw_log: Option<DrawLog>,
    /// XENON_SHADER_DISASM / XENON_SHADER_DUMP state (None when off).
    pub shader_log: Option<crate::gpu::shader_log::ShaderLog>,
    /// XENON_SHADER_EXEC state (None when off).
    pub shader_exec: Option<crate::gpu::shader_exec::ShaderExec>,
    /// XENON_RASTER_PROFILE: host time spent per pixel shader (draws, seconds) and in resolves; None when off.
    pub raster_profile: Option<(HashMap<u64, (u64, f64)>, u64, f64)>,
}

impl XenosGpu {
    pub fn new() -> Self {
        Self {
            color_buffer: vec![0u32; XENOS_WIDTH * XENOS_HEIGHT],
            edram: Edram::new(),
            packets_executed: 0,
            real_opcode_counts: HashMap::new(),
            gpu_counter: 0,
            real_draws: 0,
            pending_cp_interrupts: Vec::new(),
            swaps: 0,
            last_swap: None,
            bin_mask: 0xFFFF_FFFF,
            bin_select: 0xFFFF_FFFF,
            vertex_shader: Vec::new(),
            pixel_shader: Vec::new(),
            ib_decode_errors: 0,
            wait_blocks: 0,
            ib_stack: Vec::new(),
            waiting: false,
            draw_log: crate::diag::config().draw_log.then(DrawLog::new),
            shader_log: crate::gpu::shader_log::ShaderLog::from_config(),
            shader_exec: crate::diag::config().shader_exec.map(crate::gpu::shader_exec::ShaderExec::new),
            raster_profile: crate::diag::config().raster_profile.then(|| (HashMap::new(), 0, 0.0)),
        }
    }

    /// Executes all pending PM4 packets in the Primary Ring Buffer
    /// Executes all pending PM4 packets in the Primary Ring Buffer, first resuming an indirect buffer the CP was
    /// blocked in. Stops at a WAIT_REG_MEM whose condition is not met; the next call starts again from it.
    pub fn execute_ring(&mut self, bus: &mut MemoryBus, ring: &mut RingBuffer) -> Result<usize, String> {
        let mut count = 0;
        self.waiting = true;
        if self.run_ib_stack(bus, &mut count)? == CpFlow::Blocked {
            return Ok(count);
        }

        while ring.has_pending() {
            let pending = ring.pending_dwords();
            if pending == 0 {
                break;
            }

            // Peek packet header to determine total packet size
            let header = ring.peek_words(bus, 1)?[0];
            let packet_dwords = Pm4Packet::size_dwords(header);

            if pending < packet_dwords {
                // Incomplete packet in ring buffer, wait for CPU
                break;
            }

            let words = ring.peek_words(bus, packet_dwords)?;
            match Pm4Packet::decode(&words) {
                Ok((pkt, consumed)) => {
                    if self.execute_packet(bus, &pkt)? == CpFlow::Blocked {
                        return Ok(count);
                    }
                    ring.advance_rptr(consumed);
                    self.packets_executed += 1;
                    count += 1;
                    // an INDIRECT_BUFFER packet pushed a frame: run it (it may block)
                    if self.run_ib_stack(bus, &mut count)? == CpFlow::Blocked {
                        return Ok(count);
                    }
                }
                Err(e) => {
                    println!("  [XENOS CP ERROR] Error decoding PM4 packet: {}", e);
                    ring.advance_rptr(1);
                    break;
                }
            }
        }

        self.waiting = false;
        Ok(count)
    }

    /// Executes the indirect buffers on `ib_stack` until they are done or a packet blocks.
    fn run_ib_stack(&mut self, bus: &mut MemoryBus, count: &mut usize) -> Result<CpFlow, String> {
        while let Some(&frame) = self.ib_stack.last() {
            let depth = self.ib_stack.len() - 1;
            if frame.pos >= frame.len {
                self.ib_stack.pop();
                continue;
            }
            let at = frame.addr + 4 * frame.pos as u64;
            let header = bus.read_u32(at).unwrap_or(0);
            let size = Pm4Packet::size_dwords(header);
            if frame.pos + size > frame.len {
                // a packet running past the end of the IB: counted and reported, then skipped
                self.ib_decode_errors += 1;
                if pm4_log_enabled() || self.draw_log.is_some() {
                    println!("  [XENOS CP] IB 0x{:08X}+{}: packet {:08X} needs {} dwords, {} left", frame.addr, frame.pos, header, size, frame.len - frame.pos);
                }
                self.ib_stack[depth].pos += 1;
                continue;
            }
            let words: Vec<u32> = (0..size).map(|i| bus.read_u32(at + 4 * i as u64).unwrap_or(0)).collect();
            let (pkt, _) = Pm4Packet::decode(&words)?;
            // advance first: a nested INDIRECT_BUFFER pushes its frame on top of this one
            self.ib_stack[depth].pos += size;
            if self.execute_packet(bus, &pkt)? == CpFlow::Blocked {
                self.ib_stack[depth].pos -= size;
                return Ok(CpFlow::Blocked);
            }
            self.packets_executed += 1;
            *count += 1;
        }
        Ok(CpFlow::Continue)
    }

    /// Executes pending PM4 packets and writes back the current RPTR to guest memory
    pub fn execute_ring_with_writeback(
        &mut self,
        bus: &mut MemoryBus,
        ring: &mut RingBuffer,
        writeback_addr: u64,
    ) -> Result<usize, String> {
        let count = self.execute_ring(bus, ring)?;
        if writeback_addr != 0 {
            let _ = bus.write_u32(writeback_addr, ring.rptr as u32);
        }
        Ok(count)
    }

    /// A register write from the command processor (Xenia CommandProcessor::WriteRegister): stores it in the
    /// shared register file, bus.gpu_regs.
    pub fn write_register(&mut self, reg: u32, val: u32, bus: &mut MemoryBus) {
        bus.gpu_regs.set(reg, val);
        // HandleSpecialRegisterWrite
        if (SCRATCH_REG0..SCRATCH_REG0 + 8).contains(&reg) {
            // scratch register writeback: enabled registers are also stored at SCRATCH_ADDR + 4 * n
            let n = reg - SCRATCH_REG0;
            if bus.gpu_regs.get(SCRATCH_UMSK) & (1 << n) != 0 {
                let address = bus.gpu_regs.get(SCRATCH_ADDR).wrapping_add(4 * n);
                let _ = bus.write_u32((address & 0x1FFF_FFFC) as u64, val);
            }
        } else if reg == COHER_STATUS_HOST {
            // marks the range dirty; the next WAIT_REG_MEM on it makes it coherent
            bus.gpu_regs.set(reg, val | 0x8000_0000);
        }
    }

    /// MakeCoherent: a pending COHER_STATUS_HOST request (status, bit 31) completes at once, as nothing is cached.
    fn make_coherent(bus: &mut MemoryBus) {
        if bus.gpu_regs.get(COHER_STATUS_HOST) & 0x8000_0000 != 0 {
            bus.gpu_regs.set(COHER_STATUS_HOST, 0);
        }
    }

    /// IM_LOAD / IM_LOAD_IMMEDIATE: makes `code` the active shader of `shader_type` (0 vertex, 1 pixel).
    fn set_shader(&mut self, shader_type: u32, code: Vec<u32>) {
        if let Some(log) = self.shader_log.as_mut() {
            log.loaded(shader_type, &code);
        }
        match shader_type {
            0 => self.vertex_shader = code,
            1 => self.pixel_shader = code,
            _ => {}
        }
    }

    /// A DRAW packet (Xenia IssueDraw): with RB_MODECONTROL = kCopy a resolve (IssueCopy); otherwise the vertex
    /// shader runs on the draw's indices and the primitives are clipped, projected and rasterized into EDRAM.
    pub fn draw(&mut self, bus: &mut MemoryBus, opcode: u8, data: &[u32]) {
        let dma = opcode == PM4_DRAW_INDX || opcode == PM4_DRAW_INDX_BIN;
        let di = DrawInitiator::decode(data.get(if dma { 1 } else { 0 }).copied().unwrap_or(0));
        if di.num_indices == 0 {
            return;
        }
        if bus.gpu_regs.get(RB_MODECONTROL) & 7 == EDRAM_MODE_COPY {
            let start = std::time::Instant::now();
            self.edram.resolve(bus);
            if let Some((_, count, seconds)) = self.raster_profile.as_mut() {
                *count += 1;
                *seconds += start.elapsed().as_secs_f64();
            }
            return;
        }
        if !crate::diag::config().raster {
            return;
        }
        let Some(state) = RenderState::from_registers(bus) else { return };
        self.edram.check_formats(&state);
        if !matches!(di.prim_type, 0x04 | 0x05 | 0x06 | 0x08 | 0x0D) {
            self.edram.warn_once(format!("primitive type {}", di.prim_type));
            return;
        }
        let Ok(indices) = draw_indices(bus, opcode, data, di.num_indices) else { return };
        let mut sampler = GpuTextureSampler::new(bus);
        // both shaders are decoded once per draw, with this draw's constants
        let vertex_shader = CompiledShader::compile(&self.vertex_shader, ShaderType::Vertex, &GpuShaderEnv { bus, sampler: &mut sampler });
        let pixel_shader = (!self.pixel_shader.is_empty())
            .then(|| CompiledShader::compile(&self.pixel_shader, ShaderType::Pixel, &GpuShaderEnv { bus, sampler: &mut sampler }));
        // the vertex shader runs once per distinct index
        let mut shaded: HashMap<u32, Vertex> = HashMap::new();
        let vertices: Vec<Vertex> = indices
            .iter()
            .map(|&index| {
                shaded
                    .entry(index)
                    .or_insert_with(|| {
                        let mut vs = ShaderState::default();
                        vs.r[0][0] = index as f32;
                        let mut env = GpuShaderEnv { bus, sampler: &mut sampler };
                        let _ = vertex_shader.run(&mut vs, &mut env);
                        Vertex { position: clip_position(bus, vs.exports.position.unwrap_or([0.0, 0.0, 0.0, 1.0])), interpolators: vs.exports.interpolators }
                    })
                    .clone()
            })
            .collect();
        let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
        let start = std::time::Instant::now();
        for tri in assemble_primitives(di.prim_type, &vertices) {
            let polygon: Vec<ScreenVertex> = clip_triangle(bus, &tri).iter().map(|v| project(bus, v)).collect();
            for i in 1..polygon.len().saturating_sub(1) {
                let tri = [polygon[0].clone(), polygon[i].clone(), polygon[i + 1].clone()];
                rasterize_triangle(&mut self.edram, &state, &tri, bus, pixel_shader.as_ref(), threads);
            }
        }
        if let Some((draws, _, _)) = self.raster_profile.as_mut() {
            let entry = draws.entry(crate::gpu::shader_log::shader_hash(&self.pixel_shader)).or_insert((0, 0.0));
            entry.0 += 1;
            entry.1 += start.elapsed().as_secs_f64();
        }
    }

    /// One vertical blank (Xenia GraphicsSystem::MarkVblank): advances the GPU counter. The frame loop then calls
    /// the guest's interrupt callback with source 0.
    pub fn mark_vblank(&mut self) {
        self.gpu_counter = self.gpu_counter.wrapping_add(1);
    }

    /// Executes one decoded PM4 packet (Xenia ExecutePacketType0..3; opcodes from xenos.h).
    /// INDIRECT_BUFFER only pushes its frame on `ib_stack`; `execute_ring` runs it.
    pub fn execute_packet(&mut self, bus: &mut MemoryBus, packet: &Pm4Packet) -> Result<CpFlow, String> {
        let mut flow = CpFlow::Continue;
        if pm4_log_enabled() {
            match packet {
                Pm4Packet::Type0 { base_reg, one_reg, values } => println!("  [PM4] T0 base=0x{:04X}{} n={} v0=0x{:08X}", base_reg, if *one_reg { " one-reg" } else { "" }, values.len(), values.first().copied().unwrap_or(0)),
                Pm4Packet::Type1 { reg1, val1, reg2, val2 } => println!("  [PM4] T1 0x{:04X}=0x{:08X} 0x{:04X}=0x{:08X}", reg1, val1, reg2, val2),
                Pm4Packet::Type2Nop => {}
                Pm4Packet::Type3 { opcode, predicated, data } => {
                    let shown: Vec<String> = data.iter().take(8).map(|d| format!("{:08X}", d)).collect();
                    println!("  [PM4] T3 op=0x{:02X} ({}){} n={} data=[{}]", opcode, Pm4Packet::opcode_name(*opcode), if *predicated { " pred" } else { "" }, data.len(), shown.join(" "));
                }
            }
        }
        match packet {
            Pm4Packet::Type0 { base_reg, one_reg, values } => {
                for (i, &val) in values.iter().enumerate() {
                    let reg = if *one_reg { *base_reg } else { base_reg + i as u32 };
                    self.write_register(reg, val, bus);
                }
            }
            Pm4Packet::Type1 { reg1, val1, reg2, val2 } => {
                self.write_register(*reg1, *val1, bus);
                self.write_register(*reg2, *val2, bus);
            }
            Pm4Packet::Type2Nop => {}
            Pm4Packet::Type3 { opcode, predicated, data } => {
                *self.real_opcode_counts.entry(*opcode).or_insert(0) += 1;
                // Predicated packets run only if a selected bin is enabled; predicated swaps never run.
                if *predicated && ((self.bin_select & self.bin_mask) == 0 || *opcode == PM4_XE_SWAP) {
                    return Ok(CpFlow::Continue);
                }
                let word = |i: usize| data.get(i).copied().unwrap_or(0);
                match *opcode {
                    PM4_ME_INIT | PM4_NOP | PM4_WAIT_FOR_IDLE | PM4_WAIT_IB_PFD_COMPLETE | PM4_CONTEXT_UPDATE => {}
                    PM4_INDIRECT_BUFFER | PM4_INDIRECT_BUFFER_PFD => {
                        // [list_ptr, list_length]; length in dwords, 20 bits
                        let ib_addr = word(0) as u64;
                        let ib_count = (word(1) & 0xF_FFFF) as usize;
                        if ib_addr != 0 && ib_count > 0 {
                            self.ib_stack.push(IbFrame { addr: ib_addr, len: ib_count, pos: 0 });
                        }
                    }
                    PM4_REG_RMW => {
                        // [rmw_info, and_mask, or_mask]: reg = rmw_info & 0x1FFF; bit 31 = AND with a register
                        // (and_mask & 0x1FFF) instead of the immediate, bit 30 = same for OR
                        let (info, and_mask, or_mask) = (word(0), word(1), word(2));
                        let reg = info & 0x1FFF;
                        let and_value = if (info >> 31) & 1 != 0 { bus.gpu_regs.get(and_mask & 0x1FFF) } else { and_mask };
                        let or_value = if (info >> 30) & 1 != 0 { bus.gpu_regs.get(or_mask & 0x1FFF) } else { or_mask };
                        let value = (bus.gpu_regs.get(reg) & and_value) | or_value;
                        self.write_register(reg, value, bus);
                    }
                    PM4_REG_TO_MEM => {
                        // [register, address | endian]
                        let value = bus.gpu_regs.get(word(0));
                        gpu_store(bus, word(1), value);
                    }
                    PM4_WAIT_REG_MEM => {
                        // [wait_info, poll_reg_or_addr, reference, mask, wait_interval]
                        if data.len() >= 4 {
                            let wait_info = data[0];
                            let poll_addr = data[1];
                            let reference = data[2];
                            let mask = data[3];

                            let is_mem = (wait_info & 0x10) != 0;
                            let function = wait_info & 0x07;
                            let current_val = if is_mem {
                                gpu_load(bus, poll_addr)
                            } else {
                                if poll_addr == COHER_STATUS_HOST {
                                    Self::make_coherent(bus);
                                }
                                bus.gpu_regs.get(poll_addr)
                            };

                            let masked = current_val & mask;
                            let satisfied = match function {
                                0 => false,        // never
                                1 => masked < reference,
                                2 => masked <= reference,
                                3 => masked == reference,
                                4 => masked != reference,
                                5 => masked >= reference,
                                6 => masked > reference,
                                _ => true,         // always
                            };

                            // Not met: the CP stops here and executes this packet again on its next run (Xenia's CP
                            // thread polls/sleeps in place); the CPU threads in between can make it true.
                            if !satisfied {
                                self.wait_blocks += 1;
                                if pm4_log_enabled() || self.draw_log.is_some() {
                                    println!("  [XENOS CP] WAIT_REG_MEM blocks: {} 0x{:08X} = 0x{:08X}, wants (& 0x{:08X}) fn {} 0x{:08X}",
                                        if is_mem { "mem" } else { "reg" }, poll_addr, current_val, mask, function, reference);
                                }
                                flow = CpFlow::Blocked;
                            }
                        }
                    }
                    PM4_MEM_WRITE => {
                        // [address | endian, data...]: count - 1 consecutive dwords
                        let address = word(0);
                        for (i, &v) in data.iter().skip(1).enumerate() {
                            gpu_store(bus, address.wrapping_add(4 * i as u32), v);
                        }
                    }
                    PM4_EVENT_WRITE_SHD => {
                        // EVENT_WRITE_SHD: [initiator, address, value]. Low 2 address bits = endianness of the stored word.
                        let (initiator, address) = (word(0), word(1));
                        bus.gpu_regs.set(VGT_EVENT_INITIATOR, initiator & 0x3F);
                        let value = if (initiator >> 31) & 1 != 0 { self.gpu_counter } else { word(2) };
                        // Addresses above 0x1FFFFFFF inside the writeback window (WRITEBACK_START/SIZE, once the
                        // guest set them) land at virtual 0x7F000000 + offset.
                        let writeback_base = bus.gpu_regs.get(WRITEBACK_START);
                        let offset = (address & !3).wrapping_sub(writeback_base);
                        if address & !3 > 0x1FFF_FFFF && writeback_base != 0 && offset < bus.gpu_regs.get(WRITEBACK_SIZE) {
                            let _ = bus.write_u32(0x7F00_0000 + offset as u64, guest_endian_swap(value, address));
                        } else {
                            gpu_store(bus, address, value);
                        }
                    }
                    PM4_EVENT_WRITE => {
                        // EVENT_WRITE: [initiator] (+ optional address, unused by the dash)
                        bus.gpu_regs.set(VGT_EVENT_INITIATOR, word(0) & 0x3F);
                    }
                    PM4_COND_WRITE => {
                        // COND_WRITE: [wait_info, poll_addr, reference, mask, write_addr, write_data]
                        if data.len() >= 6 {
                            let wait_info = data[0];
                            let is_mem = (wait_info & 0x10) != 0;
                            let cur = if is_mem { gpu_load(bus, data[1]) } else { bus.gpu_regs.get(data[1]) };
                            let masked = cur & data[3];
                            let ok = match wait_info & 7 {
                                0 => false, 1 => masked < data[2], 2 => masked <= data[2], 3 => masked == data[2],
                                4 => masked != data[2], 5 => masked >= data[2], 6 => masked > data[2], _ => true,
                            };
                            if ok {
                                if (wait_info & 0x100) != 0 { gpu_store(bus, data[4], data[5]); }
                                else { self.write_register(data[4], data[5], bus); }
                            }
                        }
                    }
                    PM4_INTERRUPT => {
                        // INTERRUPT: [cpu_mask]; one CP interrupt (callback source 1) per set bit 0..5
                        // (Xenia ExecutePacketType3_INTERRUPT -> DispatchInterruptCallback(1, n)).
                        let cpu_mask = word(0);
                        for n in 0..6 {
                            if cpu_mask & (1 << n) != 0 {
                                self.pending_cp_interrupts.push(n);
                            }
                        }
                    }
                    PM4_XE_SWAP => {
                        // XE_SWAP (Xenia ExecutePacketType3_XE_SWAP): [signature 'SWAP', frontbuffer physical, width,
                        // height, ...]; issues the swap and advances the GPU counter.
                        if data.first() == Some(&crate::kernel::video::SWAP_SIGNATURE) && data.len() >= 4 {
                            self.last_swap = Some((data[1], data[2], data[3]));
                            self.swaps += 1;
                            if let Some((draws, resolves, resolve_seconds)) = self.raster_profile.as_ref() {
                                if self.swaps % 100 == 0 {
                                    let mut rows: Vec<_> = draws.iter().collect();
                                    rows.sort_by(|a, b| b.1 .1.total_cmp(&a.1 .1));
                                    let total: f64 = rows.iter().map(|r| r.1 .1).sum::<f64>() + resolve_seconds;
                                    println!("  [RASTER PROFILE] {} swaps: {:.2} s ({} resolves {:.2} s)", self.swaps, total, resolves, resolve_seconds);
                                    for (hash, (n, seconds)) in rows {
                                        println!("    ps {:016x}: {} draws {:.2} s", hash, n, seconds);
                                    }
                                }
                            }
                        }
                        self.gpu_counter = self.gpu_counter.wrapping_add(1);
                    }
                    PM4_DRAW_INDX | PM4_DRAW_INDX_2 | PM4_DRAW_INDX_BIN | PM4_DRAW_INDX_2_BIN => {
                        self.real_draws += 1;
                        if let Some(log) = self.draw_log.as_mut() {
                            log.log(bus, *opcode, *predicated, data, &self.vertex_shader, &self.pixel_shader);
                        }
                        if let Some(exec) = self.shader_exec.as_mut() {
                            exec.draw(bus, *opcode, data, &self.vertex_shader, &self.pixel_shader);
                        }
                        self.draw(bus, *opcode, data);
                    }
                    PM4_IM_LOAD_IMMEDIATE => {
                        // [shader type (0 vertex, 1 pixel), start << 16 | size in dwords, microcode...]
                        let size = (word(1) & 0xFFFF) as usize;
                        let code = data.iter().skip(2).take(size).copied().collect();
                        self.set_shader(word(0), code);
                    }
                    PM4_IM_LOAD => {
                        // [physical address | shader type, start << 16 | size in dwords]
                        let address = (word(0) & 0x1FFF_FFFC) as u64;
                        let size = word(1) & 0xFFFF;
                        let code = (0..size).map(|i| bus.read_u32(address + 4 * i as u64).unwrap_or(0)).collect();
                        self.set_shader(word(0) & 3, code);
                    }
                    PM4_SET_CONSTANT => {
                        // [offset_type, values...]: index = bits 0..10, type = bits 16..23
                        let offset_type = word(0);
                        if let Some(base) = constant_register_base((offset_type >> 16) & 0xFF) {
                            let first = base + (offset_type & 0x7FF);
                            for (i, &v) in data.iter().skip(1).enumerate() {
                                self.write_register(first + i as u32, v, bus);
                            }
                        }
                    }
                    PM4_SET_CONSTANT2 | PM4_SET_SHADER_CONSTANTS => {
                        // [offset_type, values...]: register index = low 16 bits
                        let first = word(0) & 0xFFFF;
                        for (i, &v) in data.iter().skip(1).enumerate() {
                            self.write_register(first + i as u32, v, bus);
                        }
                    }
                    PM4_LOAD_ALU_CONSTANT => {
                        // [address (physical), offset_type, size_dwords]: registers loaded from memory
                        let address = (word(0) & 0x3FFF_FFFF) as u64;
                        let offset_type = word(1);
                        let size = word(2) & 0xFFF;
                        if let Some(base) = constant_register_base((offset_type >> 16) & 0xFF) {
                            let first = base + (offset_type & 0x7FF);
                            for i in 0..size {
                                let v = bus.read_u32(address + 4 * i as u64).unwrap_or(0);
                                self.write_register(first + i, v, bus);
                            }
                        }
                    }
                    PM4_SET_BIN_MASK_LO => self.bin_mask = (self.bin_mask & !0xFFFF_FFFF) | word(0) as u64,
                    PM4_SET_BIN_MASK_HI => self.bin_mask = (self.bin_mask & 0xFFFF_FFFF) | (word(0) as u64) << 32,
                    PM4_SET_BIN_SELECT_LO => self.bin_select = (self.bin_select & !0xFFFF_FFFF) | word(0) as u64,
                    PM4_SET_BIN_SELECT_HI => self.bin_select = (self.bin_select & 0xFFFF_FFFF) | (word(0) as u64) << 32,
                    PM4_SET_BIN_MASK => self.bin_mask = (word(0) as u64) << 32 | word(1) as u64,
                    PM4_SET_BIN_SELECT => self.bin_select = (word(0) as u64) << 32 | word(1) as u64,
                    PM4_INVALIDATE_STATE | PM4_VIZ_QUERY | PM4_EVENT_WRITE_EXT
                    | PM4_EVENT_WRITE_ZPD => {
                        // Implemented by Xenia but with no effect yet on this software GPU (state invalidation, viz
                        // queries, extents, z-pass counts: Phase 4).
                    }
                    _ => {
                        // Unimplemented in Xenia too (HitUnimplementedOpcode); counted in real_opcode_counts.
                    }
                }
            }
        }
        Ok(flow)
    }

}


const VGT_EVENT_INITIATOR: u32 = 0x21F9;
const SCRATCH_UMSK: u32 = 0x01DC;
const SCRATCH_ADDR: u32 = 0x01DD;
const SCRATCH_REG0: u32 = 0x0578;
const COHER_STATUS_HOST: u32 = 0x0A31;
const WRITEBACK_START: u32 = 0x0A04;
const WRITEBACK_SIZE: u32 = 0x0A05;

/// A 32-bit value the GPU moves between memory and itself with endian mode `endian & 3` (xenos::Endian, the low
/// address bits), as the big-endian guest sees it. Xenia stores GpuSwap(v, endian) with a little-endian host store
/// (and loads the same way), so through lwz/stw: kNone = byte swap, k8in16 = half-words swapped, k8in32 = unchanged,
/// k16in32 = bytes swapped inside each half-word. Each mode is its own inverse.
pub(crate) fn guest_endian_swap(value: u32, endian: u32) -> u32 {
    match endian & 3 {
        0 => value.swap_bytes(),
        1 => value.rotate_left(16),
        2 => value,
        _ => ((value << 8) & 0xFF00_FF00) | ((value >> 8) & 0x00FF_00FF),
    }
}

/// GPU store to `address` (physical; bits 0..1 = endian mode), as Xenia's TranslatePhysical + GpuSwap.
fn gpu_store(bus: &mut MemoryBus, address: u32, value: u32) {
    let _ = bus.write_u32((address & 0x1FFF_FFFC) as u64, guest_endian_swap(value, address));
}

/// GPU load from `address` (physical; bits 0..1 = endian mode).
fn gpu_load(bus: &MemoryBus, address: u32) -> u32 {
    guest_endian_swap(bus.read_u32((address & 0x1FFF_FFFC) as u64).unwrap_or(0), address)
}

/// First register of a SET_CONSTANT / LOAD_ALU_CONSTANT type (Xenia Write{ALU,Fetch,Bool,Loop,REGISTERS}Range*).
fn constant_register_base(kind: u32) -> Option<u32> {
    match kind {
        0 => Some(0x4000), // ALU
        1 => Some(0x4800), // FETCH
        2 => Some(0x4900), // BOOL
        3 => Some(0x4908), // LOOP
        4 => Some(0x2000), // REGISTERS
        _ => None,
    }
}

/// XENON_PM4_LOG=1 prints every PM4 packet the Command Processor executes (cached after first call).
fn pm4_log_enabled() -> bool {
    use std::sync::OnceLock;
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| crate::diag::config().pm4_log)
}

#[cfg(test)]
mod cp_tests {
    use super::*;

    fn t3(opcode: u8, data: &[u32]) -> Pm4Packet {
        Pm4Packet::Type3 { opcode, predicated: false, data: data.to_vec() }
    }

    #[test]
    fn interrupt_queues_one_cp_interrupt_per_cpu_bit() {
        let mut gpu = XenosGpu::new();
        let mut bus = MemoryBus::new();
        gpu.execute_packet(&mut bus, &t3(0x54, &[0b10_0101])).unwrap();
        assert_eq!(gpu.pending_cp_interrupts, vec![0, 2, 5]);
        // bits above 5 are ignored (six hardware threads)
        gpu.execute_packet(&mut bus, &t3(0x54, &[0xC0])).unwrap();
        assert_eq!(gpu.pending_cp_interrupts, vec![0, 2, 5]);
    }

    #[test]
    fn cp_and_mmio_share_one_register_file() {
        let mut gpu = XenosGpu::new();
        let mut bus = MemoryBus::new();
        // Type-0: two registers from 0x2000 (a register outside the fixed MMIO ones)
        gpu.execute_packet(&mut bus, &Pm4Packet::Type0 { base_reg: 0x2000, one_reg: false, values: vec![0x1234, 0x5678] }).unwrap();
        assert_eq!(bus.read_u32(0x7FC8_0000 + 0x2000 * 4).unwrap(), 0x1234);
        assert_eq!(bus.read_u32(0x7FC8_0000 + 0x2001 * 4).unwrap(), 0x5678);
        // a CPU MMIO store is what REG_RMW (0x21) reads back: (old & and) | or
        bus.write_u32(0x7FC8_0000 + 0x0200 * 4, 0xFF00).unwrap();
        gpu.execute_packet(&mut bus, &t3(0x21, &[0x0200, 0x0F00, 0x0003])).unwrap();
        assert_eq!(bus.gpu_regs.get(0x0200), 0x0F03);
        assert_eq!(bus.read_u32(0x7FC8_0800).unwrap(), 0x0F03);
    }

    #[test]
    fn type0_one_reg_writes_every_value_to_the_base_register() {
        let mut gpu = XenosGpu::new();
        let mut bus = MemoryBus::new();
        gpu.execute_packet(&mut bus, &Pm4Packet::Type0 { base_reg: 0x2100, one_reg: true, values: vec![1, 2, 3] }).unwrap();
        assert_eq!((bus.gpu_regs.get(0x2100), bus.gpu_regs.get(0x2101)), (3, 0));
    }

    #[test]
    fn predicated_packets_follow_bin_select_and_mask() {
        let mut gpu = XenosGpu::new();
        let mut bus = MemoryBus::new();
        let pred = |opcode: u8, data: &[u32]| Pm4Packet::Type3 { opcode, predicated: true, data: data.to_vec() };
        // default mask/select 0xFFFFFFFF: predicated INTERRUPT runs, predicated XE_SWAP never does
        gpu.execute_packet(&mut bus, &pred(PM4_INTERRUPT, &[1])).unwrap();
        gpu.execute_packet(&mut bus, &pred(PM4_XE_SWAP, &[0x5357_4150, 0, 1, 1])).unwrap();
        assert_eq!((gpu.pending_cp_interrupts.len(), gpu.swaps, gpu.gpu_counter), (1, 0, 0));
        // SET_BIN_SELECT (hi, lo) disjoint from the mask: skipped
        gpu.execute_packet(&mut bus, &t3(PM4_SET_BIN_SELECT, &[1, 0])).unwrap();
        assert_eq!(gpu.bin_select, 1 << 32);
        gpu.execute_packet(&mut bus, &pred(PM4_INTERRUPT, &[1])).unwrap();
        assert_eq!(gpu.pending_cp_interrupts.len(), 1);
        gpu.execute_packet(&mut bus, &t3(PM4_SET_BIN_MASK_HI, &[1])).unwrap();
        gpu.execute_packet(&mut bus, &pred(PM4_INTERRUPT, &[1])).unwrap();
        assert_eq!(gpu.pending_cp_interrupts.len(), 2);
    }

    #[test]
    fn constants_land_in_the_register_file() {
        let mut gpu = XenosGpu::new();
        let mut bus = MemoryBus::new();
        // SET_CONSTANT type 0 (ALU) index 8 -> 0x4008; type 1 (FETCH) index 2 -> 0x4802; type 4 -> 0x2000 + index
        gpu.execute_packet(&mut bus, &t3(PM4_SET_CONSTANT, &[0x0000_0008, 0xA, 0xB])).unwrap();
        gpu.execute_packet(&mut bus, &t3(PM4_SET_CONSTANT, &[0x0001_0002, 0xC])).unwrap();
        gpu.execute_packet(&mut bus, &t3(PM4_SET_CONSTANT, &[0x0004_0081, 0xD])).unwrap();
        assert_eq!((bus.gpu_regs.get(0x4008), bus.gpu_regs.get(0x4009), bus.gpu_regs.get(0x4802), bus.gpu_regs.get(0x2081)), (0xA, 0xB, 0xC, 0xD));
        // LOAD_ALU_CONSTANT: [physical address, offset_type, size] -> registers from memory
        bus.write_u32(0x0100_0000, 0x11).unwrap();
        bus.write_u32(0x0100_0004, 0x22).unwrap();
        gpu.execute_packet(&mut bus, &t3(PM4_LOAD_ALU_CONSTANT, &[0x0100_0000, 0x0000_0010, 2])).unwrap();
        assert_eq!((bus.gpu_regs.get(0x4010), bus.gpu_regs.get(0x4011)), (0x11, 0x22));
        // SET_CONSTANT2: register index in the low 16 bits
        gpu.execute_packet(&mut bus, &t3(PM4_SET_CONSTANT2, &[0x0000_2300, 0xE])).unwrap();
        assert_eq!(bus.gpu_regs.get(0x2300), 0xE);
        // the old invented opcode 0x22 is DRAW_INDX: counted as a draw, writes no register
        let before = bus.gpu_regs.non_zero().count();
        gpu.execute_packet(&mut bus, &t3(PM4_DRAW_INDX, &[0x84, 0x1234])).unwrap();
        assert_eq!((gpu.real_draws, bus.gpu_regs.non_zero().count()), (1, before));
    }

    #[test]
    fn gpu_memory_writes_honor_the_endian_mode() {
        let mut gpu = XenosGpu::new();
        let mut bus = MemoryBus::new();
        const A: u32 = 0x0100_0000;
        // MEM_WRITE: N dwords from the address; endian 2 (k8in32) keeps the value as lwz sees it
        gpu.execute_packet(&mut bus, &t3(PM4_MEM_WRITE, &[A | 2, 0xAABB_CCDD, 0x1122_3344, 0x5566_7788])).unwrap();
        assert_eq!((bus.read_u32(A as u64).unwrap(), bus.read_u32(A as u64 + 8).unwrap()), (0xAABB_CCDD, 0x5566_7788));
        // other modes: none = byte swap, k8in16 = halves swapped, k16in32 = bytes swapped inside halves
        for (endian, seen) in [(0, 0xDDCC_BBAA), (1, 0xCCDD_AABB), (3, 0xBBAA_DDCC)] {
            gpu.execute_packet(&mut bus, &t3(PM4_MEM_WRITE, &[A | endian, 0xAABB_CCDD])).unwrap();
            assert_eq!(bus.read_u32(A as u64).unwrap(), seen, "endian {}", endian);
            assert_eq!(gpu_load(&bus, A | endian), 0xAABB_CCDD, "loads undo the same mode");
        }
        // COND_WRITE polls memory with endian 0 and writes memory with endian 0
        bus.write_u32(A as u64, 0x0300_0000).unwrap();
        gpu.execute_packet(&mut bus, &t3(PM4_COND_WRITE, &[0x113, A, 3, 0xFF, (A + 4), 7])).unwrap();
        assert_eq!(bus.read_u32(A as u64 + 4).unwrap(), 0x0700_0000);
        // EVENT_WRITE_SHD with endian 0, and the initiator lands in VGT_EVENT_INITIATOR
        gpu.execute_packet(&mut bus, &t3(PM4_EVENT_WRITE_SHD, &[0x14, A + 8, 9])).unwrap();
        assert_eq!((bus.read_u32(A as u64 + 8).unwrap(), bus.gpu_regs.get(0x21F9)), (0x0900_0000, 0x14));
        // REG_TO_MEM
        bus.gpu_regs.set(0x2000, 0x1234_5678);
        gpu.execute_packet(&mut bus, &t3(PM4_REG_TO_MEM, &[0x2000, (A + 12) | 2])).unwrap();
        assert_eq!(bus.read_u32(A as u64 + 12).unwrap(), 0x1234_5678);
    }

    #[test]
    fn event_write_shd_uses_the_writeback_window() {
        let mut gpu = XenosGpu::new();
        let mut bus = MemoryBus::new();
        // no window set: an address above 0x1FFFFFFF goes to its physical alias
        gpu.execute_packet(&mut bus, &t3(PM4_EVENT_WRITE_SHD, &[0, 0xC000_0102, 5])).unwrap();
        assert_eq!(bus.read_u32(0x0000_0100).unwrap(), 5);
        bus.gpu_regs.set(0x0A04, 0xC000_0000);
        bus.gpu_regs.set(0x0A05, 0x1000);
        gpu.execute_packet(&mut bus, &t3(PM4_EVENT_WRITE_SHD, &[0, 0xC000_0042, 6])).unwrap();
        assert_eq!(bus.read_u32(0x7F00_0040).unwrap(), 6);
    }

    #[test]
    fn writeback_window_set_by_type0_has_no_other_side_effect() {
        // 0x0A00..0x0A5F (FB_START.., WRITEBACK_START/SIZE) used to be decoded as invented texture descriptors
        let mut gpu = XenosGpu::new();
        let mut bus = MemoryBus::new();
        gpu.execute_packet(&mut bus, &Pm4Packet::Type0 { base_reg: 0x0A04, one_reg: false, values: vec![0xC000_0000, 0x1000] }).unwrap();
        assert_eq!(bus.gpu_regs.non_zero().collect::<Vec<_>>(), vec![(0x0A04, 0xC000_0000), (0x0A05, 0x1000)]);
        gpu.execute_packet(&mut bus, &t3(PM4_EVENT_WRITE_SHD, &[0, 0xC000_0012, 7])).unwrap();
        assert_eq!(bus.read_u32(0x7F00_0010).unwrap(), 7);
    }

    #[test]
    fn shader_loads_set_the_active_shaders() {
        let mut gpu = XenosGpu::new();
        let mut bus = MemoryBus::new();
        // IM_LOAD_IMMEDIATE, pixel shader of 2 dwords (the payload may be padded past the size)
        gpu.execute_packet(&mut bus, &t3(PM4_IM_LOAD_IMMEDIATE, &[1, 2, 0xAAAA_0001, 0xAAAA_0002, 0])).unwrap();
        assert_eq!(gpu.pixel_shader, vec![0xAAAA_0001, 0xAAAA_0002]);
        // IM_LOAD of a vertex shader from memory
        bus.write_u32(0x0100_0000, 0xBBBB_0001).unwrap();
        bus.write_u32(0x0100_0004, 0xBBBB_0002).unwrap();
        bus.write_u32(0x0100_0008, 0xBBBB_0003).unwrap();
        gpu.execute_packet(&mut bus, &t3(PM4_IM_LOAD, &[0x0100_0000, 3])).unwrap();
        assert_eq!(gpu.vertex_shader, vec![0xBBBB_0001, 0xBBBB_0002, 0xBBBB_0003]);
    }

    #[test]
    fn scratch_registers_write_back_and_coherency_waits_complete() {
        let mut gpu = XenosGpu::new();
        let mut bus = MemoryBus::new();
        // the dash's setup: SCRATCH_ADDR 0x10103000, SCRATCH_UMSK 0x37 (registers 0-2, 4, 5)
        gpu.execute_packet(&mut bus, &Pm4Packet::Type0 { base_reg: 0x01DD, one_reg: false, values: vec![0x1010_3000] }).unwrap();
        gpu.execute_packet(&mut bus, &Pm4Packet::Type0 { base_reg: 0x01DC, one_reg: false, values: vec![0x0002_0037] }).unwrap();
        gpu.execute_packet(&mut bus, &Pm4Packet::Type0 { base_reg: 0x057C, one_reg: false, values: vec![0x921A_4D20, 0x100] }).unwrap();
        gpu.execute_packet(&mut bus, &Pm4Packet::Type0 { base_reg: 0x0578, one_reg: false, values: vec![4] }).unwrap();
        gpu.execute_packet(&mut bus, &Pm4Packet::Type0 { base_reg: 0x057B, one_reg: false, values: vec![0xDEAD] }).unwrap();
        assert_eq!(bus.read_u32(0x1010_3000).unwrap(), 4, "SCRATCH_REG0");
        assert_eq!((bus.read_u32(0x1010_3010).unwrap(), bus.read_u32(0x1010_3014).unwrap()), (0x921A_4D20, 0x100), "REG4/REG5");
        assert_eq!(bus.read_u32(0x1010_300C).unwrap(), 0, "REG3 is masked off");
        // the dash's waits on them (endian 2) are met without forcing
        for (addr, value) in [(0x1010_3002u32, 4u32), (0x1010_3016, 0x100), (0x1010_3012, 0x921A_4D20)] {
            gpu.execute_packet(&mut bus, &t3(PM4_WAIT_REG_MEM, &[0x13, addr, value, 0xFFFF_FFFF, 0x100])).unwrap();
        }
        // COHER_STATUS_HOST write marks it pending; WAIT_REG_MEM (status bit == 0) makes it coherent
        gpu.execute_packet(&mut bus, &Pm4Packet::Type0 { base_reg: 0x0A31, one_reg: false, values: vec![0x0100_0000] }).unwrap();
        assert_eq!(bus.gpu_regs.get(0x0A31), 0x8100_0000);
        gpu.execute_packet(&mut bus, &t3(PM4_WAIT_REG_MEM, &[3, 0x0A31, 0, 0x8000_0000, 8])).unwrap();
        assert_eq!((bus.gpu_regs.get(0x0A31), gpu.wait_blocks), (0, 0));
    }

    #[test]
    fn unmet_wait_blocks_the_ring_and_resumes() {
        let mut gpu = XenosGpu::new();
        let mut bus = MemoryBus::new();
        let mut ring = RingBuffer::new(0x0100_0000, 0x1000);
        const FLAG: u32 = 0x0200_0000;
        // WAIT_REG_MEM memory FLAG (endian 2) == 5, then MEM_WRITE 0x0200_0010 = 7
        ring.write_packet(&mut bus, &build_type3(PM4_WAIT_REG_MEM, &[0x13, FLAG | 2, 5, 0xFFFF_FFFF, 0x100])).unwrap();
        ring.write_packet(&mut bus, &build_type3(PM4_MEM_WRITE, &[0x0200_0010 | 2, 7])).unwrap();
        assert_eq!(gpu.execute_ring(&mut bus, &mut ring).unwrap(), 0);
        assert_eq!((ring.rptr, gpu.waiting, gpu.wait_blocks, bus.read_u32(0x0200_0010).unwrap()), (0, true, 1, 0));
        // still not met: stays blocked, the flag is not forced
        gpu.execute_ring(&mut bus, &mut ring).unwrap();
        assert_eq!((ring.rptr, bus.read_u32(FLAG as u64).unwrap()), (0, 0));
        // the CPU sets it: the CP goes on
        bus.write_u32(FLAG as u64, 5).unwrap();
        assert_eq!(gpu.execute_ring(&mut bus, &mut ring).unwrap(), 2);
        assert_eq!((ring.rptr, ring.wptr, gpu.waiting, bus.read_u32(0x0200_0010).unwrap()), (9, 9, false, 7));
    }

    #[test]
    fn a_wait_inside_nested_indirect_buffers_resumes_in_place() {
        let mut gpu = XenosGpu::new();
        let mut bus = MemoryBus::new();
        let mut ring = RingBuffer::new(0x0100_0000, 0x1000);
        const FLAG: u32 = 0x0200_0000;
        // inner IB: MEM_WRITE A = 1; WAIT FLAG == 1; MEM_WRITE B = 2
        let inner = [build_type3(PM4_MEM_WRITE, &[0x0200_0100 | 2, 1]), build_type3(PM4_WAIT_REG_MEM, &[0x13, FLAG | 2, 1, 0xFFFF_FFFF, 0x100]),
            build_type3(PM4_MEM_WRITE, &[0x0200_0104 | 2, 2])].concat();
        for (i, w) in inner.iter().enumerate() {
            bus.write_u32(0x0300_0000 + 4 * i as u64, *w).unwrap();
        }
        // outer IB: IB(inner); MEM_WRITE C = 3
        let outer = [build_type3(PM4_INDIRECT_BUFFER, &[0x0300_0000, inner.len() as u32]), build_type3(PM4_MEM_WRITE, &[0x0200_0108 | 2, 3])].concat();
        for (i, w) in outer.iter().enumerate() {
            bus.write_u32(0x0300_1000 + 4 * i as u64, *w).unwrap();
        }
        ring.write_packet(&mut bus, &build_type3(PM4_INDIRECT_BUFFER, &[0x0300_1000, outer.len() as u32])).unwrap();
        ring.write_packet(&mut bus, &build_type3(PM4_MEM_WRITE, &[0x0200_010C | 2, 4])).unwrap();
        gpu.execute_ring(&mut bus, &mut ring).unwrap();
        let read = |bus: &MemoryBus, a: u64| bus.read_u32(a).unwrap();
        assert_eq!((read(&bus, 0x0200_0100), read(&bus, 0x0200_0104), read(&bus, 0x0200_0108), read(&bus, 0x0200_010C)), (1, 0, 0, 0));
        assert_eq!((gpu.ib_stack.len(), gpu.ib_stack[1].pos, ring.rptr), (2, 3, 3), "blocked at the WAIT of the inner IB");
        bus.write_u32(FLAG as u64, 1).unwrap();
        gpu.execute_ring(&mut bus, &mut ring).unwrap();
        assert_eq!((read(&bus, 0x0200_0100), read(&bus, 0x0200_0104), read(&bus, 0x0200_0108), read(&bus, 0x0200_010C)), (1, 2, 3, 4));
        assert_eq!((gpu.ib_stack.len(), gpu.waiting, ring.rptr), (0, false, 6));
    }

    #[test]
    fn the_cp_interrupt_handler_sees_its_scratch_value() {
        // the dash's sequence: SCRATCH_REG4 = handler, SCRATCH_REG0 = 4, INTERRUPT, wait for the CPU to clear
        // scratch 0, then SCRATCH_REG4 = 0x0BADF00D
        let mut gpu = XenosGpu::new();
        let mut bus = MemoryBus::new();
        let mut ring = RingBuffer::new(0x0100_0000, 0x1000);
        for p in [build_type0(0x01DD, &[0x1010_3000]), build_type0(0x01DC, &[0x37]), build_type0(0x057C, &[0x921A_4D20]),
            build_type0(0x0578, &[4]), build_type3(PM4_INTERRUPT, &[4]),
            build_type3(PM4_WAIT_REG_MEM, &[0x13, 0x1010_3002, 0, 0xFFFF_FFFF, 0x100]), build_type0(0x057C, &[0x0BAD_F00D])] {
            ring.write_packet(&mut bus, &p).unwrap();
        }
        gpu.execute_ring(&mut bus, &mut ring).unwrap();
        assert_eq!(gpu.pending_cp_interrupts, vec![2]);
        assert_eq!(bus.read_u32(0x1010_3010).unwrap(), 0x921A_4D20, "the callback reads the handler, not 0x0BADF00D");
        // the CPU's handler acknowledges by clearing scratch 0
        bus.write_u32(0x1010_3000, 0).unwrap();
        gpu.execute_ring(&mut bus, &mut ring).unwrap();
        assert_eq!((bus.read_u32(0x1010_3010).unwrap(), gpu.waiting), (0x0BAD_F00D, false));
    }

    #[test]
    fn reg_rmw_register_operands() {
        let mut gpu = XenosGpu::new();
        let mut bus = MemoryBus::new();
        bus.gpu_regs.set(0x100, 0xFFFF);
        bus.gpu_regs.set(0x101, 0x0F0F);
        bus.gpu_regs.set(0x102, 0x3000_0000);
        // AND with register 0x101, OR with register 0x102
        gpu.execute_packet(&mut bus, &t3(PM4_REG_RMW, &[0xC000_0100, 0x101, 0x102])).unwrap();
        assert_eq!(bus.gpu_regs.get(0x100), 0x3000_0F0F);
        // immediates
        gpu.execute_packet(&mut bus, &t3(PM4_REG_RMW, &[0x100, 0xFF, 0x100])).unwrap();
        assert_eq!(bus.gpu_regs.get(0x100), 0x10F);
    }

    #[test]
    fn counter_advances_on_vblank_and_swap_not_on_event_write_shd() {
        let mut gpu = XenosGpu::new();
        let mut bus = MemoryBus::new();
        const FENCE: u64 = 0x1010_2000;
        // the dash's fence write: [initiator 3, address | endian 2, value]
        gpu.execute_packet(&mut bus, &t3(0x58, &[3, 0x1010_2002, 0xB])).unwrap();
        assert_eq!((bus.read_u32(FENCE).unwrap(), gpu.gpu_counter), (0xB, 0));
        gpu.mark_vblank();
        gpu.mark_vblank();
        gpu.execute_packet(&mut bus, &t3(0x64, &[0x5357_4150, 0x1031_4000, 1280, 720])).unwrap();
        assert_eq!(gpu.gpu_counter, 3);
        assert_eq!((gpu.swaps, gpu.last_swap), (1, Some((0x1031_4000, 1280, 720))));
        // initiator bit 31 writes the counter instead of the value
        gpu.execute_packet(&mut bus, &t3(0x58, &[0x8000_0003, 0x1010_2002, 0xDEAD])).unwrap();
        assert_eq!((bus.read_u32(FENCE).unwrap(), gpu.gpu_counter), (3, 3));
    }

    #[test]
    fn swap_does_not_write_guest_memory() {
        let mut gpu = XenosGpu::new();
        let mut bus = MemoryBus::new();
        bus.write_u32(0x0040_0000, 0x1234_5678).unwrap();
        gpu.execute_packet(&mut bus, &t3(PM4_XE_SWAP, &[0x5357_4150, 0x0040_0000, 1280, 720])).unwrap();
        assert_eq!(gpu.last_swap, Some((0x0040_0000, 1280, 720)));
        assert_eq!(bus.read_u32(0x0040_0000).unwrap(), 0x1234_5678);
    }
}
