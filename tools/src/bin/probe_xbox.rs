use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

fn test_port(ip: &str, port: u16, timeout: Duration) -> Option<String> {
    let addr: SocketAddr = format!("{}:{}", ip, port).parse().ok()?;
    match TcpStream::connect_timeout(&addr, timeout) {
        Ok(mut stream) => {
            let _ = stream.set_read_timeout(Some(Duration::from_millis(1500)));
            let _ = stream.set_write_timeout(Some(Duration::from_millis(1500)));

            let mut banner = String::new();
            let mut buf = [0u8; 1024];

            // Some services (FTP, XBDM) send a greeting immediately upon connection
            if let Ok(n) = stream.read(&mut buf) {
                if n > 0 {
                    banner = String::from_utf8_lossy(&buf[..n]).trim().to_string();
                }
            }

            // If port 730 (XBDM), send a test command if no banner
            if port == 730 {
                if banner.is_empty() {
                    let _ = stream.write_all(b"dbgversion\r\n");
                    if let Ok(n) = stream.read(&mut buf) {
                        if n > 0 {
                            banner = String::from_utf8_lossy(&buf[..n]).trim().to_string();
                        }
                    }
                }
            }

            Some(banner)
        }
        Err(_) => None,
    }
}

fn main() {
    let target_ip = "192.168.3.6";
    println!("========================================================");
    println!("     Xenon-Emu: Sondando Xbox 360 Hardware Real         ");
    println!("     IP Alvo: {}", target_ip);
    println!("========================================================");

    let ports_to_check = [
        (730, "XBDM Debug Manager (Porta Primaria)"),
        (731, "XBDM Secondary"),
        (21, "FTP Server (Aurora / DashLaunch)"),
        (28710, "JRPC / JRPC2 Remote Procedure Call"),
        (9999, "Aurora Nova Web Interface"),
        (80, "HTTP Web Server"),
    ];

    for (port, desc) in &ports_to_check {
        print!("[*] Testando porta {:5} ({}) ... ", port, desc);
        match test_port(target_ip, *port, Duration::from_millis(1500)) {
            Some(banner) => {
                println!("CONECTADO COM SUCESSO!");
                if !banner.is_empty() {
                    println!("    [BANNER]: {}", banner);
                }
            }
            None => {
                println!("Sem resposta / Fechada");
            }
        }
    }
}
