use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let ip = "192.168.3.6";
    println!("========================================================");
    println!("  Xenon-Emu: Testando protocolo XBDM 'getfile'         ");
    println!("========================================================");

    let mut stream = TcpStream::connect(format!("{}:730", ip))?;
    stream.set_read_timeout(Some(Duration::from_millis(5000))).ok();
    stream.set_write_timeout(Some(Duration::from_millis(5000))).ok();

    // Read greeting
    let mut greeting = Vec::new();
    let mut b = [0u8; 1];
    while stream.read(&mut b)? == 1 {
        if b[0] == b'\n' { break; }
        greeting.push(b[0]);
    }
    println!("[*] Greeting: {}", String::from_utf8_lossy(&greeting));

    // Test getfile with a small file: flash:\SegoeXbox-Light.xtt (24,576 bytes)
    let test_file = "flash:\\SegoeXbox-Light.xtt";
    let cmd = format!("getfile name=\"{}\"\r\n", test_file);
    stream.write_all(cmd.as_bytes())?;
    stream.flush()?;

    // Read response line
    let mut resp_line = Vec::new();
    while stream.read(&mut b)? == 1 {
        if b[0] == b'\n' { break; }
        if b[0] != b'\r' { resp_line.push(b[0]); }
    }
    let resp_str = String::from_utf8_lossy(&resp_line).to_string();
    println!("[*] Response to getfile: '{}'", resp_str);

    // Let's inspect the next 32 bytes from stream
    let mut header_buf = [0u8; 32];
    let n = stream.read(&mut header_buf)?;
    println!("[*] Next {} bytes: {:02X?}", n, &header_buf[..n]);

    Ok(())
}
