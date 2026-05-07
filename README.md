# Ecxon Connect

Agente desktop **Windows** que substitui o `ecxon-diag-agent` (Go) com janela nativa e system tray.

Permite ao site **Ecxon Diagnóstico** (`https://www.ecxon.com.br/diagnostic`) realizar testes TCP/UDP raw a partir da máquina do usuário, contornando limitações do navegador.

- HTTP server local em `http://127.0.0.1:5556`
- Endpoints `GET /health` e `POST /probe`
- System tray sempre presente (fechar a janela apenas esconde)
- Auto-start no Windows (toggle na UI)
- Single-instance (segunda execução foca a janela existente)
- Validação de porta antes do bind (mensagem clara se 5556 estiver em uso)

---

## Pré-requisitos (Windows, para build local)

1. **Rust** (toolchain MSVC) — https://www.rust-lang.org/tools/install
   - Instale também os "Build Tools for Visual Studio" (componente "Desktop development with C++").
2. **Node.js 18+** — https://nodejs.org/
3. **WebView2** — já vem instalado no Windows 10/11 modernos. Se faltar, baixe em https://developer.microsoft.com/microsoft-edge/webview2/.

---

## Build local

```bash
cd ecxon-connect
npm install
npm run tauri build
```

Os instaladores ficam em:

- `src-tauri/target/release/bundle/msi/Ecxon Connect_1.0.0_x64_pt-BR.msi`
- `src-tauri/target/release/bundle/nsis/Ecxon Connect_1.0.0_x64-setup.exe`

Para desenvolvimento com hot-reload:

```bash
npm run tauri dev
```

---

## Release & distribuição

Crie uma tag começando com `v`:

```bash
git tag v1.0.0
git push origin v1.0.0
```

O GitHub Actions:

1. Compila para `x86_64-pc-windows-msvc`.
2. Gera `.msi` (WiX) e `-setup.exe` (NSIS).
3. Cria/atualiza o GitHub Release com os instaladores.

---

## Reduzindo falsos-positivos de antivírus / SmartScreen (100% grátis)

O instalador **não é assinado** com certificado de código. Sem assinatura, o **Microsoft SmartScreen** mostra "O Windows protegeu o seu PC — aplicativo desconhecido" nos primeiros downloads, e alguns antivírus podem detectar como falso-positivo.

Isto **não é um bug** — é o comportamento esperado para binário sem reputação. O usuário clica em **"Mais informações" → "Executar assim mesmo"**.

### Como acelerar reputação (grátis)

1. **Submeter ao [Microsoft Security Intelligence](https://www.microsoft.com/wdsi/filesubmission)** (mais eficaz)
   - Selecione "Software developer" → upload do `.msi` e do `.exe`.
   - Marque "Incorrectly detected as malware/malicious".
   - Resposta em 1-3 dias. Após whitelist, SmartScreen para de mostrar aviso para esse hash.

2. **Submeter ao [VirusTotal](https://www.virustotal.com/)** — alguns AV consultam reputação dele.
   - Para falsos-positivos de AVs específicos, cada vendor tem portal próprio (Avast, AVG, Kaspersky, etc.).

3. **Repetir a cada release nova** — cada hash precisa de reputação separada.

---

## Estrutura

```
ecxon-connect/
├── package.json                  Scripts npm + deps Tauri JS
├── src/                          Frontend (HTML/CSS/JS vanilla)
│   ├── index.html
│   ├── style.css
│   └── main.js
├── src-tauri/                    Backend (Rust)
│   ├── Cargo.toml
│   ├── build.rs
│   ├── tauri.conf.json
│   ├── capabilities/default.json
│   ├── icons/                    Gerados via `npx tauri icon`
│   └── src/
│       ├── main.rs               Entry point
│       ├── lib.rs                Builder Tauri + plugins + setup
│       ├── http_server.rs        Axum em 127.0.0.1:5556
│       ├── probe.rs              TCP/UDP raw (semântica idêntica ao Go)
│       └── tray.rs               System tray menu
├── .github/workflows/release.yml CI: tag v* → build + release
└── README.md
```

---

## Endpoints HTTP locais

### `GET /health`

```json
{ "ok": true, "version": "1.0.0", "os": "windows", "arch": "x86_64" }
```

### `POST /probe`

Body:

```json
{ "host": "45.40.99.71", "port": 443, "proto": "tcp" }
```

Response:

```json
{ "ok": true, "latency_ms": 23 }
```

CORS allowlist: `https://www.ecxon.com.br`, `https://ecxon.com.br`, `https://ecxon.pastorello-lab.com.br`, `http://localhost:5173`.

Header `Access-Control-Allow-Private-Network: true` em todas as respostas (necessário pra navegador permitir HTTPS → 127.0.0.1).
