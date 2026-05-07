# Ícones

Esta pasta deve conter os ícones do app, gerados pelo Tauri CLI.

## Como gerar

A partir de uma imagem fonte (PNG quadrada, recomendado 1024x1024):

```bash
npx tauri icon path/to/source.png
```

O comando gera automaticamente:

- `32x32.png`
- `128x128.png`
- `128x128@2x.png`
- `icon.ico` (Windows)
- `icon.icns` (macOS)
- `icon.png` (genérico — usado pelo tray)
- Ícones Android/iOS (ignorar para desktop)

## Placeholder temporário

Enquanto o ícone definitivo não chega, você pode:

1. Copiar qualquer `.ico` válido para `icon.ico` e `.png` para os demais.
2. Ou rodar `npx tauri icon` apontando para o logo Ecxon (`Ecxon/public/logo.svg` convertido pra PNG).

> **Importante:** o build (`npm run tauri build`) **falha** se os ícones referenciados em `tauri.conf.json` não existirem.
