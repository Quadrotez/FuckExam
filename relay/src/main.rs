fn detect_ips() -> Vec<String> {
    let mut ips = Vec::new();
    // primary IP via UDP connect trick
    if let Ok(sock) = std::net::UdpSocket::bind("0.0.0.0:0") {
        if sock.connect("8.8.8.8:80").is_ok() {
            if let Ok(addr) = sock.local_addr() {
                if let std::net::SocketAddr::V4(v4) = addr {
                    ips.push(v4.ip().to_string());
                }
            }
        }
    }
    // try getifaddrs for all interfaces
    let mut list: *mut libc::ifaddrs = std::ptr::null_mut();
    if unsafe { libc::getifaddrs(&mut list) } == 0 {
        let mut cur = list;
        while !cur.is_null() {
            let ifa = unsafe { &*cur };
            if !ifa.ifa_addr.is_null() {
                let family = unsafe { (*ifa.ifa_addr).sa_family } as i32;
                if family == libc::AF_INET {
                    let sin = ifa.ifa_addr as *const libc::sockaddr_in;
                    let addr = unsafe { &*sin }.sin_addr;
                    if addr.s_addr != 0 {
                        let b = addr.s_addr.to_ne_bytes();
                        let ip = format!("{}.{}.{}.{}", b[0], b[1], b[2], b[3]);
                        if !ip.starts_with("127.") && !ip.starts_with("0.") && !ip.starts_with("169.254.") && !ips.contains(&ip) {
                            ips.push(ip);
                        }
                    }
                }
            }
            cur = ifa.ifa_next;
        }
        unsafe { libc::freeifaddrs(list) };
    }
    ips
}

#[tokio::main]
async fn main() {
    let port = std::env::var("RELAY_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(fuck_exam_relay::DEFAULT_PORT);
    let addr = format!("0.0.0.0:{port}");

    let host_override = std::env::var("RELAY_HOST").ok();
    let ips = detect_ips();

    let explicit = host_override.is_some();
    let display_hosts: Vec<String> = match host_override {
        Some(h) => vec![h],
        None if ips.is_empty() => vec!["<ip>".into()],
        None => ips,
    };

    println!("FuckExam relay listening on {addr}");
    for h in &display_hosts {
        println!("  recorder: ws://{h}:{port}   room: <любая>");
        println!("  viewer:   ws://{h}:{port}   room: <та же>");
    }
    if explicit {
        println!("  (RELAY_HOST задан вручную; авто-определение пропущено)");
    } else if display_hosts.iter().any(|h| h == "<ip>") {
        eprintln!("  WARNING: не удалось определить IP, задайте RELAY_HOST вручную");
    }

    if let Err(e) = fuck_exam_relay::serve(&addr).await {
        eprintln!("relay error: {e}");
        std::process::exit(1);
    }
}