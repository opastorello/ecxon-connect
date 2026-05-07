// HTTP server local — endpoints /health e /probe.
// Mantém semântica idêntica ao agente Go (ecxon-diag-agent).

use std::net::SocketAddr;
use std::time::Duration;

use axum::{
    extract::State,
    http::{HeaderName, HeaderValue, Method, StatusCode},
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
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

const ALLOWED_ORIGINS: &[&str] = &[
    "https://www.ecxon.com.br",
    "https://ecxon.com.br",
    "https://ecxon.pastorello-lab.com.br",
    "http://localhost:5173",
];

#[derive(Clone)]
struct AppCtx {
    handle: AppHandle,
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

#[derive(Serialize)]
struct HealthResponse {
    ok: bool,
    version: &'static str,
    os: &'static str,
    arch: &'static str,
}

/// Tipo do callback chamado **assim que o bind 5556 é estabelecido com sucesso**,
/// antes de `axum::serve` começar (que nunca retorna em condições normais).
/// Permite o caller atualizar UI/state pra "running" sem esperar o serve eterno.
pub type OnReadyCb = Box<dyn FnOnce() + Send + 'static>;

pub async fn start(
    handle: AppHandle,
    on_ready: OnReadyCb,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // CORS: validação manual para suportar Access-Control-Allow-Private-Network.
    // tower-http CorsLayer não tem suporte nativo a esse header (PNA é não-padrão),
    // então adicionamos via middleware customizado abaixo.
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

    let ctx = AppCtx { handle };

    let app = Router::new()
        .route("/health", get(handle_health).options(handle_options))
        .route("/probe", post(handle_probe).options(handle_options))
        .with_state(ctx)
        .layer(axum::middleware::from_fn(private_network_header))
        .layer(cors)
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

/// Middleware que adiciona `Access-Control-Allow-Private-Network: true`
/// em todas as respostas (incluindo preflight). PNA exige isso quando a página
/// HTTPS chama 127.0.0.1.
async fn private_network_header(
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let mut res = next.run(req).await;
    res.headers_mut().insert(
        HeaderName::from_static("access-control-allow-private-network"),
        HeaderValue::from_static("true"),
    );
    res
}

async fn handle_options() -> impl IntoResponse {
    StatusCode::NO_CONTENT
}

async fn handle_health() -> Json<HealthResponse> {
    Json(HealthResponse {
        ok: true,
        version: VERSION,
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
    })
}

async fn handle_probe(
    State(ctx): State<AppCtx>,
    Json(req): Json<ProbeRequest>,
) -> Response {
    if req.host.is_empty() || req.port == 0 {
        return (StatusCode::BAD_REQUEST, "invalid host/port").into_response();
    }

    let proto_lower = req.proto.to_lowercase();
    let res = match proto_lower.as_str() {
        "tcp" => probe::probe_tcp(&req.host, req.port, req.timeout_ms).await,
        "udp" => probe::probe_udp(&req.host, req.port, req.timeout_ms).await,
        _ => return (StatusCode::BAD_REQUEST, "proto must be tcp or udp").into_response(),
    };

    // Emite evento para a UI atualizar a lista de probes recentes.
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
