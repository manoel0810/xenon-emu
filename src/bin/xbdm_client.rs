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
        
        stream.set_read_timeout(Some(Duration::from_millis(2500))).ok();
        stream.set_write_timeout(Some(Duration::from_millis(2500))).ok();

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
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let target_ip = "192.168.3.6";
    let mut xbdm = XbdmClient::connect(target_ip)?;

    let cmds = [
        "getcurtitle",
        "dmesg",
        "altaddr",
        "systeminfo",
        "drivefreespace name=\"Hdd:\"",
        "dirlist name=\"Hdd:\\\"",
    ];

    for cmd in &cmds {
        println!("\n[*] Comando: '{}'", cmd);
        match xbdm.send_command(cmd) {
            Ok(lines) => {
                for l in lines.iter().take(8) {
                    println!("    -> {}", l);
                }
                if lines.len() > 8 {
                    println!("    ... (mais {} linhas)", lines.len() - 8);
                }
            }
            Err(e) => println!("    [!] Erro: {}", e),
        }
    }

    Ok(())
}
