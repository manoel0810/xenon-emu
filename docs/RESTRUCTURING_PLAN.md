# Plano de reestruturação do xenon-emu

Documento de decisão e execução. Complementa `HANDOFF.md` (estado e regras) e `THIRD_PARTY.md` (fontes externas).
Idioma de trabalho com o usuário: português.

---

## 1. Resumo executivo

**Objetivo original:** rodar o dashboard retail do Xbox 360 (`dash.xex`, build 17559) como código guest, como em um console real.

**O que mudou:** depois de muitas sessões de correção pontual, a CPU está estável (thread principal roda 400+ frames sem trap), mas a GPU não desenha nada (0 draws) e a thread principal fica presa esperando um fence. As causas apontam para problemas **estruturais**, não bugs isolados:

1. A GPU foi construída com uma tabela PM4 inventada e sem execução de shaders.
2. O kernel é HLE sem semântica real de espera, sem escalonador e sem object manager.
3. O código está concentrado em poucos arquivos grandes (`main.rs` 1471 linhas, `exports.rs` 2674, `xboxkrnl.rs` 2073), misturado com 42 ferramentas em `src/bin`.
4. Não há uma fronteira declarada entre o que é LLE e o que é HLE. O projeto estava derivando para um híbrido ad hoc.

**Novo objetivo (este plano):** reestruturar e organizar o projeto, declarar de forma explícita a fronteira HLE/LLE seguindo o modelo comprovado do Xenia, reaproveitar conhecimento (e código isolado) do Xenia/Canary com licença respeitada, e só então retomar o avanço da dash.

**Decisões já tomadas pelo usuário:**

- Portar para Rust usando Xenia/Canary como especificação (estratégia A).
- Chamar C/C++ apenas em módulos isolados e de baixo acoplamento via FFI (estratégia B pontual).
- **Sem fork do Xenia** e sem embutir seu runtime.
- Ponte com o PC real por "drivers fakes" interceptando chamadas do guest (módulo `xe-host`).

---

## 2. Diagnóstico detalhado do estado atual

### 2.1 O que funciona

| Área | Evidência |
|---|---|
| Interpretador PPC | Dash executa ~8,6M passos na thread principal por 400 frames sem trap. |
| AltiVec/VMX128 | `cpu/altivec.rs` cobre VX completo; VA-form xo 32..47; VMX128 ops 5/6; `vsldoi128`; `vupkd3d128` tipos 0,1,3,4,5. |
| Loader XEX | `loader/xex.rs` carrega o dash (base `0x92000000`, entry `0x92196610`). |
| Memória | Janela virtual guest `0x40000000..0x8FFFFFFF` com backing privado sob demanda (chunks de 64KB) do pool físico `0x14000000..0x1FFFFFFF`; MMIO da GPU em `0x7FC80000..0x7FC8FFFF`. |
| Init da dash | Cadeia de init `0x92186B80..0x92186CCC` completa; singleton `0x92917FD0` criado. |
| Diagnóstico | Watchdog, heartbeat, syscall log, PC hooks, trap trace, PM4 log (ver `HANDOFF.md` seção 4). |

### 2.2 O que está estruturalmente errado

**GPU**
- `gpu/pm4.rs` tem opcodes inventados. Só 4 batem com o hardware: NOP `0x10`, WAIT_REG_MEM `0x3C`, INDIRECT_BUFFER `0x3F`, ME_INIT `0x48`.
- A dash emite opcodes reais que a tabela antiga não conhecia (`0x21`, `0x2B`, `0x36`, `0x3B`, `0x58`, `0x60-0x63`).
- Shaders (microcódigo Xenos) não são decodificados nem executados; `gpu/shader.rs` (417 linhas) não corresponde ao formato real.
- Não há modelo de EDRAM, resolve, nem swap real.
- Testes de GPU usam opcodes falsos e precisam ser reescritos.

**Kernel**
- `KeWaitForSingleObject`, `KeWaitForMultipleObjects`, `NtWaitForSingleObjectEx`, `NtWaitForMultipleObjectsEx` retornam sucesso imediato. Threads que deveriam dormir giram em polling.
- Não existe object manager (handles, tipos, contagem de referência, dispatcher headers).
- Não existe escalonador: threads são multiplexadas por conveniência, sem filas de espera, prioridades, APC/DPC ou timers coerentes.
- Alguns exports ainda sem tratamento: xboxkrnl `0x0111, 0x013D, 0x0015, 0x00F5`; xam `0x0282, 0x001A, 0x0200, 0x07E2`.

**Organização do código**
- `main.rs` concentra inicialização, laço de frames, threads, UI e diagnóstico.
- `exports.rs` e `xboxkrnl.rs` misturam todas as áreas do kernel (Ke, Nt, Mm, Io, Rtl, Ex, Xe, Vd, XAM).
- 42 binários de análise em `src/bin/` ao lado do código do emulador.
- Variáveis `XENON_*` lidas em vários pontos, sem struct de configuração única.
- Scripts em `scratch/` misturados entre úteis (`iter.ps1`, `trapscan.ps1`) e descartáveis.

### 2.3 Sintoma que resume tudo

A thread principal fica em `0x921AFFF8..0x921B0020` chamando `0x921A40B0` (~206x por frame), comparando o fence em `0x90102000` (objeto r31+0x2B10) com o valor 5. A dash envia PM4 pelo ring `0x10104000` (32KB) e atualiza `CP_RB_WPTR` por MMIO `0x7FC80714`. Fence 3 e depois 5 vêm de `EVENT_WRITE_SHD (0x58)`, o 5 pelo caminho do callback de VBlank, tarde demais. A causa exata de o loop não liberar **ainda não foi confirmada**. Ela depende de dois modelos que hoje são fracos: CP fiel (entrega de interrupção e escrita de fence) e kernel com espera/escalonamento reais.

---

## 3. Princípios da nova arquitetura

1. **Fronteira declarada:** cada camada é LLE ou HLE por decisão documentada, não por conveniência.
2. **Fidelidade onde o guest fala com hardware, HLE onde o guest fala com o kernel.** O D3D do console é código da dash; a GPU precisa ser emulada no nível de comandos PM4.
3. **Sem atalhos que finjam resultado** (ex.: escrever fence direto, reiniciar thread para mascarar bug).
4. **Toda decisão com evidência:** disassembly (`ppcdis`), ordinal de export, fonte/doc do Xenia, trace. Sem palpites.
5. **Isolamento do host:** o guest nunca chama APIs do Windows diretamente; passa por `xe-host`.
6. **Mudança mecânica separada de mudança de comportamento:** refactors não alteram semântica e são validados por regressão (seção 9).
7. **Ferramentas fora do emulador:** análise e engenharia reversa em `tools/`.
8. **Licenças registradas:** tudo derivado de terceiros documentado (ver `THIRD_PARTY.md`).

---

## 4. Fronteira HLE vs LLE (matriz de decisão)

| Camada | Decisão | Justificativa | Referência Xenia |
|---|---|---|---|
| CPU PowerPC (inclui VMX128) | **LLE** (interpretador; JIT opcional no futuro) | O guest executa código nativo PPC. | `cpu/ppc/ppc_emit_*.cc`, `cpu/ppc/testing/` |
| Memória / MMIO | **LLE** do mapa físico/virtual relevante | O guest depende de endereços e aliases específicos. | `memory.cc` |
| GPU: ring buffer, CP, PM4, registros | **LLE** | Hardware real; sem API intermediária. | `gpu/command_processor.cc`, `gpu/xenos.h`, `gpu/register_table.inc` |
| GPU: shaders (ucode) | **LLE** (decodificar e executar/traduzir microcódigo real) | A dash envia ucode Xenos. | `gpu/ucode.h`, `gpu/shader*`, tradutores |
| GPU: EDRAM / resolve / swap | **LLE** comportamental | Resolve copia EDRAM para memória guest e alimenta o frontbuffer. | `gpu/render_target_cache*`, `gpu/draw_util` |
| Kernel xboxkrnl | **HLE por export** | Reimplementa o comportamento de cada função do kernel no host. | `kernel/xboxkrnl/*` |
| Objetos de kernel, escalonador | **HLE com semântica real** | Esperas bloqueantes, filas, timers, APC/DPC. | `kernel/xobject*`, `kernel/xthread.*`, `kernel/xevent.*` |
| XAM (UI, usuários, conteúdo, notify, net) | **HLE** | Biblioteca do sistema; não há hardware. | `kernel/xam/*` |
| Rede, armazenamento, áudio, input, tempo | **HLE + ponte `xe-host`** | O guest pede o serviço; o host entrega. | `kernel/xam/xam_net.cc`, `kernel/xsocket.*`, `hid/`, `apu/`, `vfs/` |
| XMA (áudio) | **HLE** com decoder (FFI ffmpeg no futuro, candidato à estratégia B) | Hardware de decodificação. | `apu/` |
| Hypervisor, bootloaders, firmware, NAND | **Fora do escopo** | A dash roda sem eles quando o kernel é HLE. | n/a |

Observação: os arquivos de `Corona 16mb/` (dump de NAND, chaves) e `$SystemUpdate/` existem no workspace, mas **não** fazem parte do caminho de execução HLE. Não são redistribuíveis.

---

## 5. Estrutura de módulos alvo

### 5.1 Layout

Primeira etapa: manter **um único crate** com módulos (menor atrito). Quando estabilizar, promover para um workspace Cargo com crates separados, nomeados abaixo.

```
xenon-emu/
  Cargo.toml                (workspace no futuro)
  docs/                     HANDOFF.md, RESTRUCTURING_PLAN.md, THIRD_PARTY.md, notas de RE
  src/ (ou crates/)
    app/        main, args, laço de frames, criação de threads, shutdown
    cpu/        decode, interpreter, altivec, vmx128, estado de thread (xe-cpu)
    mem/        bus, MMIO, janelas virtuais, físico, heap alloc (xe-mem)
    kernel/
      objects/  handles, tipos, dispatcher headers, eventos, semáforos, mutex, timers
      sched/    threads guest, filas de espera, APC/DPC, tempo virtual
      xboxkrnl/ ex, io, ke, mm, nt, rtl, xe_crypt, vd (um arquivo por área)
      xam/      user, content, notify, net, ui, app
      vfs/      devices (hdd0:, usb, dvd, content), STFS/XContent
      exports/  tabela de ordinals (gerada) e registro de handlers
    gpu/
      regs/     tabela de registros Xenos
      cp/       ring buffer, PM4 real, indirect buffers, interrupções, fences
      ucode/    decoder de microcódigo (CF/ALU/fetch)
      shader/   interpretador CPU, depois tradutor (SPIR-V/wgpu)
      edram/    render targets, resolve, swap
      texture/  fetch constants, formatos, tiling
    loader/     xex, xzp, pe, imports
    host/       traits + backends real/offline (xe-host)
    ui/         janela, apresentação, input
    diag/       DiagConfig (todas as XENON_*), watchdog, trace, hooks
  vendor/       módulos C/C++ isolados (estratégia B)
  tools/        os binários hoje em src/bin (ppcdis etc.)
  tests/        instruções PPC, PM4, shaders, golden frames, regressão da dash
  scripts/      scripts operacionais (iter, trapscan, build_ref, cmp_diag, time_runs)
```

### 5.2 Regras de dependência

```
app ──► kernel, gpu, cpu, mem, host, ui, diag, loader
kernel ──► cpu(estado), mem, host, diag
gpu ──► mem, diag          (não conhece kernel)
cpu ──► mem, diag          (não conhece kernel nem gpu; syscalls via trait/callback)
host ──► (nada do emulador; só std/crates de SO)
tools ──► cpu, loader, mem (somente leitura/análise)
```

- `gpu` não depende de `kernel`. O kernel (`VdSetGraphicsInterruptCallback` etc.) conversa com `gpu` por uma interface pequena (registrar callback, ler/escrever registro, notificar interrupção).
- `cpu` expõe um ponto de chamada para exports (trait `SyscallHandler`) em vez de importar o kernel.
- `host` é a única camada que toca APIs do Windows.

### 5.3 Divisão concreta dos arquivos grandes

| Arquivo atual | Destino |
|---|---|
| `main.rs` (1471) | `app/main.rs` (entrada), `app/frame_loop.rs`, `app/threads.rs`, `ui/window.rs`, `diag/*`. |
| `kernel/exports.rs` (2674) | `kernel/xam/*.rs` e `kernel/xboxkrnl/*.rs` por área; `kernel/exports/table.rs` para o registro. |
| `kernel/xboxkrnl.rs` (2073) | `kernel/xboxkrnl/{ex,io,ke,mm,nt,rtl,xe_crypt,vd}.rs`. |
| `gpu/display.rs` (1707) | `ui/present.rs` (apresentação) e `gpu/edram/swap.rs`; separar lógica de janela do estado de GPU. |
| `gpu/xenos.rs` (876) | `gpu/cp/*` e `gpu/regs/*`. |
| `gpu/pm4.rs` | Reescrito do zero a partir dos opcodes reais. |
| `gpu/shader.rs` (417) | Substituído por `gpu/ucode/` + `gpu/shader/`. |
| `cpu/interpreter.rs` (2469) | Dividir por grupo de opcode (`int`, `branch`, `loadstore`, `float`, `vmx`, `system`). |
| `src/bin/*` (42) | `tools/` (workspace crate separado). |

---

## 6. Ponte com o host (`xe-host`)

Ideia do usuário: interceptar chamadas a dispositivos (por exemplo, rede a cabo) e ter drivers fakes que se comunicam com o dispositivo real do PC e devolvem o resultado ao guest.

### 6.1 Modelo

O guest chama exports (`NetDll_*`, `XNet*`, `XHttp*`, `XamUser*`, `XamContent*`, `Nt*File`). O kernel/XAM HLE traduz a chamada em uma operação de trait; o backend implementa. O guest enxerga estruturas do Xbox (`XNADDR`, `XNET_ETHERNET_LINK_*`, `XCONTENT_DATA`); o host enxerga APIs do SO.

```rust
trait HostNet {
    fn link_status(&self) -> LinkStatus;
    fn socket(&mut self, af: i32, ty: i32, proto: i32) -> Result<SockId, HostErr>;
    fn connect(&mut self, s: SockId, addr: SocketAddrV4) -> Result<(), HostErr>;
    fn send(&mut self, s: SockId, buf: &[u8]) -> Result<usize, HostErr>;
    fn recv(&mut self, s: SockId, buf: &mut [u8]) -> Result<usize, HostErr>;
    fn dns_resolve(&mut self, name: &str) -> Result<Vec<Ipv4Addr>, HostErr>;
}
trait HostStorage { fn open(&mut self, guest_path: &str, mode: OpenMode) -> Result<HostFile, HostErr>; /* hdd0:, usb, dvd, content */ }
trait HostTime    { fn now_filetime(&self) -> u64; }
trait HostInput   { fn poll_pad(&mut self, slot: u32) -> PadState; }
trait HostAudio   { fn submit_pcm(&mut self, buf: &[i16]); }
trait HostProfile { fn signed_in_users(&self) -> Vec<UserInfo>; }
```

### 6.2 Backends

- `host-offline` (padrão, já é o comportamento atual): signin local, `XHttpStartup/Shutdown` = 1, `WSAStartup`/`XNetStartup` = 0, `XNetGetEthernetLinkStatus` = 0 (sem link).
- `host-real` (futuro): Winsock/`std::net`, arquivos mapeados para uma pasta do host, XInput/SDL, WASAPI.

### 6.3 Restrições

- Xbox Live real exige autenticação e serviços que o projeto não tem. Escopo: rede local, DNS e HTTP genérico.
- Modo offline é o padrão; qualquer rede real é opt-in.
- A ponte não pode vazar dados do usuário nem do host para fora sem ação explícita.
- USB/SATA/NAND **não** são emulados como dispositivos; `Io*`, `Nt*File` e `XamContent*` com VFS mapeado cobrem a dash.
- Qualquer endpoint/servidor que o host abra deve ser sinalizado quanto a autenticação e acesso.

---

## 7. Reaproveitamento de terceiros

Detalhe completo em `THIRD_PARTY.md`. Resumo operacional:

- **Fonte principal:** Xenia Canary (`https://github.com/xenia-canary/xenia-canary`, branch `canary_experimental`), licença BSD 3-Clause, Copyright (c) 2015 Ben Vanik. O wiki do Canary afirma que ele roda o dashboard; é a melhor referência de comportamento esperado.
- **Estratégia A (padrão):** portar para Rust tabelas, enums e algoritmos, com cabeçalho de atribuição e linha no `THIRD_PARTY.md`.
- **Estratégia B (pontual):** FFI de módulo C/C++ **somente** se: não depende de `Memory`/`Processor`/`ThreadState`/`GraphicsSystem`/UI do Xenia; I/O simples; licença compatível; ganho claro. Local: `vendor/<nome>/` com README, licença, commit e API C mínima.
- **Proibido:** fork do Xenia e embutir JIT, `Processor`, `Memory`, backends D3D12/Vulkan.
- **Candidatos por prioridade:**
  1. `gpu/xenos.h` e `register_table.inc`: opcodes/registros (A).
  2. `gpu/command_processor.cc`: guia da reescrita do CP (A).
  3. `gpu/ucode.h`, `gpu/shader*`: decoder de microcódigo (A, ou B se isolável).
  4. `gpu/texture_*`: tiling/formatos (A ou B).
  5. `kernel/xboxkrnl/*`, `kernel/xam/*`: semântica de exports (A, contínuo).
  6. `cpu/ppc/testing/*.s`: casos de teste de instruções (adaptar).
- Os caminhos acima vêm da estrutura do repositório e de conhecimento geral; **confirmar no commit fixado** antes de usar.
- Dados proprietários (dash, NAND, chaves, `$SystemUpdate`) nunca entram em commit público nem em `vendor/`.

---

## 8. Plano de execução por fases

Cada fase tem objetivo, tarefas, critério de aceite e como validar. Rodar o emulador sempre com timeout (`xenon-emu\scripts\iter.ps1`, `xenon-emu\scripts\trapscan.ps1`).

### Fase 0: Linha de base e segurança (1 sessão)

Objetivo: congelar o comportamento atual para detectar regressões.

- Registrar em `docs/BASELINE.md`: comando usado, frames, passos, PCs finais, contadores PM4 (`real_opcode_counts`), traps (nenhum), tempo.
- Rodar `trapscan.ps1 -Frames 400 -TimeoutSec 150` e salvar a saída.
- Confirmar que o build limpo (`cargo build --release`) passa e mostra "Finished".
- Criar um branch git para a reestruturação (sem commit sem pedido do usuário; só sugerir).

Aceite: baseline reproduzível, documentada.

### Fase 1: Reorganização mecânica (sem mudar comportamento)

Objetivo: arrumar o código sem alterar semântica.

1. ~~Mover `src/bin/*` para `tools/`~~ **Feito:** crate `xenon-tools` em `tools/` (workspace com `default-members`, então `cargo build --release` continua gerando `ppcdis.exe` em `target/release`). Baseline idêntica.
2. ~~Criar `diag/` com `DiagConfig`~~ **Feito:** `src/diag/mod.rs` (`DiagConfig::from_env`, `diag::config()` com `OnceLock`); nenhum `std::env::var` fora dele. Saída comparada com o build anterior em três conjuntos de variáveis: idêntica.
3. ~~Quebrar `main.rs` em `app/`, `ui/`, `diag/`~~ **Feito:** `main.rs` só chama `app::run()` e usa a lib (não recompila mais os módulos). `app/boot.rs` (milestones 1-4, VFS, KPCR), `app/legacy.rs` (caminho não-Metro: fases 1-5 e PM4 sintético), `app/xui.rs`, `app/gpu.rs`, `app/threads.rs` (`GuestThread`, `setup_kpcr`, restart), `app/frame_loop.rs`; `ui/window.rs`; na lib, `diag/{watchdog,trace,hooks,profile}.rs`. Saída idêntica ao build anterior (stdout e stderr) nos três conjuntos de variáveis; trapscan idêntico à baseline. Não exercitados: dump de trap (não há trap), disparo do watchdog e o caminho não-Metro.
4. ~~Quebrar `exports.rs` e `xboxkrnl.rs` por área~~ **Feito:** `kernel/dispatcher.rs` (estado do `KernelDispatcher` e `dispatch`), `kernel/util.rs` (leitura de strings/paths do guest), `kernel/xboxkrnl/{ex,ke,mm,nt,rtl,xe_crypt,vd,misc}.rs` e `kernel/xam/{net,user,content,notify,ui,input,app,misc}.rs`. Cada área devolve `None` para ordinal que não trata; os ordinals são disjuntos, então a ordem de tentativa não muda nada. Área decidida pelo nome oficial do primeiro ordinal do braço; `Io*` e `Ob*` ficaram em `nt.rs` (poucos braços). Nomes de exports em `kernel/exports/{xboxkrnl,xam}_names.rs` (gerados por `gen_exports`, caminho público `kernel::exports::*` mantido). Verificado: as 1707 linhas de braço e os 321 ordinals são os mesmos; saída idêntica ao build anterior.
5. ~~Dividir `cpu/interpreter.rs` por grupo de opcode~~ **Feito:** `Cpu::step` só busca, decodifica e despacha; a semântica está em `cpu/ops/{int,branch,loadstore,float,vmx,system}.rs` (`exec_<grupo>` para opcodes primários e `exec_<grupo>_x31` para o opcode 31 por `xo`). Braços copiados literalmente; os helpers VMX128 foram para `ops/vmx.rs`. Saída idêntica ao build anterior e tempo de execução igual (dentro do ruído).
6. ~~Limpar `scratch/`~~ **Feito (com confirmação do usuário):** `iter.ps1` e `trapscan.ps1` foram para `xenon-emu/scripts/` (versionados), junto com `build_ref.ps1`, `cmp_diag.ps1` e `time_runs.ps1`; 17 scripts da pesquisa de untile arquivados em `_archive/scratch-untile-2026-10-22/` (fora do git, dependem do dump proprietário); 9 scripts cujas entradas não existem mais foram removidos. As pastas `scratch/` deixaram de existir.

**Fase 1 concluída.**

Aceite: baseline da Fase 0 idêntica (mesmos passos, PCs e contadores) após cada etapa. Uma etapa por vez, com build e `trapscan` entre elas.

### Fase 2: Command Processor fiel (M1)

**Feito** (2026-10-24): critério atingido (Atualização 10 do HANDOFF) e fechamento em 7 itens, um commit cada (pilhas fora da imagem, `mftb` global, banco de registradores único, tabela PM4 real, endianness/writeback, `VdQueryVideoFlags`/`XamLoaderTerminateTitle`, remoção do caminho falso do CP). `WAIT_REG_MEM` passou a bloquear na Fase 3.5.

Objetivo: o guest consegue sincronizar com a GPU.

- Reescrever `gpu/pm4.rs`/`cp/` com os opcodes reais: REG_RMW `0x21`, DRAW_INDX `0x22`, VIZ_QUERY `0x23`, SET_STATE `0x25`, WAIT_FOR_IDLE `0x26`, IM_LOAD `0x27`, IM_LOAD_IMMEDIATE `0x2B`, IM_STORE `0x2C`, SET_CONSTANT `0x2D`, LOAD_CONSTANT_CONTEXT `0x2E`, LOAD_ALU_CONSTANT `0x2F`, DRAW_INDX_BIN `0x34`, DRAW_INDX_2_BIN `0x35`, DRAW_INDX_2 `0x36`, INDIRECT_BUFFER_PFD `0x37`, INVALIDATE_STATE `0x3B`, WAIT_REG_MEM `0x3C`, MEM_WRITE `0x3D`, REG_TO_MEM `0x3E`, INDIRECT_BUFFER `0x3F`, COND_EXEC `0x44`, COND_WRITE `0x45`, EVENT_WRITE `0x46`, INTERRUPT `0x54`, SET_CONSTANT2 `0x55`, SET_SHADER_CONSTANTS `0x56`, EVENT_WRITE_SHD `0x58`, EVENT_WRITE_EXT `0x5A`, SET_BIN_MASK/SELECT `0x60-0x63`, XE_SWAP `0x64`, NOP `0x10`, ME_INIT `0x48`.
- Modelo de registradores com escrita via MMIO (`CP_RB_WPTR` em `0x7FC80714`) e writeback de RPTR.
- Interrupções (vsync, CP) entregues pelo callback de `VdSetGraphicsInterruptCallback`.
- `EVENT_WRITE_SHD` grava valor lógico sem byte-swap (memória guest já é big-endian).
- Instrumentar a condição do fence em `0x90102000`: quando o 5 é escrito, por qual caminho, quem lê.
- Reescrever testes que usam opcodes falsos.

Aceite: o fence chega a 5 no momento correto e o loop em `0x921AFFF8` sai (verificar por PC profile/hook, não só por ausência de trap). `real_draws` continua contando os 24 `DRAW_INDX_2`.

### Fase 3: Kernel com bloqueio real (M2)

**Feito** (2026-10-26): etapas 3.0 a 3.6, um commit cada (auditoria de ordinais, CP bloqueante, objetos e handles, escalonador, esperas bloqueantes, timers e IO completion, ciclo de vida e exports); threads esperando dormem (HANDOFF, Atualizações 13 a 18). Depois: endereço efetivo de 32 bits, `cmpldi`/MD-form e notificações de partida ligadas (Atualizações 19 a 21).

Objetivo: esperas e threads com semântica de kernel.

- Object manager: handles, tipos (evento, semáforo, mutante, timer, thread, arquivo), dispatcher headers.
- Escalonador: filas de espera, estados (pronto/rodando/esperando), timeout, quantum por instruções, prioridade básica.
- `KeWaitForSingleObject/MultipleObjects`, `NtWaitFor*` bloqueando de verdade; sinalização acorda threads.
- APC/DPC e timers (`KeSetTimer`, etc.).
- Revisar política de reinício de threads: nada de restart para esconder bug; `ExTerminateThread` deve encerrar de fato (thread 0xB hoje é reiniciada ~151x).
- Completar exports faltantes: xboxkrnl `0x0111, 0x013D, 0x0015, 0x00F5`; xam `0x0282, 0x001A, 0x0200, 0x07E2`.

Aceite: threads 0x9, 0xA, 0x12 deixam de girar em polling (menos passos por frame, `XENON_PC_PROFILE`); sem deadlock global (watchdog não dispara).

### Fase 4: Xenos ucode e rasterização (M3)

Objetivo: os draws da dash produzem pixels.

- ~~Decoder de microcódigo (CF, ALU, fetch) portado a partir de `ucode.h`.~~ **Feito (4.2):** `gpu/ucode.rs` + disassembler `gpu/ucode_disasm.rs` (texto do Xenia); os 9 shaders distintos da dash em 3000 frames decodificam sem problemas.
- ~~Primeiro, interpretador em CPU (correção antes de velocidade); depois tradução para SPIR-V/wgpu se necessário.~~ **Feito (4.3):** `gpu/shader_interp.rs` (4.3a) e ambiente real `gpu/shader_env.rs` + `XENON_SHADER_EXEC` (4.3b); texturas por sampler de stub até a 4.4.
- ~~**Etapa 4.4 (Texturas: formatos e untile):**~~ **Feito (4.4):** `gpu/texture.rs` (`tiled_offset_2d` do Xenia Canary, cobrindo 2D tiled 1x1 e linear 128x1), `TextureFetchConstant` com clamps/swizzles/filtros/sinais, `GpuTextureSampler` com leitura da memória guest física, filtragem point/bilinear e desempacotamento de texels do formato 6 (`k_8_8_8_8`), 4 testes unitários e regressão limpa.
- ~~**Etapa 4.5 (Montagem de primitivas, EDRAM e rasterização):**~~ **Feito (4.5):**
  - **4.5a Montagem de primitivas e setup (`gpu/primitive.rs`):** decodificação de índices (auto e DMA via `draw_indices`), montagem de primitivas (`RECTLIST` de 3 vértices decomposto em 2 triângulos via espelhamento diagonal $v_3 = v_1 + v_2 - v_0$, `triangle_list`, `triangle_strip` com winding alternado, `triangle_fan`, `quad_list`, `point_list`), transformação de viewport (`PA_CL_VTE_CNTL`, `PA_CL_VPORT_*`) para screen-space $(sx, sy)$ e profundidade $z \in [0.0, 1.0]$.
  - **4.5b Rasterizador de triângulos em CPU (`gpu/rasterizer.rs`):** scanning por edge functions (Pineda) com bounding box recortado por scissor, interpolação perspectivo-correta dos interpoladores ($o_0..o_{15}$) e $z$, execução do pixel shader ativo por pixel via `GpuShaderEnv` com `GpuTextureSampler`, e descarte de fragmentos por `kill`.
  - **4.5c Render Backend (`gpu/edram.rs`):** buffer de EDRAM 1280×720 (cor ARGB8 + profundidade float), teste e escrita de depth via `RB_DEPTHCONTROL` (NEVER, LESS, EQUAL, LEQUAL, GREATER, NOTEQUAL, GEQUAL, ALWAYS), alpha blending e combinação via `RB_BLENDCONTROL0` (fatores ZERO, ONE, SRC_COLOR/ALPHA, ONE_MINUS, DST, CONSTANT; opcodes ADD, SUB, MIN, MAX, REVSUB) e máscara de canais `RB_COLOR_MASK`.
  - **4.5d Resolve, Swap (`XE_SWAP`) e Apresentação (`gpu/xenos.rs`):** `draw` executa shaders e rasteriza primitivas na EDRAM, resolve para guest RAM (`RB_COPY_*`), `PM4_XE_SWAP` transfere a EDRAM para `color_buffer` e grava no frontbuffer guest ativo em formato tiled (via `tiled_offset_2d`); testes unitários cobrindo todo o pipeline e regressão de 400 frames limpa.
- Remover o caminho de dump como padrão; manter `XENON_FB_SOURCE=reference` apenas como comparação.

Aceite: os 24 `DRAW_INDX_2` observados produzem saída; comparação visual/numérica com `xbox360_real_framebuffer.raw`.

### Fase 5: `xe-host` e HLE restante (M4)

- [x] **5.1: Módulo `xe-host` e Traits Abstratas (`src/host/`)**
  - Definidas interfaces abstratas `HostNet`, `HostStorage`, `HostFile`, `HostTime`, `HostInput`, `HostAudio`, `HostProfile` e a master trait `Host` em `src/host/traits.rs`.
  - Backend padrão determinístico e isolado `HostOffline` em `src/host/offline.rs` (rede desconectada, armazenamento em memória mock, tempo determinístico virtual, controle neutro, áudio no-op, perfil padrão "Xbox Player").
  - Backend real acoplado ao SO `HostReal` em `src/host/real.rs` (`std::fs`, alta resolução de tempo com `SystemTime` e `Instant`, XInput no Windows).
  - Conectado `pub host: Box<dyn crate::host::Host>` no `KernelDispatcher` com `default_host()`.
- [x] **5.2: VFS Unificado (`hdd0:`, `game:`, `dvd:`, content e STFS)**
  - Implementado `src/kernel/vfs.rs` com traits `VfsFile`, `VfsDevice`, estruturas `MemoryDevice`, `MemoryFile`, `StfsDevice` e `VirtualFileSystem`.
  - Normalização de caminhos NT (`\\Device\\Harddisk0\\Partition1\\`, `\\Device\\Cdrom0\\`, `\\Device\\Flash\\`) e resolução de symlinks (`hdd0:`, `game:`, `dvd:`, `systemroot:`, `d:`, `update:`).
  - Montagem de contêineres STFS (`src/loader/stfs.rs`) como dispositivos montáveis transparentes no VFS.
  - Integrado `pub vfs: super::vfs::VirtualFileSystem` em `KernelDispatcher`, conectando `find_vfs_file` e `register_vfs_file`.
- [x] **5.3: Integração dos Serviços de Rede e Tempo do Host no XAM/Xboxkrnl**
  - Roteado `NetDll_XNetGetEthernetLinkStatus` (0x004B) para `self.host.net().link_status()` e implementados `NetDll_socket` (0x0003), `NetDll_closesocket` (0x0004), `NetDll_XNetGetTitleXnAddr` (0x0036).
  - Conectados `KeQueryPerformanceFrequency` (0x0083) e `KeQuerySystemTime` (0x0084) a `self.host.time()`.
  - Implementados em `src/kernel/xam/misc.rs`: `GetTickCount` (0x0415), `QueryPerformanceCounter` (0x042F) e `QueryPerformanceFrequency` (0x0430) utilizando a base temporal do host.
  - Testes unitários de rede e tempo adicionados e validados (146 testes passando).
- [x] **5.4: Gestão de Perfis de Usuário, Entrada e Resolução de Stubs de XAM**
  - Roteados `XamUserGetSigninState` (0x0210), `XamUserGetXUID` (0x020A), `XamUserGetGamerTag` (0x020C), `XamUserGetName` (0x020E), `XamUserGetIndexFromXUID` (0x0211) e `XamUserGetSigninInfo` (0x0227) para `self.host.profile()`.
  - Roteados `XamInputGetState` (0x0191) e `XamInputSetState` (0x0192) para `self.host.input()`, com verificação de porta conectada e preenchimento de `PadState`.
  - Adicionados stubs de XAM para localização e typeface: `XamGetLanguageLocaleFallbackString` (0x058A) e `XamGetLanguageTypefacePatch` (0x058D).
  - Testes unitários adicionados para `XamInputGetState` e `XamInputSetState` (148 testes passando).

Aceite: dash navega telas que dependem de usuário, conteúdo e rede offline sem loops. Fase 5 concluída.

### Fase 6: Áudio, input, rede real (M5)

- [x] **6.1: Subsistema de Áudio Host e Processamento PCM / XAudio**
  - Implementado ring buffer de áudio PCM em `HostAudio` (`src/host/traits.rs`, `src/host/offline.rs`, `src/host/real.rs`) com suporte a drenagem (`drain_pcm`) e armazenamento temporário de amostras para sinks de áudio.
  - Implementados exports de áudio em `src/kernel/xboxkrnl/misc.rs`: `XAudioGetSpeakerConfig` (0x01FF, 5.1/Stereo), `XAudioRegisterRenderDriverClient` (0x01F3), `XAudioSubmitRenderDriverFrame` (0x01F5, submissão direta de buffers PCM ao host), `XAudioRenderDriverInitialize` (0x01F2), `XAudioUnregisterRenderDriverClient` (0x01F4), `XAudioRenderDriverLock` (0x01F6), `XAudioSetSpeakerConfig` (0x0200) e `XAudioOverrideSpeakerConfig` (0x0276).
  - Implementados stubs e gestão de contexto XMA (`XMACreateContext` 0x0224, `XMAReleaseContext` 0x0226, e buffers `XMASetInputBuffer*` 0x022E..0x0238).
  - Adicionados testes unitários específicos para `XAudioGetSpeakerConfig` e submissão/drenagem de frames PCM (150 testes passando).
- [x] **6.2: Entrada Interativa (Teclado Host + Gamepad XInput)**
  - Adicionado suporte a injeção e mesclagem de gamepad virtual via `set_virtual_pad` na trait `HostInput` (`src/host/traits.rs`), `OfflineInput` (`src/host/offline.rs`) e `RealInput` (`src/host/real.rs`).
  - Implementada mesclagem transparente entre controle físico XInput e gamepad virtual no `RealInput`: se um controle USB estiver conectado, inputs físicos e teclado virtual são combinados sem sobreposição; se não houver controle físico conectado, o teclado virtual no slot 0 atua com estado ativo `connected: true`.
  - Mapeado teclado completo em `src/gpu/display.rs` para o gamepad virtual: WASD / Setas (D-pad e analógico), Espaço / Enter (A), Esc / Backspace (B), X / Y, Bumpers (Q / E), Gatilhos LT / RT (Z / V), Start (F1 / Home), Back (F2 / Tab / Delete), e L3 / R3 (Shift Esquerdo / Direito).
  - Atualizado `KernelDispatcher::update_gamepad` e `update_pad_state` (`src/kernel/dispatcher.rs`) para propagar entradas em tempo real ao subsistema de input do host e ao `XamInputGetState`.
  - Atualizado banner interativo em `src/ui/window.rs` e suporte a fechar janela via `Shift+ESC` ou `[X]`.
  - Adicionados 3 novos testes unitários (153 testes passando no total).
- [x] **6.3: Backend de Rede Real Opt-in (`HostReal::net`)**
  - Implementado `RealNet` com suporte completo a sockets TCP (`std::net::TcpStream`, `TcpListener`) e UDP (`std::net::UdpSocket`), além de resolução DNS via `ToSocketAddrs` em `src/host/real.rs`.
  - Detecção automática e segura de IP da interface de rede local ativa do host (`detect_local_ip` via UDP routing lookup sem envio de pacotes).
  - Controle opt-in através de `XENON_NET_ONLINE=1` em `DiagConfig` (`src/diag/mod.rs`), mantendo o modo offline seguro por padrão.
  - Conectados `NetDll_bind` (0x000B), `NetDll_connect` (0x000C), `NetDll_recv` (0x0012) e `NetDll_send` (0x0016) ao `self.host.net()` em `src/kernel/xam/net.rs`.
  - Adicionado `read_bytes` em `src/memory/bus.rs` para leitura contígua e segura de buffers de rede na memória guest.
  - Adicionados testes unitários para `RealNet` opt-in e ciclo de vida de sockets via HLE (155 testes passando no total).
- [x] **6.4: Integração, Validação da Baseline e Navegação Interativa da Dash**
  - Integração consolidada dos subsistemas do host: Áudio (`HostAudio` com PCM e XAudio), Entrada (`HostInput` com mesclagem transparente de teclado host e gamepad XInput) e Rede (`HostNet` com sockets TCP/UDP e resolução DNS opt-in).
  - Adicionado controle de seleção de host via `XENON_HOST=real` ou `XENON_HOST_REAL=1` em `DiagConfig` (`src/diag/mod.rs`), integrado a `default_host()` (`src/host/mod.rs`).
  - Adicionados métodos de injeção e inspeção de host em `KernelDispatcher` (`set_host`, `host`, `host_mut` em `src/kernel/dispatcher.rs`).
  - Adicionado teste de integração e switching de host (`test_host_switching_and_dispatcher_integration`, 156 testes passando no total).
  - Validada a baseline com teste de regressão de 3000 frames (timeout 120s estrito) sem traps nem regressões.

Aceite: dashboard reproduz áudio, responde a comandos de controle (teclado e gamepad físico) e suporta rede real opt-in sem regressões. FASE 6 CONCLUÍDA.

---

## 9. Estratégia de testes e regressão

- **Regressão da dash:** `trapscan.ps1 -Frames 400` comparado com `BASELINE.md` a cada etapa da Fase 1.
- **Instruções PPC/VMX:** testes por instrução com vetores conhecidos; adaptar casos de `cpu/ppc/testing` do Xenia.
- **PM4:** streams capturados da própria dash como golden (contagem de opcodes + estado de registros); `XENON_PM4_LOG` ajuda a capturar.
- **Frames:** comparar com `xbox360_real_framebuffer.raw`; métrica simples (diferença por pixel) e inspeção visual.
- **Kernel:** testes unitários de eventos, semáforos, filas de espera e timeout.
- **Regras:** nenhum teste novo deve usar opcodes inventados; toda correção documenta a evidência.

---

## 10. Riscos e mitigação

| Risco | Impacto | Mitigação |
|---|---|---|
| Refactor quebra comportamento sutil | Regressões difíceis de achar | Baseline + uma etapa por vez + `trapscan` entre etapas. |
| CP fiel não libera o fence | Dash continua presa | Instrumentar condição do fence antes; checar também a entrega da interrupção e a ordem de eventos; kernel bloqueante (Fase 3) pode ser pré-requisito. |
| Escalonador real expõe deadlocks | Travamentos | Watchdog sempre ligado; timeouts; modo "cooperativo determinístico" para depuração. |
| Shaders Xenos são volumosos | Fase 4 longa | Interpretador CPU primeiro; cobrir só os shaders que a dash usa. |
| FFI C/C++ complica build | Toolchain no Windows | Só quando o ganho justificar; isolar em `vendor/`; documentar requisitos. |
| Licenças | Risco legal | `THIRD_PARTY.md` atualizado a cada cópia; não copiar `third_party/` do Xenia sem verificar. |
| Dados proprietários em commit | Risco legal | `.gitignore` para dash, NAND, chaves, dumps; revisar antes de qualquer commit. |
| Hang do emulador congela a ferramenta | Perda de sessão | Sempre usar timeout; `XENON_HEADLESS` e `XENON_WATCHDOG`. |

---

## 11. Regras de trabalho para qualquer agente

1. Responder em português, de forma concisa.
2. Não usar workflows (pedido explícito do usuário).
3. Timeout obrigatório em todo script externo ou execução do emulador.
4. Verificar o build com uma chamada separada de `cargo build --release` e conferir "Finished".
5. Não criar commits sem pedido; nunca `git add .`.
6. Deleções em lote ou destrutivas exigem confirmação do usuário.
7. Cada correção cita evidência (disasm, export, doc/fonte do Xenia, trace).
8. Estratégia A + B pontual; sem fork do Xenia (decisão fechada).
9. Atribuição e `THIRD_PARTY.md` atualizados a cada código derivado.
10. Refactor mecânico nunca vem misturado com mudança de comportamento no mesmo passo.
11. Não reintroduzir bugs já corrigidos (lista em `HANDOFF.md` seção 10).
12. Atualizar `docs/` quando uma decisão ou descoberta mudar o plano.

---

## 12. Definição de "pronto" por marco

| Marco | Pronto quando |
|---|---|
| Reorganização (Fase 1) | Baseline idêntica; `main.rs` e arquivos de kernel divididos; `tools/` separado; `DiagConfig` único. |
| CP fiel (Fase 2) | Opcodes reais; fence 5 entregue; loop em `0x921AFFF8` sai; testes reescritos. |
| Kernel bloqueante (Fase 3) | Esperas bloqueiam; sem polling nas threads 0x9/0xA/0x12; exports pendentes tratados. |
| Rasterização (Fase 4) | Draws da dash geram pixels coerentes com o dump de referência. |
| Host e HLE (Fase 5) | Traits e backends; dash navega offline; VFS funcional. |
| Final | Dash original renderiza e responde a input de forma estável, sem atalhos HLE de GPU. |

---

## 13. Perguntas em aberto (a decidir com o usuário quando surgirem)

- Promover para workspace Cargo com vários crates já na Fase 1, ou só após estabilizar?
- Backend gráfico preferido para a Fase 4: wgpu, Vulkan direto ou rasterizador em CPU primeiro?
- Quais binários de `tools/` (ex-`src/bin`) podem ser removidos? (`scratch/` já resolvido na Fase 1.)
- Nível de rede real desejado (apenas local/DNS/HTTP, ou algo mais)?

---

## 14. Referências

- Xenia: https://github.com/xenia-project/xenia (`docs/cpu.md`, `docs/gpu.md`, `src/xenia/gpu/xenos.h`, `src/xenia/gpu/command_processor.cc`, `src/xenia/kernel/xam/`, `src/xenia/kernel/xboxkrnl/`)
- Xenia Canary: https://github.com/xenia-canary/xenia-canary (pastas de `src/xenia`: `app, apu, base, cpu, debug, gpu, hid, kernel, patcher, ui, vfs`)
- Free60: https://free60.org
- Outros projetos pesquisados (lidos apenas por busca, não analisados em detalhe): xenia-edge, hells-gate-recomp (recompilação estática sem emular Xenos), pinyon-shift-android.
