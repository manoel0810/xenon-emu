use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

pub struct Xbdm {
    stream: TcpStream,
}

impl Xbdm {
    pub fn connect(ip: &str) -> Result<Self, String> {
        let stream = TcpStream::connect(format!("{}:730", ip))
            .map_err(|e| format!("Connect error: {}", e))?;
        stream.set_read_timeout(Some(Duration::from_millis(5000))).ok();
        stream.set_write_timeout(Some(Duration::from_millis(5000))).ok();

        let mut s = Self { stream };
        // Read greeting
        let _ = s.read_line();
        Ok(s)
    }

    pub fn read_line(&mut self) -> Result<String, String> {
        let mut bytes = Vec::new();
        let mut buf = [0u8; 1];
        loop {
            match self.stream.read(&mut buf) {
                Ok(1) => {
                    if buf[0] == b'\n' {
                        break;
                    }
                    if buf[0] != b'\r' {
                        bytes.push(buf[0]);
                    }
                }
                Ok(_) => break,
                Err(e) => return Err(format!("Read line timeout/error: {}", e)),
            }
        }
        Ok(String::from_utf8_lossy(&bytes).to_string())
    }

    pub fn send(&mut self, cmd: &str) -> Result<Vec<String>, String> {
        let full = format!("{}\r\n", cmd);
        self.stream.write_all(full.as_bytes())
            .map_err(|e| format!("Write error: {}", e))?;
        self.stream.flush().map_err(|e| format!("Flush error: {}", e))?;

        let mut lines = Vec::new();
        loop {
            match self.read_line() {
                Ok(line) => {
                    let trimmed = line.trim().to_string();
                    if trimmed == "." || trimmed.is_empty() {
                        if !lines.is_empty() {
                            break;
                        }
                    }
                    lines.push(trimmed.clone());
                    if trimmed.starts_with("200 ")
                        || trimmed.starts_with("201 ")
                        || trimmed.starts_with("400 ")
                        || trimmed.starts_with("401 ")
                        || trimmed.starts_with("402 ")
                        || trimmed.starts_with("403 ")
                        || trimmed.starts_with("404 ")
                        || trimmed.starts_with("410 ")
                    {
                        break;
                    }
                }
                Err(e) => {
                    if lines.is_empty() {
                        return Err(e);
                    }
                    break;
                }
            }
        }
        Ok(lines)
    }

    pub fn download_file(&mut self, remote_path: &str, local_path: &str) -> Result<usize, String> {
        let cmd = format!("getfile name=\"{}\"\r\n", remote_path);
        self.stream.write_all(cmd.as_bytes())
            .map_err(|e| format!("Write getfile error: {}", e))?;
        self.stream.flush().map_err(|e| format!("Flush getfile error: {}", e))?;

        // First line response
        let resp = self.read_line()?;
        println!("  [GETFILE RESP] {}", resp);

        // Typical XBDM getfile response: "203- binary response follows" (size in bytes follows or in header)
        // Or "200- length=0x..."
        let mut total_bytes = 0usize;
        if resp.starts_with("203") || resp.starts_with("200") {
            let mut file = std::fs::File::create(local_path)
                .map_err(|e| format!("Create local file failed: {}", e))?;
            let mut buf = [0u8; 8192];
            loop {
                match self.stream.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        total_bytes += n;
                        file.write_all(&buf[..n])
                            .map_err(|e| format!("File write error: {}", e))?;
                    }
                    Err(_) => break, // timeout when done
                }
            }
            Ok(total_bytes)
        } else {
            Err(format!("XBDM rejected getfile: {}", resp))
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let ip = "192.168.3.6";
    println!("========================================================");
    println!("  Xenon-Emu: Sondando e Extraindo Dashboard Moderna     ");
    println!("  Console Alvo: {}:730 (Modo Somente-Leitura)          ", ip);
    println!("========================================================");

    let mut xbdm = Xbdm::connect(ip)?;

    // 1. Check current title
    println!("\n[*] 1. Inspecionando Titulo Atual (title):");
    if let Ok(lines) = xbdm.send("title") {
        for l in lines {
            println!("  {}", l);
        }
    }

    // 2. Drive list
    println!("\n[*] 2. Unidades montadas no sistema (drivelist):");
    if let Ok(lines) = xbdm.send("drivelist") {
        for l in lines {
            println!("  {}", l);
        }
    }

    // 3. Inspect 'game:\' directory (where default.xex is running)
    println!("\n[*] 3. Arquivos em 'game:\\':");
    if let Ok(lines) = xbdm.send("dirlist name=\"game:\\\"") {
        for l in lines {
            println!("  {}", l);
        }
    }

    // 4. Inspect 'flash:\' directory
    println!("\n[*] 4. Arquivos na particao NAND 'flash:\\':");
    if let Ok(lines) = xbdm.send("dirlist name=\"flash:\\\"") {
        for l in lines {
            println!("  {}", l);
        }
    }

    // 5. Inspect 'hdd:\Content\0000000000000000\FFFE07D1\' (Dashboard container)
    println!("\n[*] 5. Inspecionando 'hdd:\\Content\\0000000000000000\\FFFE07D1\\':");
    if let Ok(lines) = xbdm.send("dirlist name=\"hdd:\\Content\\0000000000000000\\FFFE07D1\\\"") {
        for l in lines {
            println!("  {}", l);
        }
    }

    // 6. Inspect 'hdd:\$SystemUpdate\'
    println!("\n[*] 6. Inspecionando 'hdd:\\$SystemUpdate\\':");
    if let Ok(lines) = xbdm.send("dirlist name=\"hdd:\\$SystemUpdate\\\"") {
        for l in lines {
            println!("  {}", l);
        }
    }

    // 7. Inspect console system version (xbeinfo / sysinfo)
    println!("\n[*] 7. Informacoes da versao do Kernel e SO do Console (sysinfo):");
    if let Ok(lines) = xbdm.send("sysinfo") {
        for l in lines {
            println!("  {}", l);
        }
    }

    Ok(())
}
