use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::time::Duration;

pub struct XbdmClient {
    reader: BufReader<TcpStream>,
}

impl XbdmClient {
    pub fn connect(ip: &str) -> Result<Self, String> {
        let stream = TcpStream::connect(format!("{}:730", ip))
            .map_err(|e| format!("Falha ao conectar no Xbox 360 ({}:730): {}", ip, e))?;
        
        stream.set_read_timeout(Some(Duration::from_millis(3000))).ok();
        stream.set_write_timeout(Some(Duration::from_millis(3000))).ok();

        let mut client = Self {
            reader: BufReader::new(stream),
        };

        loop {
            let mut line = String::new();
            match client.reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {
                    let trimmed = line.trim();
                    if trimmed.starts_with("201 ") || !trimmed.starts_with("201-") {
                        break;
                    }
                }
                Err(_) => break,
            }
        }

        Ok(client)
    }

    pub fn send_command(&mut self, cmd: &str) -> Result<Vec<String>, String> {
        let full_cmd = format!("{}\r\n", cmd);
        self.reader.get_mut().write_all(full_cmd.as_bytes())
            .map_err(|e| format!("Erro ao enviar comando '{}': {}", cmd, e))?;
        self.reader.get_mut().flush()
            .map_err(|e| format!("Erro no flush: {}", e))?;

        let mut lines = Vec::new();
        loop {
            let mut line = String::new();
            match self.reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {
                    let trimmed = line.trim().to_string();
                    if trimmed == "." {
                        break;
                    }
                    lines.push(trimmed.clone());

                    if trimmed.len() >= 4 {
                        let code = &trimmed[..3];
                        let sep = trimmed.as_bytes()[3];
                        if sep == b' ' && (code == "200" || code == "202" || code.starts_with('4')) {
                            break;
                        }
                    }
                }
                Err(e) => {
                    if lines.is_empty() {
                        return Err(format!("Timeout XBDM: {}", e));
                    }
                    break;
                }
            }
        }

        Ok(lines)
    }

    pub fn read_memory(&mut self, addr: u64, len: usize) -> Result<Vec<u8>, String> {
        let cmd = format!("getmem addr=0x{:X} length={}", addr, len);
        let lines = self.send_command(&cmd)?;
        for line in lines {
            if line.starts_with("202-") || line.starts_with("200") {
                continue;
            }
            let hex_str = line.trim();
            if hex_str.len() >= len * 2 {
                let mut bytes = Vec::with_capacity(len);
                for i in 0..len {
                    if let Ok(b) = u8::from_str_radix(&hex_str[i*2..i*2+2], 16) {
                        bytes.push(b);
                    }
                }
                return Ok(bytes);
            }
        }
        Err("Resposta de memoria invalida".to_string())
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let target_ip = "192.168.3.6";
    let mut xbdm = XbdmClient::connect(target_ip)?;

    println!("========================================================");
    println!("  Xenon-Emu: Mapeando Subsistema de Video da Dashboard  ");
    println!("========================================================");

    // 1. Inspect .edata of dash.xex at 0x92A7D000
    println!("\n[*] 1. Lendo .edata de dash.xex (0x92A7D000):");
    match xbdm.read_memory(0x92A7D000, 128) {
        Ok(bytes) => {
            println!("    -> Export directory header de dash.xex:");
            print_hex(&bytes, 0x92A7D000);
        }
        Err(e) => println!("    [!] Erro .edata: {}", e),
    }

    // 2. Query all threads to locate dash.xex threads (0x92000000..0x93000000)
    println!("\n[*] 2. Varrendo todas as threads para encontrar as de 'dash.xex' (0x92000000 - 0x93000000):");
    let mut dash_threads = Vec::new();
    if let Ok(lines) = xbdm.send_command("threads") {
        for l in lines {
            if let Ok(tid_dec) = l.parse::<i32>() {
                let tid_hex = format!("0x{:08X}", tid_dec as u32);
                if let Ok(info_lines) = xbdm.send_command(&format!("threadinfo thread={}", tid_hex)) {
                    for info in info_lines {
                        if info.contains("start=0x92") || info.contains("start=0x93") {
                            println!("    -> ENCONTRADA THREAD DA DASHBOARD: TID={} info={}", tid_hex, info);
                            dash_threads.push((tid_hex.clone(), info));
                        } else if info.contains("start=") {
                            // Check if proc matches
                            if let Some(pos) = info.find("start=0x") {
                                let addr_str = &info[pos + 8..pos + 16];
                                if addr_str.starts_with("92") {
                                    println!("    -> DASH THREAD: TID={} | {}", tid_hex, info);
                                    dash_threads.push((tid_hex.clone(), info));
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    println!("[+] Total de threads pertencentes a dash.xex: {}", dash_threads.len());
    for (t_hex, info) in &dash_threads {
        println!("    [DASH THREAD {}] {}", t_hex, info);
        if let Ok(ctx) = xbdm.send_command(&format!("getcontext thread={} control", t_hex)) {
            println!("       Context: {:?}", ctx);
        }
    }

    // 3. Query full integer registers (GPR0..GPR31) of the top dash threads
    for (t_hex, _info) in dash_threads.iter().take(4) {
        println!("\n[*] Registradores Inteiros (GPRs) da Thread {}:", t_hex);
        if let Ok(lines) = xbdm.send_command(&format!("getcontext thread={} int", t_hex)) {
            for l in &lines {
                println!("    {}", l);
            }
        }
    }

    Ok(())
}

fn print_hex(bytes: &[u8], base_addr: u64) {
    for (i, chunk) in bytes.chunks(16).enumerate() {
        let addr = base_addr + (i * 16) as u64;
        print!("    {:08X} | ", addr);
        for b in chunk {
            print!("{:02X} ", b);
        }
        if chunk.len() < 16 {
            for _ in 0..(16 - chunk.len()) {
                print!("   ");
            }
        }
        print!("| ");
        for b in chunk {
            let c = *b as char;
            if c.is_ascii_graphic() || c == ' ' {
                print!("{}", c);
            } else {
                print!(".");
            }
        }
        println!();
    }
}
