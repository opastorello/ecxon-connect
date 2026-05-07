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
/// Latência só é reportada quando há sinal real (resposta do peer ou ICMP
/// refused). "ok por timeout" retorna `latency_ms = None` porque não houve
/// medição — antes vinha o read timeout (~250ms) como se fosse latência,
/// enganoso.
///
/// Heurística:
///   - resposta recebida: porta aberta (ok, latência real send→recv)
///   - timeout no read (default 250ms): sem ICMP refused = porta provavelmente
///     aberta (ok conservador, sem latência)
///   - ConnectionRefused: ICMP port unreachable = porta fechada (fail, latência
///     real até o SO entregar o erro)
pub async fn probe_udp(host: &str, port: u16, timeout_ms: Option<u64>) -> ProbeResponse {
    let addr = format!("{host}:{port}");
    let read_to = clamp_timeout(timeout_ms, DEFAULT_UDP_TIMEOUT_MS);
    // Send é quase sempre não-bloqueante; damos pelo menos 500ms de folga.
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

    // Mede só o RTT do send→recv. Bind/connect têm overhead irrelevante (~µs)
    // e conceitualmente não fazem parte da latência da porta remota.
    let start = Instant::now();

    match timeout(write_to, socket.send(&[0u8])).await {
        Ok(Ok(_)) => {}
        Ok(Err(e)) => {
            return ProbeResponse {
                ok: false,
                latency_ms: None,
                error: Some(e.to_string()),
            };
        }
        Err(_) => {
            return ProbeResponse {
                ok: false,
                latency_ms: None,
                error: Some("write timeout".into()),
            };
        }
    }

    let mut buf = [0u8; 1];
    match timeout(read_to, socket.recv(&mut buf)).await {
        // Resposta real do peer = latência líquida medida.
        Ok(Ok(_)) => ProbeResponse {
            ok: true,
            latency_ms: Some(start.elapsed().as_millis() as u64),
            error: None,
        },
        Ok(Err(e)) => {
            let kind = e.kind();
            let s = e.to_string().to_lowercase();
            if kind == ErrorKind::ConnectionRefused
                || s.contains("connection refused")
                || s.contains("port unreachable")
                || s.contains("network unreachable")
                || s.contains("host unreachable")
            {
                // ICMP refused = latência real até o SO entregar o erro.
                ProbeResponse {
                    ok: false,
                    latency_ms: Some(start.elapsed().as_millis() as u64),
                    error: Some(e.to_string()),
                }
            } else if kind == ErrorKind::TimedOut
                || s.contains("timed out")
                || s.contains("timeout")
            {
                // Timeout do SO sem ICMP refused = ok conservador, sem latência.
                ProbeResponse {
                    ok: true,
                    latency_ms: None,
                    error: None,
                }
            } else {
                ProbeResponse {
                    ok: false,
                    latency_ms: None,
                    error: Some(e.to_string()),
                }
            }
        }
        Err(_) => {
            // Read timeout limpo do tokio (= timeout do SO sem refused):
            // ok conservador, sem latência real.
            ProbeResponse {
                ok: true,
                latency_ms: None,
                error: None,
            }
        }
    }
}
