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

        // Read initial connection greeting lines
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
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let target_ip = "192.168.3.6";
    println!("========================================================");
    println!("  Xenon-Emu: Inspecionando DASHBOARD ORIGINAL no Xbox   ");
    println!("  IP Alvo: {} (Porta 730 XBDM)                          ", target_ip);
    println!("========================================================");

    let mut xbdm = XbdmClient::connect(target_ip)?;

    // 1. List loaded modules now that the user is in the official Dashboard
    println!("\n[*] 1. Modulos carregados na memoria (modules):");
    let mut dash_base: Option<u64> = None;
    let mut dash_name = String::new();

    if let Ok(lines) = xbdm.send_command("modules") {
        for l in &lines {
            println!("    -> {}", l);
            if l.contains("dash.xex") || l.contains("dash") {
                dash_name = l.clone();
                // extract base=0x...
                if let Some(pos) = l.find("base=0x") {
                    let rest = &l[pos + 7..];
                    if let Some(end) = rest.find(' ') {
                        if let Ok(addr) = u64::from_str_radix(&rest[..end], 16) {
                            dash_base = Some(addr);
                        }
                    }
                }
            }
        }
    }

    // 2. Sections of dash.xex
    println!("\n[*] 2. Inspecionando Secoes do modulo da Dashboard (modsections):");
    let mod_name = if !dash_name.is_empty() { "dash.xex" } else { "xboxkrnl.exe" };
    if let Ok(lines) = xbdm.send_command(&format!("modsections name=\"{}\"", mod_name)) {
        for l in lines {
            println!("    -> {}", l);
        }
    }

    // 3. Active Threads in the Dashboard
    println!("\n[*] 3. Threads ativas na Dashboard Original (threads):");
    let mut thread_ids = Vec::new();
    if let Ok(lines) = xbdm.send_command("threads") {
        for l in lines {
            println!("    -> {}", l);
            if let Ok(tid) = l.parse::<i64>() {
                thread_ids.push(tid);
            }
        }
    }

    // 4. Thread Info for first few threads
    println!("\n[*] 4. Detalhes das Threads de Renderizacao e UI:");
    for tid in thread_ids.iter().take(5) {
        if let Ok(lines) = xbdm.send_command(&format!("threadinfo thread={}", tid)) {
            for l in lines {
                println!("    [TID {}] -> {}", tid, l);
            }
        }
    }

    // 5. If dash_base found, read first 64 bytes of PE header
    if let Some(base) = dash_base {
        println!("\n[*] 5. Lendo 64 bytes da Dashboard Original em 0x{:08X}:", base);
        if let Ok(lines) = xbdm.send_command(&format!("getmem addr=0x{:X} length=64", base)) {
            for l in lines {
                println!("    -> {}", l);
            }
        }
    }

    println!("\n========================================================");
    println!("  [+] Captura de Hardware Real Concluida!");
    println!("========================================================");

    Ok(())
}
