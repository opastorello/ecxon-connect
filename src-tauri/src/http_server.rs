// HTTP server local — endpoints /health e /probe.
// Mantém semântica idêntica ao agente Go (ecxon-diag-agent), com hardening:
// Host header validation (anti DNS-rebind), target allowlist (anti DDoS-reflector
// e LAN-scanner), token-bucket rate limit, /health não expõe fingerprint.

use std::net::SocketAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::{
    extract::State,
    http::{HeaderName, HeaderValue, Method, Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use http::header;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};
use tokio::net::TcpListener;
use tower::limit::ConcurrencyLimitLayer;
use tower_http::cors::{AllowOrigin, CorsLayer};

use crate::probe;

pub const BIND_ADDR: &str = "127.0.0.1:5556";

/// Origins do browser (CORS). Apenas o site Ecxon pode chamar o agente.
const ALLOWED_ORIGINS: &[&str] = &[
    "https://www.ecxon.com.br",
    "https://ecxon.com.br",
    "https://ecxon.pastorello-lab.com.br",
    "http://localhost:5173",
];

/// Host header válidos. Qualquer outro -> 403 (kills DNS-rebind).
/// Mesmo se o navegador for enganado a fazer request via attacker.com (TTL=0),
/// o agente rejeita porque attacker.com não está aqui.
const ALLOWED_HOSTS: &[&str] = &[
    "127.0.0.1:5556",
    "localhost:5556",
    "[::1]:5556",
];

/// Hostnames permitidos como TARGET do /probe. Apenas infra Ecxon.
/// Qualquer host fora dessa lista -> 403 (anti DDoS-reflector + LAN-scan).
const ALLOWED_PROBE_HOSTS: &[&str] = &[
    "rs.pastorello-lab.com.br",
    "sp.pastorello-lab.com.br",
    "rs1.ecxon.com.br",
    "sp1.ecxon.com.br",
];

/// Token-bucket simples global (1 agente = 1 user). 100 probes/min,
/// reabastece a 5/s — basta pra testar 36 portas em alguns segundos
/// e bloqueia ataque sustentado.
const RATE_LIMIT_CAPACITY: u32 = 100;
const RATE_LIMIT_REFILL_PER_SEC: u32 = 5;

#[derive(Clone)]
struct AppCtx {
    handle: AppHandle,
    rl: std::sync::Arc<Mutex<TokenBucket>>,
}

struct TokenBucket {
    tokens: f64,
    last: Instant,
}

impl TokenBucket {
    fn new() -> Self {
        Self { tokens: RATE_LIMIT_CAPACITY as f64, last: Instant::now() }
    }
    /// Tenta consumir 1 token. Retorna true se houver crédito.
    fn try_consume(&mut self) -> bool {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last).as_secs_f64();
        self.last = now;
        self.tokens = (self.tokens + elapsed * RATE_LIMIT_REFILL_PER_SEC as f64)
            .min(RATE_LIMIT_CAPACITY as f64);
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

#[derive(Deserialize)]
struct ProbeRequest {
    host: String,
    port: u16,
    proto: String,
    /// Timeout customizado em ms — o agente clamp pra [50, 5000].
    /// Default: 4000 (TCP), 250 (UDP).
    #[serde(default)]
    timeout_ms: Option<u64>,
}

#[derive(Serialize, Clone, Debug)]
pub struct ProbeResponse {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Serialize, Clone)]
struct ProbeEvent {
    ok: bool,
    host: String,
    port: u16,
    proto: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    latency_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

/// Resposta de /health — apenas `ok`. Não vaza `version`/`os`/`arch`
/// (esses dados, somados ao PNA bypass, eram um fingerprint cross-site
/// estável mesmo em modo anônimo).
#[derive(Serialize)]
struct HealthResponse {
    ok: bool,
}

/// Tipo do callback chamado **assim que o bind 5556 é estabelecido com sucesso**,
/// antes de `axum::serve` começar (que nunca retorna em condições normais).
/// Permite o caller atualizar UI/state pra "running" sem esperar o serve eterno.
pub type OnReadyCb = Box<dyn FnOnce() + Send + 'static>;

pub async fn start(
    handle: AppHandle,
    on_ready: OnReadyCb,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::list(
            ALLOWED_ORIGINS
                .iter()
                .filter_map(|o| o.parse::<HeaderValue>().ok()),
        ))
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
        .allow_headers([
            header::CONTENT_TYPE,
            HeaderName::from_static("access-control-request-private-network"),
        ])
        .max_age(Duration::from_secs(86400));

    let ctx = AppCtx {
        handle,
        rl: std::sync::Arc::new(Mutex::new(TokenBucket::new())),
    };

    let app = Router::new()
        .route("/health", get(handle_health).options(handle_options))
        .route("/probe", post(handle_probe).options(handle_options))
        .with_state(ctx)
        // Ordem importa: host_guard roda primeiro (rejeita DNS-rebind), depois CORS,
        // depois PNA (que só responde em /probe).
        .layer(axum::middleware::from_fn(private_network_header))
        .layer(cors)
        .layer(axum::middleware::from_fn(host_guard))
        .layer(ConcurrencyLimitLayer::new(10));

    let addr: SocketAddr = BIND_ADDR.parse()?;
    // bind explícito separado de serve() pra capturar EADDRINUSE como erro claro.
    let listener = TcpListener::bind(addr).await?;
    log::info!("ecxon-connect agente escutando em http://{addr}");
    // Bind OK — sinaliza pro caller atualizar status pra "running" agora.
    // axum::serve a partir daqui é loop infinito.
    on_ready();
    axum::serve(listener, app).await?;
    Ok(())
}

/// Middleware: rejeita Host header fora da allowlist. Sem isso, um atacante
/// com DNS rebind (CNAME TTL=0 -> 127.0.0.1) faz `http://attacker.com:5556/probe`
/// e o navegador trata como same-origin a attacker.com — bypassa CORS inteiro.
async fn host_guard(req: Request<axum::body::Body>, next: Next) -> Response {
    let host_ok = req
        .headers()
        .get(http::header::HOST)
        .and_then(|h| h.to_str().ok())
        .map(|h| ALLOWED_HOSTS.contains(&h))
        .unwrap_or(false);
    if !host_ok {
        return (StatusCode::FORBIDDEN, "host not allowed").into_response();
    }
    next.run(req).await
}

/// Middleware: adiciona `Access-Control-Allow-Private-Network: true` apenas
/// no `/probe` (preflight + POST). PNA não é necessário no /health, e sem ele
/// o /health deixa de ser fetchable cross-site para fingerprinting.
async fn private_network_header(req: Request<axum::body::Body>, next: Next) -> Response {
    let is_probe = req.uri().path() == "/probe";
    let mut res = next.run(req).await;
    if is_probe {
        res.headers_mut().insert(
            HeaderName::from_static("access-control-allow-private-network"),
            HeaderValue::from_static("true"),
        );
    }
    res
}

async fn handle_options() -> impl IntoResponse {
    StatusCode::NO_CONTENT
}

async fn handle_health() -> Json<HealthResponse> {
    Json(HealthResponse { ok: true })
}

async fn handle_probe(State(ctx): State<AppCtx>, Json(req): Json<ProbeRequest>) -> Response {
    // Rate limit global (1 agente = 1 usuário humano).
    {
        let mut rl = ctx.rl.lock().expect("rate-limit mutex poisoned");
        if !rl.try_consume() {
            return (StatusCode::TOO_MANY_REQUESTS, "rate limit").into_response();
        }
    }

    if req.host.is_empty() || req.port == 0 {
        return (StatusCode::BAD_REQUEST, "invalid host/port").into_response();
    }

    // Target allowlist — apenas hosts da infra Ecxon. Sem isso, qualquer aba do
    // browser autenticada via DNS-rebind/XSS dirige o agente como DDoS reflector
    // (UDP flood, SYN scan, LAN map, cloud-metadata oracle).
    if !ALLOWED_PROBE_HOSTS.contains(&req.host.as_str()) {
        return (StatusCode::FORBIDDEN, "host not in allowlist").into_response();
    }

    let proto_lower = req.proto.to_lowercase();
    let raw = match proto_lower.as_str() {
        "tcp" => probe::probe_tcp(&req.host, req.port, req.timeout_ms).await,
        "udp" => probe::probe_udp(&req.host, req.port, req.timeout_ms).await,
        _ => return (StatusCode::BAD_REQUEST, "proto must be tcp or udp").into_response(),
    };

    // Sanitiza erro: collapse pra 2 estados ("timeout" / "unreachable") em vez
    // de propagar a string nativa do SO. Isso fecha o oracle de LAN-scan que
    // distinguia "actively refused" vs "i/o timeout" vs "network unreachable".
    let res = ProbeResponse {
        ok: raw.ok,
        latency_ms: raw.latency_ms,
        error: raw.error.as_deref().map(sanitize_error).map(str::to_string),
    };

    // Emite evento para a UI (com host completo — UI roda no Tauri local, sem
    // risco de cross-site fingerprint).
    let evt = ProbeEvent {
        ok: res.ok,
        host: req.host.clone(),
        port: req.port,
        proto: proto_lower.clone(),
        latency_ms: res.latency_ms,
        error: res.error.clone(),
    };
    let _ = ctx.handle.emit("probe", evt);

    Json(res).into_response()
}

/// Reduz mensagens de erro do SO a um conjunto fechado. Antes, mensagens
/// como "Nenhuma conexão pôde ser feita... os error 10061" funcionavam como
/// oracle pra distinguir host fechado vs host inexistente vs LAN inalcançável.
fn sanitize_error(raw: &str) -> &'static str {
    let l = raw.to_lowercase();
    if l.contains("timeout") || l.contains("timed out") {
        "timeout"
    } else if l.contains("refused") || l.contains("recusou") || l.contains("connection reset") {
        "refused"
    } else {
        "unreachable"
    }
}
