fn ipv4_valid(ip: &str) -> bool {
    ip.parse::<std::net::Ipv4Addr>().is_ok()
}

/// Декодирует HTTP-тело из chunked-ответа.
fn decode_chunked(body: &str) -> Option<String> {
    let mut rest = body;
    let mut out = String::new();
    loop {
        let line_end = rest.find("\r\n")?;
        let size = usize::from_str_radix(rest[..line_end].trim(), 16).ok()?;
        rest = &rest[line_end + 2..];
        if size == 0 {
            break;
        }
        if rest.len() < size {
            return None;
        }
        out.push_str(&rest[..size]);
        rest = &rest[size..];
        if let Some(stripped) = rest.strip_prefix("\r\n") {
            rest = stripped;
        }
    }
    let first = out.trim().split_whitespace().next().unwrap_or("").to_string();
    Some(first)
}

/// Извлекает из HTTP-ответа тело (Content-Length или chunked), первый токен — IP.
fn parse_http_body(raw: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(raw);
    let idx = text.find("\r\n\r\n").or_else(|| text.find("\n\n"))?;
    let (head, body) = text.split_at(idx);
    let body = &body[if body.starts_with("\r\n\r\n") { 4 } else { 2 }..];
    if head.to_lowercase().contains("transfer-encoding: chunked") {
        return decode_chunked(body);
    }
    let first = body.trim().split_whitespace().next().unwrap_or("").to_string();
    Some(first)
}

const PUBLIC_IP_PROVIDERS: [&str; 3] = ["ipv4.myip.coffee", "api.ipify.org", "icanhazip.com"];

/// Внешний (публичный) IP через внешний сервис по обычному HTTP.
/// Возвращаем первым в списке — это то, что видно зрителям из интернета.
async fn detect_public_ip() -> Option<String> {
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    for provider in PUBLIC_IP_PROVIDERS {
        let result = tokio::time::timeout(Duration::from_secs(4), async {
            let mut stream =
                tokio::net::TcpStream::connect(format!("{provider}:80")).await.ok()?;
            let req = format!(
                "GET / HTTP/1.1\r\nHost: {provider}\r\nUser-Agent: fuck-exam-relay\r\nConnection: close\r\n\r\n"
            );
            stream.write_all(req.as_bytes()).await.ok()?;
            let mut buf = Vec::with_capacity(64);
            stream.read_to_end(&mut buf).await.ok()?;
            Some(buf)
        })
        .await
        .ok()
        .flatten();

        if let Some(raw) = result {
            if let Some(ip) = parse_http_body(&raw).filter(|x| ipv4_valid(x)) {
                return Some(ip);
            }
        }
    }
    None
}

fn is_docker_bridge(ip: &str) -> bool {
    let parts: Vec<&str> = ip.split('.').collect();
    if parts.len() == 4 {
        if let (Ok(a), Ok(b)) = (parts[0].parse::<u8>(), parts[1].parse::<u8>()) {
            return a == 172 && (16..=31).contains(&b);
        }
    }
    false
}

fn is_filtered(ip: &str) -> bool {
    ip.starts_with("127.")
        || ip.starts_with("0.")
        || ip.starts_with("169.254.")
        || is_docker_bridge(ip)
}

/// Локальные адреса интерфейсов (без loopback/link-local/Docker bridge).
fn local_ips() -> Vec<String> {
    let mut ips = Vec::new();
    let mut push = |ip: &str| {
        if !is_filtered(ip) && !ips.iter().any(|x| x == ip) {
            ips.push(ip.to_string());
        }
    };

    // primary IP via UDP connect trick
    if let Ok(sock) = std::net::UdpSocket::bind("0.0.0.0:0") {
        if sock.connect("8.8.8.8:80").is_ok() {
            if let Ok(addr) = sock.local_addr() {
                if let std::net::SocketAddr::V4(v4) = addr {
                    let ip = v4.ip().to_string();
                    if ipv4_valid(&ip) {
                        push(&ip);
                    }
                }
            }
        }
    }

    // все интерфейсы через getifaddrs
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
                        push(&ip);
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
    let listen_port = std::env::var("RELAY_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(fuck_exam_relay::DEFAULT_PORT);
    let pub_port = std::env::var("RELAY_PUB_PORT")
        .ok()
        .and_then(|v| v.parse::<u16>().ok())
        .unwrap_or(listen_port);
    let addr = format!("0.0.0.0:{listen_port}");

    let host_override = std::env::var("RELAY_HOST").ok();

    println!("FuckExam relay listening on {addr}");

    match host_override {
        // За NAT/frp: явно задан публичный адрес, к которому подключаются зрители.
        Some(host) => {
            println!("  зрителям (за NAT/frp, ws://{host}:{pub_port}):");
            println!("    recorder: ws://{host}:{pub_port}   room: <любая>");
            println!("    viewer:   ws://{host}:{pub_port}   room: <та же>");
            let locals = local_ips();
            if !locals.is_empty() {
                println!("  локально (в той же сети):");
                for h in locals {
                    println!("    recorder: ws://{h}:{listen_port}   room: <любая>");
                    println!("    viewer:   ws://{h}:{listen_port}   room: <та же>");
                }
            }
            println!("  (RELAY_HOST задан вручную; авто-определение внешнего IP пропущено)");
            if pub_port != listen_port {
                println!("  (RELAY_PUB_PORT={pub_port} отличен от RELAY_PORT={listen_port} — убедитесь, что frp/NAT пробрасывает именно {pub_port} на {listen_port})");
            }
        }
        None => {
            let mut ips = Vec::new();
            if let Some(pub_ip) = detect_public_ip().await {
                ips.push(pub_ip);
            }
            ips.extend(local_ips());

            if ips.is_empty() {
                eprintln!("  WARNING: не удалось определить IP, задайте RELAY_HOST вручную");
                eprintln!("  (например, если сервер за NAT/frp): RELAY_HOST=1.2.3.4");
            } else {
                for h in &ips {
                    println!("  recorder: ws://{h}:{pub_port}   room: <любая>");
                    println!("  viewer:   ws://{h}:{pub_port}   room: <та же>");
                }
            }
        }
    }

    if let Err(e) = fuck_exam_relay::serve(&addr).await {
        eprintln!("relay error: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_http_body_content_length() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Length: 11\r\nConnection: close\r\n\r\n94.29.26.29\r\n";
        assert_eq!(parse_http_body(raw).as_deref(), Some("94.29.26.29"));
    }

    #[test]
    fn parse_http_body_chunked() {
        // ipv4.myip.coffee отдаёт chunked-тело
        let raw = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\nb\r\n94.29.26.29\r\n0\r\n\r\n";
        assert_eq!(parse_http_body(raw).as_deref(), Some("94.29.26.29"));
    }

    #[test]
    fn parse_http_body_chunked_multi() {
        // несколько чанков
        let raw = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\na\r\n94.29.26.2\r\n2\r\n9\r\n0\r\n\r\n";
        assert_eq!(parse_http_body(raw).as_deref(), Some("94.29.26.29"));
    }

    #[test]
    fn parse_http_body_no_body() {
        let raw = b"HTTP/1.1 204 No Content\r\n\r\n";
        assert_eq!(parse_http_body(raw).as_deref(), Some(""));
    }

    #[test]
    fn filter_docker_bridge() {
        assert!(is_docker_bridge("172.17.0.1"));
        assert!(is_docker_bridge("172.19.0.1"));
        assert!(!is_docker_bridge("172.15.0.1"));
        assert!(!is_docker_bridge("192.168.1.11"));
        assert!(!is_docker_bridge("10.0.0.5"));
        assert!(is_filtered("172.18.0.1"));
        assert!(!is_filtered("192.168.1.11"));
    }

    #[test]
    fn ipv4_validation() {
        assert!(ipv4_valid("94.29.26.29"));
        assert!(!ipv4_valid("2a00:1370:81a6::1"));
        assert!(!ipv4_valid("not-an-ip"));
    }
}