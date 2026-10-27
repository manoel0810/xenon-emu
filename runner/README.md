# Xenon-Emu: Scripts de Compilação e Teste Real

Esta pasta dedicada contém os scripts para compilar e executar o emulador em **modo de teste real** com todos os subsistemas de host conectados ao sistema operacional.

---

## Conteúdo da Pasta

| Arquivo | Descrição |
|---|---|
| `build.bat` | Script em lote (cmd/batch) para compilar o projeto em modo Release (ou Debug com `build.bat debug`). |
| `build.ps1` | Script PowerShell para compilar com saída colorida e verificação de artefatos. |
| `run_real.bat` | Script em lote para iniciar a execução do teste real da dashboard com janela 720p HD @ 60 FPS. |
| `run_real.ps1` | Script PowerShell para teste real com limpeza automática de processos, suporte a parâmetros (`-SkipBootAnim`) e injeção de ambiente. |

---

## Ambiente Configurado no Teste Real

Ao executar `run_real.bat` ou `run_real.ps1`, as seguintes configurações de hardware/host são ativadas:

1. **Host Real (`XENON_HOST=real`):**
   - Utiliza o backend `HostReal` ([src/host/real.rs](file:///d:/XBOX%20360%20-%20Firmeware%20Emulator/xenon-emu/src/host/real.rs)).
   - Sistema de arquivos real ancorado no host.
   - Relógio de alta precisão sincronizado com o sistema operacional do PC.
2. **Rede Real Opt-in (`XENON_NET_ONLINE=1`):**
   - Ativação de sockets TCP e UDP reais via `std::net`.
   - Resolução de nomes DNS do host.
   - Detecção automática de IP local ativo via routing do SO.
3. **Interface Gráfica HD e Áudio:**
   - Janela de apresentação interativa 1280x720 HD @ 60 FPS (sem headless).
   - Buffer e processamento de áudio PCM / XAudio.
4. **Entrada Interativa e Gamepad:**
   - Mesclagem transparente de teclado host e gamepad físico XInput (Xbox 360 / Xbox One / Series).

---

## Mapeamento de Teclas (Janela do Emulador)

| Ação Xbox 360 | Teclado do Host |
|---|---|
| D-Pad / Analógico Esquerdo | **WASD** ou **Setas direcionais** |
| Botão (A) | **Espaço** ou **Enter** |
| Botão (B) / Cancelar | **Esc** ou **Backspace** |
| Botões (X) / (Y) | **X** / **Y** |
| Bumpers (LB / RB) | **Q** / **E** |
| Gatilhos (LT / RT) | **Z** / **V** |
| Botão Start | **F1** ou **Home** |
| Botão Back | **F2**, **Tab** ou **Delete** |
| Analógicos L3 / R3 | **Shift Esquerdo** / **Shift Direito** |
| Alternar Buffers (Guest vs Dump) | **B** |
| Alternar Cenas XUI | **1**, **2**, **3**, **4** |
| Fechar Janela com Segurança | **Shift + ESC** ou clique no **[X]** |
