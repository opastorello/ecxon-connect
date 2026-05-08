// Ecxon Connect — front-end vanilla.
// Usa a API global do Tauri (withGlobalTauri = true em tauri.conf.json) —
// nenhum bundler/import; tudo via window.__TAURI__.

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const { getCurrentWindow } = window.__TAURI__.window;
const { getVersion } = window.__TAURI__.app;

const SITE_URL = "https://www.ecxon.com.br";

const $ = (id) => document.getElementById(id);

// --- Status -----------------------------------------------------------------

async function refreshStatus() {
  try {
    const status = await invoke("get_server_status");
    if (status?.running) {
      setStatus("ok", "Agente ativo", "Pronto — abra o diagnóstico no site Ecxon");
    } else {
      const reason = status?.error || "iniciando…";
      setStatus("err", "Agente inativo", reason);
    }
  } catch (err) {
    setStatus("err", "Falha ao consultar status", String(err));
  }
}

function setStatus(level, title, sub) {
  const card = $("status-card");
  const dot = $("status-dot");
  card.classList.remove("ok", "err");
  dot.classList.remove("ok", "err");
  if (level === "ok" || level === "err") {
    card.classList.add(level);
    dot.classList.add(level);
  }
  $("status-title").textContent = title;
  $("status-sub").textContent = sub;
}

// --- Autostart --------------------------------------------------------------

async function refreshAutostart() {
  try {
    const enabled = await invoke("plugin:autostart|is_enabled");
    $("autostart-toggle").checked = !!enabled;
  } catch (err) {
    console.error("autostart status falhou:", err);
  }
}

async function onAutostartChange(e) {
  const target = e.target;
  try {
    if (target.checked) await invoke("plugin:autostart|enable");
    else await invoke("plugin:autostart|disable");
    $("autostart-hint").textContent = target.checked
      ? "Ativo — vai iniciar no próximo login do Windows."
      : "Carrega o agente automaticamente no login.";
  } catch (err) {
    console.error("autostart toggle falhou:", err);
    target.checked = !target.checked;
    $("autostart-hint").textContent = "Erro ao alterar: " + err;
  }
}

// --- Update check -----------------------------------------------------------

const REPO = "opastorello/ecxon-connect";

async function checkForUpdate(currentVersion) {
  try {
    const res = await fetch(`https://api.github.com/repos/${REPO}/releases/latest`, {
      headers: { Accept: "application/vnd.github+json" },
    });
    if (!res.ok) return;
    const data = await res.json();
    const latest = String(data.tag_name || "").replace(/^v/, "");
    if (!latest || latest === currentVersion) return;
    const link = $("update-link");
    link.textContent = `↻ Nova versão v${latest} disponível`;
    link.hidden = false;
    // Aumenta a janela para caber o link de update
    try {
      const win = getCurrentWindow();
      const size = await win.innerSize();
      await win.setSize(new window.__TAURI__.dpi.LogicalSize(size.width / window.devicePixelRatio, (size.height / window.devicePixelRatio) + 30));
    } catch {}
    link.addEventListener("click", async () => {
      try {
        await invoke("plugin:opener|open_url", { url: `${SITE_URL}/diagnostic` });
      } catch (err) {
        console.error("abrir /diagnostic falhou:", err);
        link.textContent = `↻ Falha ao abrir — copie: ${SITE_URL}/diagnostic`;
      }
    });
  } catch { /* offline ou rate limit — silencioso */ }
}

// --- Window controls --------------------------------------------------------

function setupWindowControls() {
  const win = getCurrentWindow();
  $("btn-min").addEventListener("click", () => win.minimize());
  $("btn-hide").addEventListener("click", () => win.hide());
}

// --- Init -------------------------------------------------------------------

async function init() {
  // Versão no rodapé
  let currentVersion = null;
  try {
    currentVersion = await getVersion();
    $("footer-version").textContent = "v" + currentVersion;
  } catch { /* ignore */ }

  // Detecção de nova versão (não bloqueia init)
  if (currentVersion) checkForUpdate(currentVersion);

  setupWindowControls();
  await refreshStatus();
  await refreshAutostart();

  $("autostart-toggle").addEventListener("change", onAutostartChange);

  // Eventos do Rust pra status
  await listen("server-status", (evt) => {
    const s = evt?.payload;
    if (!s) return;
    if (s.running) setStatus("ok", "Agente ativo", "Pronto — abra o diagnóstico no site Ecxon");
    else setStatus("err", "Agente inativo", s.error || "servidor parado");
  });

  // Polling leve a cada 5s — fallback caso event não chegue
  setInterval(refreshStatus, 5000);
}

init().catch((err) => {
  console.error("init falhou:", err);
  setStatus("err", "Erro ao iniciar UI", String(err));
});
