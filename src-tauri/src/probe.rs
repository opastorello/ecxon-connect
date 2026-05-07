// Probes TCP/UDP raw — semântica idêntica ao agente Go original.
// Refs: ecxon-diag-agent/main.go linhas 78-122.

use std::io::ErrorKind;
use std::time::{Duration, Instant};

use tokio::net::{TcpStream, UdpSocket};
use tokio::time::timeout;

use crate::http_server::ProbeResponse;

// Defaults usados quando o cliente não informa timeout_ms na request.
pub const DEFAULT_TCP_TIMEOUT_MS: u64 = 4000;
pub const DEFAULT_UDP_TIMEOUT_MS: u64 = 250;
// Limites pra evitar abuso (cliente passando 60s, segurando socket).
pub const MAX_TIMEOUT_MS: u64 = 5000;
pub const MIN_TIMEOUT_MS: u64 = 50;

fn clamp_timeout(ms: Option<u64>, default_ms: u64) -> Duration {
    let ms = ms.unwrap_or(default_ms).clamp(MIN_TIMEOUT_MS, MAX_TIMEOUT_MS);
    Duration::from_millis(ms)
}

/// TCP probe — SYN handshake completo. timeout_ms padrão 4000ms.
pub async fn probe_tcp(host: &str, port: u16, timeout_ms: Option<u64>) -> ProbeResponse {
    let addr = format!("{host}:{port}");
    let start = Instant::now();
    let to = clamp_timeout(timeout_ms, DEFAULT_TCP_TIMEOUT_MS);
    match timeout(to, TcpStream::connect(&addr)).await {
        Ok(Ok(_stream)) => ProbeResponse {
            ok: true,
            latency_ms: Some(start.elapsed().as_millis() as u64),
            error: None,
        },
        Ok(Err(e)) => ProbeResponse {
            ok: false,
            latency_ms: None,
            error: Some(e.to_string()),
        },
        Err(_) => ProbeResponse {
            ok: false,
            latency_ms: None,
            error: Some("i/o timeout".into()),
        },
    }
}

/// UDP probe — envia 1 byte e aguarda resposta ou ICMP unreachable.
/// Mesma heurística do agente Go:
///   - resposta recebida: porta aberta (ok)
///   - timeout no read (1s): provavelmente aberto, sem ICMP refused (ok conservador)
///   - ConnectionRefused: ICMP port unreachable = porta fechada (fail)
pub async fn probe_udp(host: &str, port: u16, timeout_ms: Option<u64>) -> ProbeResponse {
    let addr = format!("{host}:{port}");
    let start = Instant::now();
    let read_to = clamp_timeout(timeout_ms, DEFAULT_UDP_TIMEOUT_MS);
    // Write timeout = max(read, 500ms) — algum delay aceitável pra send.
    let write_to = std::cmp::max(read_to, Duration::from_millis(500));

    // Bind local efêmero. v4 + fallback v6.
    let socket = match UdpSocket::bind("0.0.0.0:0").await {
        Ok(s) => s,
        Err(_) => match UdpSocket::bind("[::]:0").await {
            Ok(s) => s,
            Err(e) => {
                return ProbeResponse {
                    ok: false,
                    latency_ms: None,
                    error: Some(format!("bind local: {e}")),
                }
            }
        },
    };

    if let Err(e) = socket.connect(&addr).await {
        return ProbeResponse {
            ok: false,
            latency_ms: None,
            error: Some(e.to_string()),
        };
    }

    // Envia 1 byte (com timeout de 2s).
    match timeout(write_to, socket.send(&[0u8])).await {
        Ok(Ok(_)) => {}
        Ok(Err(e)) => {
            return ProbeResponse {
                ok: false,
                latency_ms: Some(start.elapsed().as_millis() as u64),
                error: Some(e.to_string()),
            };
        }
        Err(_) => {
            return ProbeResponse {
                ok: false,
                latency_ms: Some(start.elapsed().as_millis() as u64),
                error: Some("write timeout".into()),
            };
        }
    }

    // Recebe (timeout 1s).
    let mut buf = [0u8; 1];
    match timeout(read_to, socket.recv(&mut buf)).await {
        Ok(Ok(_)) => ProbeResponse {
            ok: true,
            latency_ms: Some(start.elapsed().as_millis() as u64),
            error: None,
        },
        Ok(Err(e)) => {
            // ConnectionRefused = ICMP port unreachable = porta fechada.
            let kind = e.kind();
            let s = e.to_string().to_lowercase();
            let latency = start.elapsed().as_millis() as u64;
            if kind == ErrorKind::ConnectionRefused
                || s.contains("connection refused")
                || s.contains("port unreachable")
                || s.contains("network unreachable")
                || s.contains("host unreachable")
            {
                ProbeResponse {
                    ok: false,
                    latency_ms: Some(latency),
                    error: Some(e.to_string()),
                }
            } else if kind == ErrorKind::TimedOut
                || s.contains("timed out")
                || s.contains("timeout")
            {
                // Sem ICMP refused dentro da janela = porta provavelmente aberta.
                ProbeResponse {
                    ok: true,
                    latency_ms: Some(latency),
                    error: None,
                }
            } else {
                ProbeResponse {
                    ok: false,
                    latency_ms: Some(latency),
                    error: Some(e.to_string()),
                }
            }
        }
        Err(_) => {
            // Tokio elapsed = read timeout limpo, sem erro do SO. Equivalente a
            // i/o timeout do Go, e indica ausência de ICMP refused = ok.
            ProbeResponse {
                ok: true,
                latency_ms: Some(start.elapsed().as_millis() as u64),
                error: None,
            }
        }
    }
}
