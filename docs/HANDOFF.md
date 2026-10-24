# xenon-emu: contexto geral e plano (handoff para o próximo agente)

Idioma de trabalho com o usuário: **português**. Respostas concisas.

## 0. Regras de operação do usuário (obrigatórias)

- **Não usar workflows** (o usuário pediu explicitamente). Trabalhar direto.
- **Sempre usar timeout** ao executar o emulador ou scripts externos. Travar a ferramenta congela a sessão.
  - `.\xenon-emu\scripts\iter.ps1 -Frames N [-NoBuild] [-TimeoutSec 60] [-Env 'XENON_X=1',...]` (saída em `%TEMP%\iter_run.txt` e `.err`; roda a partir da raiz do workspace)
  - `.\xenon-emu\scripts\trapscan.ps1 -Frames 400 -TimeoutSec 150`
  - Refactor sem mudança de comportamento: `build_ref.ps1`, `cmp_diag.ps1`, `time_runs.ps1` em `xenon-emu\scripts\` (ver `docs/BASELINE.md`).
  - Verificar o build com uma chamada separada de `cargo build --release` e conferir "Finished" (a saída de build às vezes é engolida). O `XenonEmu.exe` na raiz pode estar travado.
- Não criar commits sem pedido explícito. Nunca `git add .`.
- **Decisão do usuário (fechada): estratégia A + B pontual.** Portar para Rust usando o Xenia/Canary como especificação; chamar C/C++ **apenas** para módulos isolados e de baixo acoplamento (via FFI). **Proibido** fazer fork do Xenia ou embutir seu runtime (JIT, `Processor`, `Memory`, backends D3D12/Vulkan, `GraphicsSystem`). O produto continua sendo o xenon-emu em Rust.
- Todo código derivado do Xenia exige o cabeçalho de atribuição e uma linha em `docs/THIRD_PARTY.md` (commit fixado, arquivos).
- Todo módulo C/C++ externo fica isolado em `vendor/` (ou `third_party/`) atrás de uma API C pequena e um wrapper Rust seguro. Nada de C++ espalhado pelo código do emulador.
- Toda correção deve citar evidência (disassembly, ordinal de export, doc/fonte do Xenia). Vários bugs anteriores vieram de palpites (tabela PM4 inventada, `lvsl`/`lvewx` com xo errado).

## 1. Objetivo

Rodar o **dashboard retail original do Xbox 360 (`dash.xex`, build 17559)** como código guest, "como se fosse um Xbox 360 real".

- Imagem da dash: `D:\XBOX 360 - Firmeware Emulator\SystemUpdate_17559_USB\$SystemUpdate\dash.xex` (base `0x92000000`, entry `0x92196610`, stack_top `0x93018000`).
- Projeto: `D:\XBOX 360 - Firmeware Emulator\xenon-emu` (Rust). Módulos: `cpu`, `crypto`, `gpu`, `kernel`, `loader`, `memory`, mais `tools/` (crate `xenon-tools` no workspace, 42 ferramentas em `tools/src/bin`).
- Extras na raiz: `extracted_xui/` (XUR/XUI extraídos), `Corona 16mb/` (dump de NAND), `_archive/` (scripts de análise antigos, fora do git: pesquisa de untile em `scratch-untile-2026-10-22/`), `_git_backup/` (o `.git` contaminado anterior), `xbox360_real_framebuffer.raw` (dump de referência do framebuffer real).

## 2. Estado atual (resumo honesto)

| Camada | Abordagem | Estado |
|---|---|---|
| CPU PPC | LLE, interpretador (`cpu/interpreter.rs` decodifica; semântica em `cpu/ops/{int,branch,loadstore,float,vmx,system}.rs`; `cpu/altivec.rs`) | Estável. Thread principal roda 400+ frames sem trap (~8,6M passos). Faltam `vpkd3d128` e `vupkd3d128` tipos 2 e 6. |
| Kernel (xboxkrnl) | HLE (`kernel/dispatcher.rs`, `kernel/xboxkrnl/*`, `kernel/xam/*`, nomes em `kernel/exports/`) | Funcional, mas **esperas não bloqueiam** (`KeWait*`/`NtWait*` retornam sucesso). Sem escalonador, sem object manager real. |
| XAM | HLE (stubs offline) | Suficiente para a dash avançar; sem UI, notify ou rede reais. |
| GPU (Xenos) | Improvisada (`gpu/xenos.rs`, `gpu/pm4.rs`, `gpu/shader.rs`, `gpu/display.rs`) | A tabela PM4 de `pm4.rs` é **inventada** (só NOP 0x10, WAIT_REG_MEM 0x3C, INDIRECT 0x3F, ME_INIT 0x48 batem). Shaders ucode não são executados. 0 draws rasterizados. |
| Apresentação | `BufferSource::LiveEmulation` (guest) por padrão; `XENON_FB_SOURCE=reference` usa o dump; tecla `[B]` alterna. |

**Bloqueio atual (confirmado em 2026-10-24, Fase 2):** a thread principal **não** está presa no fence. O `main` da dash (`0x921870A0`, entra no frame 3) **retorna `0x80070057` (E_INVALIDARG) no frame 255**, e o CRT encerra o título. Cadeia, verificada com `XENON_PC_HOOK` nos pontos de retorno:

1. `mainCRTStartup` (`0x92196780..0x921967C4`): `r30 = main(argc, argv)` → `DbgPrint("[XAPI RETURN VALUE] %d\n", r30)` (fmt em `0x9200557C`) → `XamLoaderTerminateTitle` (xam `0x01A9`). Nosso HLE retorna desse export (no console ele não retorna), a execução cai na função seguinte `0x921967C8` com `LR=0x921967C8` e gira para sempre (é o laço quente `0x921967C8`/`0x92198658` do profile).
2. `main`: `r30 = 0x92186CD8(app)`; com `r30 < 0` pula `0x92185790`, chama o shutdown `0x92183938` (frames 216..255) e retorna `r30`.
3. `0x92186CD8`: `0x92186AA0` ok (frame 128); `0x921860B8` retorna E_INVALIDARG (frame 216).
4. `0x921860B8`: `0x92185650` (chamado em `0x9218621C`) retorna E_INVALIDARG.
5. `0x92185650`: `0x921887D8(0, L"dashuiskin.xzp", L"skin.xur", buf, 128)` monta o locator XUI e `0x921E8FB8(buf, 0)` o carrega. O buffer chega **vazio**, e o carregamento retorna E_INVALIDARG.
6. `0x921887D8` → `0x92187BE0`: `_snwprintf(buf, 128, L"section://%X,%s%s%s", hModule, nome, L"#", L"skin.xur")` (fmt em `0x92003624`; `nome` = até 8 caracteres de `dashuiskin` antes do `.`, passados por `0x921996E0`; em execução resultou em `dashuisk`, minúsculo, que é o nome da seção de recurso); sucesso se o retorno ≥ 0.
7. **Causa raiz: `_snwprintf` (xboxkrnl `0x013D`) não tem HLE.** O default devolve 0 sem escrever nada ("Unhandled xboxkrnl.exe ordinal 0x013D", 12× em 220 frames). A família toda está quebrada: `_snprintf`/`sprintf` (`0x013A`/`0x013B`) e `_vsnprintf`/`_vscwprintf`/`vswprintf` (`0x014D`/`0x014F`/`0x0151`) retornam 0 sem escrever.

O laço de fence em `0x921AFFF8..0x921B0020` (fence `0x90102000`, valor 5) visto antes pertence ao período de shutdown (frames 216..255), não ao bloqueio. Referência para a implementação: Xenia `src/xenia/kernel/xboxkrnl/xboxkrnl_strings.cc` (formatação estilo MSVC, `%s` em funções wide = string wide).

**Atualização (2026-10-24, depois da família printf em `src/kernel/strings.rs`):** `_snwprintf` agora monta o locator, que sai `section://0,dashuisk#skin.xur`. O carregamento da skin passa a falhar com `0x8030001C` (frame 218; o `main` retorna esse código no frame 258). Próximo bloqueio, com evidência:

- **`XexGetModuleHandle` (xboxkrnl `0x0195`) está errado.** A dash chama `GetModuleHandle(NULL)` em `0x921927C8` → `XexGetModuleHandle(NULL, &h)`; nosso HLE devolve `0x92000000` como **status** e não escreve `*h`. A dash lê 0 da pilha e grava `[0x92969854] = 0` em `0x92187108` (watchpoint). Daí o `0` no locator.
- **`XexGetModuleSection` (`0x0196`) ignora o nome** e sempre devolve `0x92105000`/`0x58A894`. A seção certa vem da tabela de recursos do cabeçalho XEX (`0x000002FF`, em `0x1AAC` no arquivo): 36 entradas de `{nome[8], endereço, tamanho}`, por exemplo `dashuisk` em `0x92C51700`, tamanho `0x127F0`. Referência: Xenia `xboxkrnl_modules.cc` e `UserModule::GetSection`.
- **Sobreposição da pilha com recursos:** `stack_top = vaddr_base + image_size - 0x10000 = 0x93018000` fica **dentro** do recurso `hubapp` (`0x92FF6780..0x93026A12`). A pilha da thread principal cresce para baixo sobre esse recurso (e `arcade` vai até `0x93027EF2`). A pilha precisa sair da imagem.

**Atualização 2 (2026-10-24, depois de `XexGetModuleHandle`/`XexGetModuleSection` em `src/kernel/modules.rs`):** os módulos (dash, `xboxkrnl.exe`, `xam.xex`) têm `X_LDR_DATA_TABLE_ENTRY` em `0x931F0000 + 0x70·i`, e o handle é o endereço dessa entrada. O locator sai correto (`section://931F0000,dashuisk#skin.xur`), e o carregamento da skin passa a falhar com `0x80004005` (E_FAIL; o `main` retorna esse código no frame 257). Nomes pedidos pela dash: `NULL`, `xam.xex` (×2), `xboxkrnl.exe`, `xbdm.xex` (este dá NOT_FOUND, como num console retail).

Próximo bloqueio, com evidência: **todas** as chamadas de `XexGetModuleSection` (frames 85, 104, 122, 127, 128 e 217) retornam `X_STATUS_NOT_FOUND`, porque o nome da seção chega como lixo. A dash separa o locator em `0x922192C0` (handle em hex até a `,`, por `0x92219180`) e converte o nome wide para narrow em `0x92219218` → `0x92192B60` (`WideCharToMultiByte`) → **`RtlUnicodeToMultiByteN` (xboxkrnl `0x0143`), que é um stub `=> 0`** (não escreve nada). O inverso `RtlMultiByteToUnicodeN` (`0x0133`) também é stub. Referência: Xenia `xboxkrnl_rtl.cc`.

**Atualização 3 (2026-10-24, depois de `RtlUnicodeToMultiByteN`/`RtlMultiByteToUnicodeN`):** `XexGetModuleSection` passa a achar as seções (retorna 0 nos frames 85, 162, 186, 301...), a skin carrega e **o `main` deixa de retornar**. A dash executa a inicialização real por cerca de 1100 frames (1500 frames levam ~13 s headless).

Próximo bloqueio: no **frame 1100** a thread principal para em `0x9221FA28` com `Unimplemented VMX128 vpkd3d128 (insn=0x1BE166D4)`. Em `cpu/ops/vmx.rs`, `vpkd3d128` não tem nenhuma variante implementada (só existe `vupkd3d128` tipos 0, 1, 3, 4, 5). Referência: Xenia `src/xenia/cpu/ppc/ppc_emit_altivec.cc` (`InstrEmit_vpkd3d128`) e o encoding VX128_4 em `ppc_instr.h`.

Observação: com a dash seguindo o caminho normal, o PM4 para no frame 28 (72 pacotes). Os 120 pacotes e os 24 `DRAW_INDX_2` da baseline vinham do **caminho de erro/shutdown** (frames 216..255), não da inicialização normal. A thread 0x3 continua girando em `RtlEnterCriticalSection`/`RtlLeaveCriticalSection` (`0x0125`/`0x0130`) e `0x00F4` (esperas não bloqueantes, Fase 3).

Dados do PM4 abaixo continuam válidos como observação.

Stream PM4 observado: `0x48, 0x3F×7, 0x3C, 0x21×2, 0x3B×3, 0x2B×2, 0x36×24, 0x10, 0x58×2, 0x60-0x63`.

## 3. Mapa de código (tamanhos aproximados)

`cpu/ops/` (ex-`interpreter.rs`; maiores: `int.rs` ~890, `vmx.rs` ~770, `loadstore.rs` ~540), `kernel/` (`dispatcher.rs` estado + entrada, `util.rs`, `xboxkrnl/{ex,ke,mm,nt,rtl,xe_crypt,vd,misc}.rs`, `xam/{net,user,content,notify,ui,input,app,misc}.rs`, `exports/` tabelas de nomes geradas), `gpu/display.rs` 1707, `app/` (ex-`main.rs`: `boot`, `frame_loop`, `threads`, `legacy`, `xui`, `gpu`), `ui/window.rs`, `diag/` (`watchdog`, `trace`, `hooks`, `profile`), `gpu/xenos.rs` 876, `loader/xex.rs` 544, `gpu/shader.rs` 417, `memory/bus.rs` 281, `cpu/altivec.rs` 262.

Memória:
- Janela virtual guest `0x40000000..0x8FFFFFFF` com backing privado sob demanda em chunks de 64KB do pool físico `0x14000000..0x1FFFFFFF`. `valloc_bump` começa em `0x40000000`.
- `phys_bump` separado (`0x90100000`, para `MmAllocatePhysicalMemoryEx`; deve ficar `< 0x92000000`).
- MMIO da GPU em `0x7FC80000..0x7FC8FFFF` interceptado no bus (não é RAM).
- KPCR por thread: `0x93100000 + slot*0x2000` (`setup_kpcr`). Um KPCR compartilhado vazava last-error entre threads.

Threads da dash (entrada): principal Metro; dinâmicas 0x1 (`0x92261CE8`), 0x2 (`0x921C9830`), 0x3/0x4 (`0x9281FBC0`, XUI), 0x5-0x8 (`0x92267C58`), 0x9 (`0x9226A2A8`, KeWaitForMultipleObjects + spinlock por frame), 0xA (`0x927C6D08`, loop XamUser*), 0xB/0xC (`0x9226E410`; 0xB chama ExTerminateThread ~151x e é reiniciada), 0xD..0x13 (ver `XENON_SYSCALL_SUMMARY`).

Exports HLE ainda não tratados (vistos em 400 frames depois da família printf): xboxkrnl `0x0015, 0x004B, 0x0082, 0x00F5, 0x0111`; xam `0x001A, 0x0200, 0x0282, 0x0291, 0x03CA, 0x058A, 0x058C, 0x058D, 0x059F, 0x05DC, 0x05E7, 0x07E2`. Família printf (`0x013A`, `0x013B`, `0x013D`, `0x013E`, `0x014D`..`0x0151`) implementada em `src/kernel/strings.rs`, com testes (`cargo test --release --lib kernel::strings`).

## 4. Ferramentas de depuração existentes

Variáveis de ambiente (todas lidas uma única vez em `src/diag/mod.rs` → `diag::config()`; variável nova entra lá): `XENON_HEADLESS`, `XENON_WATCHDOG` (4s sem progresso imprime frame/pc/lr/syscall, exit 86), `XENON_HEARTBEAT`, `XENON_SYSCALL_LOG`, `XENON_LOG_SYSCALLS` (log de cada despacho HLE no kernel), `XENON_SYSCALL_SUMMARY`, `XENON_PC_PROFILE`, `XENON_PC_HOOK=0xA,0xB` (+`_LIMIT/_MIN_FRAME/_ALL/_MEM=reg:hexoff:words/_LIST=reg/_STRREGS/_R3STR`), `XENON_WATCH=0xADDR` (+`XENON_WATCH_MEM`, `XENON_WATCH_DUMP_HIT`), `XENON_TRAP_TRACE` (+`_FULL`, `XENON_DUMP_RING_FRAME`, `XENON_DUMP_RING_N`), `XENON_NO_RESTART`, `XENON_RESTART_MAIN`, `XENON_PM4_LOG`, `XENON_GPU_MMIO_LOG`, `XENON_FB_SOURCE=reference`, `XENON_SKIP_BOOTANIM`, `XENON_MAX_FRAMES`.

Disassembler: `.\target\release\ppcdis.exe` (a partir de `xenon-emu`): `START END`, `words ADDR N`, `str ADDR..` (UTF-16), `refs IMM16 HI16`, `lis IMM`, `callers ADDR`, `importcallers ExportName`.

## 5. Decisão estratégica: reutilizar código de terceiros

O usuário perguntou se podemos usar código C/C++ já pronto (Xenia / Xenia Canary) em vez de recriar CPU/GPU. **Sim, é permitido** (licença BSD 3-Clause, ver `THIRD_PARTY.md`) desde que se mantenha o aviso de copyright e a licença. Mas há limites técnicos que o próximo agente deve considerar antes de decidir.

### 5.1 O que é realmente reaproveitável (e como)

Classificação com base na estrutura do repo `xenia-canary/src/xenia` (pastas: `app, apu, base, cpu, debug, gpu, hid, kernel, patcher, ui, vfs`). **Não li o código-fonte de todas; as colunas "acoplamento" são avaliação a verificar.**

| Componente Xenia | Pasta | Reuso | Acoplamento |
|---|---|---|---|
| Definições Xenos (opcodes PM4, registros, formatos, enums) | `gpu/xenos.h`, `gpu/register_table.inc` | Alto. Dados puros, portar para Rust ou usar via `bindgen`. | Baixo |
| Decoder/disassembler de microcódigo de shader | `gpu/ucode.h`, `gpu/shader.*`, `gpu/ucode_disassembler` | Alto. | Baixo a médio |
| Tradutor de shader (DXBC/SPIR-V) | `gpu/dxbc_shader_translator*`, `gpu/spirv_shader_translator*` | Médio. Útil como referência da semântica; depende de infraestrutura Xenia. | Médio |
| Command processor (PM4) | `gpu/command_processor.cc` | Médio. **Melhor referência para reescrever em Rust**; acoplado a `GraphicsSystem`, `Memory`, `RegisterFile`. | Alto |
| Texturas (tiling/untile/formatos) | `gpu/texture_*`, `gpu/texture_conversion` | Alto. Funções quase puras. | Baixo |
| EDRAM / render target cache | `gpu/render_target_cache*`, `gpu/draw_util` | Referência | Alto |
| CPU PPC (frontend, HIR, JIT x64/a64) | `cpu/ppc/`, `cpu/compiler/`, `cpu/backend/` | Baixo para integrar; é um JIT acoplado a `ThreadState`/`Memory`/`Processor`. **Não substitui nosso interpretador.** Serve como referência de semântica (`ppc_emit_*.cc`) e de testes (`cpu/ppc/testing/*.s`). | Alto |
| Kernel xboxkrnl / XAM / objetos | `kernel/xboxkrnl/`, `kernel/xam/`, `kernel/xobject*` | Referência primária para semântica de exports (assinaturas, retornos, estruturas). Reescrever em Rust. | Alto |
| VFS / STFS / XContent | `vfs/` | Referência/portar | Médio |
| Loader XEX | `kernel/util/xex2*`, `cpu/xex_module` | Referência (já temos `loader/xex.rs`) | Médio |
| APU/XMA | `apu/` (usa libavcodec/ffmpeg) | Via FFI para ffmpeg | Médio |

### 5.2 Estratégia adotada (decisão do usuário)

- **A. Portar para Rust, usando o Xenia como especificação. É a base.** Traduzimos tabelas, enums e algoritmos, mantendo atribuição. Vale para CP/PM4, kernel, XAM, VFS e semântica de CPU.
- **B. FFI pontual com módulos C/C++ isolados.** Somente peças de baixo acoplamento (candidatos: decoder/disassembler de microcódigo, untile/conversão de texturas, talvez o tradutor de shader se for separável; ffmpeg para XMA no futuro). Integração via crate `cc`/`cmake` + `bindgen`, atrás de uma API C pequena e wrapper Rust seguro. Exige toolchain MSVC/clang.
- **C. Fork do Xenia Canary: descartado.** Não fazer fork nem embutir o runtime do Xenia.

Critério para aceitar um módulo C/C++ via B: (1) não depende de `Memory`, `Processor`, `ThreadState`, `GraphicsSystem` ou da UI do Xenia; (2) entradas e saídas são buffers/structs simples; (3) licença compatível e registrada; (4) ganho claro frente a portar para Rust. Se falhar em qualquer item, portar para Rust (A).

Não encaixar o JIT nem os backends gráficos do Xenia no nosso runtime: o acoplamento é grande.

### 5.3 Antes de copiar qualquer coisa

1. Fixar um commit específico do Xenia Canary e registrar em `THIRD_PARTY.md` (hash, arquivos usados, licença).
2. Manter o cabeçalho de copyright nos arquivos derivados.
3. Verificar a licença de cada dependência que vier junto (ffmpeg é LGPL/GPL conforme build; `third_party/` do Xenia tem licenças próprias). Não copiar `third_party/` sem checar.

## 6. Arquitetura alvo

Fronteiras (modelo Xenia):
- CPU: LLE (interpretador; JIT opcional no futuro).
- Kernel e XAM: HLE no nível de export, com semântica real (objetos, esperas bloqueantes, escalonador, APC/DPC, timers).
- GPU: **LLE no nível PM4**. O D3D do console é código da própria dash, não há API de GPU para simular. Proibido atalho HLE que "finge" resultado de GPU (ex.: escrever fence direto).
- Hypervisor/firmware de boot: fora do escopo.

Organização de módulos alvo:

```
xe-cpu/    decode, interpreter, altivec/vmx128
xe-mem/    bus, MMIO, janelas virtuais, físico
xe-kernel/ objects/ sched/ xboxkrnl/(ex,io,ke,mm,nt,rtl,crypt,vd) xam/(user,content,notify,net,ui) vfs/
xe-gpu/    regs, cp/ (PM4 real), ucode/, shader/, edram/, texture/
xe-loader/ xex, xzp, pe
xe-host/   ponte para o PC real (ver seção 7)
vendor/    módulos C/C++ isolados (estratégia B), cada um com README, licença, commit de origem e API C mínima
xe-ui/     janela, apresentação, input
xe-diag/   hooks XENON_* centralizados em DiagConfig
tools/     os binários de src/bin
tests/     PPC por instrução, PM4, shaders, golden frames
```

Refactors mecânicos de baixo risco: quebrar `main.rs` em `app/ ui/ diag/`; dividir `exports.rs` e `xboxkrnl.rs` por área; mover `src/bin/*` para `tools/`; centralizar `XENON_*`.

## 7. Camada de ponte com o host ("drivers fakes")

Ideia do usuário: interceptar acessos a dispositivos (rede a cabo, etc.) e devolver ao emulador via hooks que conversam com o sistema real do PC.

Esta ideia é compatível com a arquitetura HLE: os exports do kernel/XAM são o ponto de interceptação natural. O que a dash chama (ex.: `NetDll_*`, `XNet*`, `XHttp*`, `XamUser*`, `XamContent*`) é traduzido para APIs do host. Proposta:

```rust
// xe-host: traits implementadas pelo backend real (Windows) ou fake (testes/offline)
trait HostNet     { fn socket(..); fn connect(..); fn send(..); fn recv(..); fn link_status() -> LinkStatus; fn dns_resolve(..); }
trait HostStorage { fn open(guest_path) -> HostFile; /* mapeia hdd0:, usb, dvd, content */ }
trait HostTime    { fn now() -> FileTime; }
trait HostInput   { fn poll_pad(slot) -> PadState; }
trait HostAudio   { fn submit(pcm); }
trait HostProfile { fn signed_in_users() -> Vec<UserInfo>; }
```

- Camada kernel/XAM chama apenas as traits. Dois backends: `host-real` (sockets Winsock, arquivos, XInput/SDL, WASAPI) e `host-offline` (respostas estáticas, usado hoje: signin local, XHttpStartup=TRUE, XNetStartup=0, `XNetGetEthernetLinkStatus`=0).
- Referência de como o Xenia faz: `kernel/xam/xam_net.cc`, `kernel/xboxkrnl/xboxkrnl_*`, `kernel/xsocket.*`, `kernel/xam/apps/`, `hid/` (SDL, XInput), `apu/` (SDL/XAudio2/WASAPI).
- Cuidados: (1) rede Xbox Live real exige serviços/autenticação; mapear só rede local/DNS/HTTP genérico; (2) manter modo offline como padrão; (3) a ponte não deve vazar dados do usuário; (4) estado de link/IP vem do host, mas é apresentado ao guest com estruturas XNet (`XNADDR`, `XNET_ETHERNET_LINK_*`).
- Dispositivos de baixo nível (USB, SATA, NAND) **não** precisam ser emulados na dash: o HLE de `Io*`/`Nt*File`/`XamContent*` com VFS mapeado para pastas do host cobre isso.

## 8. Marcos (ordem recomendada)

1. **M1 Command Processor fiel.** Substituir a tabela de `gpu/pm4.rs` pelos opcodes reais (lista abaixo), writeback de RPTR, interrupções (vsync/CP) entregues via callback de `VdSetGraphicsInterruptCallback`; reescrever testes que usam opcodes falsos. Critério: fence chega a 5 e o loop em `0x921AFFF8` sai. Referência: `gpu/command_processor.cc`, `gpu/xenos.h`.
2. **M2 Kernel com bloqueio real.** Escalonador, wait queues, eventos/semáforos/timers/APC. Critério: threads 0x9/0xA/0x12 deixam de girar em polling.
3. **M3 Xenos ucode + rasterização.** Decoder ucode, interpretador em CPU (ou tradutor para SPIR-V/wgpu), texturas com tiling, EDRAM/resolve. Critério: os 24 `DRAW_INDX_2` produzem pixels.
4. **M4 `xe-host` + HLE/XAM restante.** Traits do host, VFS, exports faltantes, `vpkd3d128`/`vupkd3d128` (tipos 2 e 6).
5. **M5 Áudio/XMA, input, rede real.**

Opcodes PM4 reais (Xenia `xenos.h`): REG_RMW 0x21, DRAW_INDX 0x22, VIZ_QUERY 0x23, SET_STATE 0x25, WAIT_FOR_IDLE 0x26, IM_LOAD 0x27, IM_LOAD_IMMEDIATE 0x2B, IM_STORE 0x2C, SET_CONSTANT 0x2D, LOAD_CONSTANT_CONTEXT 0x2E, LOAD_ALU_CONSTANT 0x2F, DRAW_INDX_BIN 0x34, DRAW_INDX_2_BIN 0x35, DRAW_INDX_2 0x36, INDIRECT_BUFFER_PFD 0x37, INVALIDATE_STATE 0x3B, WAIT_REG_MEM 0x3C, MEM_WRITE 0x3D, REG_TO_MEM 0x3E, INDIRECT_BUFFER 0x3F, COND_EXEC 0x44, COND_WRITE 0x45, EVENT_WRITE 0x46, INTERRUPT 0x54, SET_CONSTANT2 0x55, SET_SHADER_CONSTANTS 0x56, EVENT_WRITE_SHD 0x58, EVENT_WRITE_EXT 0x5A, SET_BIN_MASK/SELECT 0x60-0x63, XE_SWAP 0x64.

Já implementado em `gpu/xenos.rs`: contadores `real_opcode_counts`, `real_draws`, `pending_interrupts`, arms para 0x58/0x46/0x45/0x54/0x36/0x35/0x22/0x34 e opcodes de estado como no-op; `WAIT_REG_MEM` com formato `[wait_info, poll, ref, mask, wait]`; `REG_RMW` com 3 palavras; WPTR lido de `bus.gpu_mmio[0x714]`. `EVENT_WRITE_SHD` grava o valor lógico sem byte-swap (a memória guest já é big-endian).

## 9. Próximos passos imediatos para o agente que assumir

1. Ler esta pasta `docs/` e `THIRD_PARTY.md`.
2. A estratégia já está decidida (seção 5.2): A + B pontual, sem fork. Não reabrir essa discussão.
3. Executar M1: instrumentar a condição do fence (`0x90102000`) após `EVENT_WRITE_SHD`, conferir o momento em que o 5 é escrito e quem o lê; implementar CP fiel a partir do `command_processor.cc` do Xenia Canary.
4. Em paralelo, refactors mecânicos (seção 6) sem alterar comportamento; validar com `trapscan.ps1`.

## 10. Bugs já corrigidos (não reintroduzir)

- `subf.` ignorava Rc; `lvsl` (xo 6), `lvsr` (38), `lvewx` (71), `lvehx` (39; 19 é `mfcr`) corrigidos; `stvewx128` mantém op7=24; adicionados `lvsl128/lvsr128`.
- `addic/addic./addc/adde/addze/subfc/subfe` agora 64-bit; formas Rc usam `cmpd`; adicionados `lwbrx/stwbrx/lhbrx/ldbrx`, `lwzux` e afins, `lwax/lhax`, `mulhd/mulhdu`, `cntlzd`, `addme/subfme/subfze`, `mfcr/mfocrf`, `mtcrf/mtocrf`.
- AltiVec VX completo em `cpu/altivec.rs`; VA-form xo 32..47; VMX128 ops 5/6 reescritos segundo `vmx128.txt` do Xenia; `vsldoi128`; `vupkd3d128` tipos 0,1,3,4,5.
- Aliasing de memória (`addr & 0x1FFFFFFF`) substituído por janela virtual com backing privado.
- `RtlFillMemoryUlong` (xboxkrnl `0x0126`) tinha ordem de argumentos errada `(dst,len,pattern)` e causava loop de 2^64 iterações (a causa real do "crash" no frame 126).
- Reiniciar a thread Metro reexecutava o CRT sobre globals inicializadas (InsertTailList duplo). Não reiniciar por padrão (`XENON_RESTART_MAIN=1` restaura).
- HLE adicionados: `XamGetOnlineSchema (0x01CB)`, `XamUserGetUserFlags 0x021C`, `GetUserTenure 0x03C3`, `SubscriptionType 0x03C4`, `ReadProfileSettings 0x0219`, `XamLoaderGetMediaInfo 0x01A3`, `XamQueryLiveHiveW 0x070D`, `WSAStartup`, `XNetStartup 0x33`, `XHttpStartup/Shutdown 0xC9/0xCA` (=1), `XNetGetEthernetLinkStatus 0x4B` (=0), `XeCryptRandom 0x018A` (inferido da tabela de exports gerada).
