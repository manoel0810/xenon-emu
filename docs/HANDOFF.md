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

- Imagem da dash: `D:\XBOX 360 - Firmeware Emulator\SystemUpdate_17559_USB\$SystemUpdate\dash.xex` (base `0x92000000`, entry `0x92196610`; pilha principal com topo em `0x71400000`, fora da imagem — ver `app/threads.rs`).
- Projeto: `D:\XBOX 360 - Firmeware Emulator\xenon-emu` (Rust). Módulos: `cpu`, `crypto`, `gpu`, `kernel`, `loader`, `memory`, mais `tools/` (crate `xenon-tools` no workspace, 42 ferramentas em `tools/src/bin`).
- Extras na raiz: `extracted_xui/` (XUR/XUI extraídos), `Corona 16mb/` (dump de NAND), `_archive/` (scripts de análise antigos, fora do git: pesquisa de untile em `scratch-untile-2026-10-22/`), `_git_backup/` (o `.git` contaminado anterior), `xbox360_real_framebuffer.raw` (dump de referência do framebuffer real).

## 2. Estado atual (resumo honesto)

| Camada | Abordagem | Estado |
|---|---|---|
| CPU PPC | LLE, interpretador (`cpu/interpreter.rs` decodifica; semântica em `cpu/ops/{int,branch,loadstore,float,vmx,system,d3d_pack}.rs`; `cpu/altivec.rs`) | Estável. VMX128 D3D pack/unpack completos (`cpu/ops/d3d_pack.rs`). A thread principal roda até o frame 1175 (bloqueio de HLE, ver abaixo). |
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
- **Sobreposição da pilha com recursos:** `stack_top = vaddr_base + image_size - 0x10000 = 0x93018000` fica **dentro** do recurso `hubapp` (`0x92FF6780..0x93026A12`). A pilha da thread principal cresce para baixo sobre esse recurso (e `arcade` vai até `0x93027EF2`). A pilha precisa sair da imagem. **Resolvido (fechamento da Fase 2, item 1):** as pilhas têm região própria na janela virtual: principal `0x71000000..0x71400000`, threads dinâmicas a partir de `0x72000000` (256 KB cada, nunca reaproveitadas, máximo 768) e callbacks de interrupção em 16 slots rotativos a partir de `0x7E000000` (64 KB cada). Teste `stacks_stay_out_of_the_image_and_do_not_overlap`.

**Atualização 2 (2026-10-24, depois de `XexGetModuleHandle`/`XexGetModuleSection` em `src/kernel/modules.rs`):** os módulos (dash, `xboxkrnl.exe`, `xam.xex`) têm `X_LDR_DATA_TABLE_ENTRY` em `0x931F0000 + 0x70·i`, e o handle é o endereço dessa entrada. O locator sai correto (`section://931F0000,dashuisk#skin.xur`), e o carregamento da skin passa a falhar com `0x80004005` (E_FAIL; o `main` retorna esse código no frame 257). Nomes pedidos pela dash: `NULL`, `xam.xex` (×2), `xboxkrnl.exe`, `xbdm.xex` (este dá NOT_FOUND, como num console retail).

Próximo bloqueio, com evidência: **todas** as chamadas de `XexGetModuleSection` (frames 85, 104, 122, 127, 128 e 217) retornam `X_STATUS_NOT_FOUND`, porque o nome da seção chega como lixo. A dash separa o locator em `0x922192C0` (handle em hex até a `,`, por `0x92219180`) e converte o nome wide para narrow em `0x92219218` → `0x92192B60` (`WideCharToMultiByte`) → **`RtlUnicodeToMultiByteN` (xboxkrnl `0x0143`), que é um stub `=> 0`** (não escreve nada). O inverso `RtlMultiByteToUnicodeN` (`0x0133`) também é stub. Referência: Xenia `xboxkrnl_rtl.cc`.

**Atualização 3 (2026-10-24, depois de `RtlUnicodeToMultiByteN`/`RtlMultiByteToUnicodeN`):** `XexGetModuleSection` passa a achar as seções (retorna 0 nos frames 85, 162, 186, 301...), a skin carrega e **o `main` deixa de retornar**. A dash executa a inicialização real por cerca de 1100 frames (1500 frames levam ~13 s headless).

Próximo bloqueio: no **frame 1100** a thread principal para em `0x9221FA28` com `Unimplemented VMX128 vpkd3d128 (insn=0x1BE166D4)`. Em `cpu/ops/vmx.rs`, `vpkd3d128` não tem nenhuma variante implementada (só existe `vupkd3d128` tipos 0, 1, 3, 4, 5). Referência: Xenia `src/xenia/cpu/ppc/ppc_emit_altivec.cc` (`InstrEmit_vpkd3d128`) e o encoding VX128_4 em `ppc_instr.h`.

Observação: com a dash seguindo o caminho normal, o PM4 para no frame 28 (72 pacotes). Os 120 pacotes e os 24 `DRAW_INDX_2` da baseline vinham do **caminho de erro/shutdown** (frames 216..255), não da inicialização normal. A thread 0x3 continua girando em `RtlEnterCriticalSection`/`RtlLeaveCriticalSection` (`0x0125`/`0x0130`) e `0x00F4` (esperas não bloqueantes, Fase 3).

**Atualização 4 (2026-10-24, depois de `vpkd3d128` e `vupkd3d128` completos em `src/cpu/ops/d3d_pack.rs`):** todos os 7 tipos de pack/unpack e as máscaras pack 1/2/3 × shift 0..3. A instrução da dash (`0x1BE166D4`) é `vpkd3d128 v63, v12`, D3DCOLOR, VPACK_32, shift 3. Os tipos 3/5 do unpack passaram a usar a conversão half do Xenos (`xenos_half_to_float`: expoente 31 é normal, denormais zeram), em vez de IEEE.

A dash passa do frame 1100, troca o frontbuffer (`0x906AC000` → `0x90314000`) e envia mais PM4 (178 pacotes no total no frame 1175). Próximo bloqueio: **frame 1175**, `twi 6, r9, 0` em `0x921BEB30` (checagem de divisão por zero do compilador antes de `divwu r11, r10, r9`, com `r10 = [[0x92000EC0]]`). `r9 = [r1+0x7C]` é o campo `+0x1C` (`pixel_rate`) do `X_VIDEO_MODE` em `r1+0x60`, preenchido por `VdQueryVideoMode` (chamado em `0x921BE904`). **Nossa HLE de `VdQueryVideoMode` (`0x01CA`) usa layout errado:** grava 60 (inteiro) em `+0x10` (`is_hi_def`) e deixa `refresh_rate` (`+0x14`, float), `video_standard`, `pixel_rate` e `widescreen_flag` em 0. Layout de 48 bytes e valores do Xenia: `kernel.h` (`X_VIDEO_MODE`) e `xboxkrnl_video.cc` (`pixel_rate = 0x8A`, `refresh_rate` 60.0/50.0, `widescreen_flag` 1/3, `is_hi_def = width >= 0x500`).

**Atualização 5 (2026-10-24, depois de `VdQueryVideoMode` em `src/kernel/video.rs`):** `X_VIDEO_MODE` de 48 bytes como no Xenia (`pixel_rate = 0x8A`, `refresh_rate` 60.0 float, `is_hi_def`, `video_standard` NTSC-M, `widescreen_flag`), a partir de `VideoConfig` (padrão 1280×720 progressivo 16:9, 60 Hz). A divisão por zero em `0x921BEB30` sumiu.

Próximo bloqueio, ainda no **frame 1175**: `Unimplemented opcode 31 xo 567 (insn=0x7DAA4C6E)` em `0x921BE5A0`, ou seja `lfsux f13, r10, r9`. Faltam as quatro formas FP indexadas com update: `lfsux` (567), `lfdux` (631), `stfsux` (695), `stfdux` (759). As formas D com update (opcodes 49/51/53/55) e as indexadas sem update (535/599/663/727) existem em `cpu/ops/loadstore.rs`.

Também divergente do Xenia (não corrigido): `VdQueryVideoFlags` (`0x01C9`) devolve 1; pelo mesmo modo deveria devolver 3 (widescreen | largura ≥ 1280).

**Atualização 6 (2026-10-24, depois de `lfsux`/`lfdux`/`stfsux`/`stfdux` em `cpu/ops/loadstore.rs`):** **nenhum trap em 4000 frames.** A thread principal segue ativa (98,9 M passos), o frontbuffer volta a alternar e no frame 1176 a GPU recebe de uma vez 568 pacotes (total 746).

**Bloqueio atual (é o da Fase 2):** a thread principal gira em `0x921A40B0` (laço quente `0x921A40D0..0x921A40EC`), chamada a partir do laço de fence (`LR = 0x921B0000`, faixa `0x921AFFF8..0x921B0020`). Cadeia: `0x921A61C0` ← `0x92187048` ← `0x9218585C` ← `0x921871F0` (retorno de `0x92185790`, o laço principal da aplicação chamado pelo `main`). Ou seja, agora a espera de fence acontece no caminho normal, não no shutdown. Histograma PM4 até o frame 1400: `0x10`×5, `0x21`×3, `0x2B`×5, `0x36`×26, `0x3B`×8, `0x3C`×10, `0x3F`×13, **`0x45` (COND_WRITE)×256**, `0x46`×1, `0x48`×1, `0x54` (INTERRUPT)×1, `0x58`×12, `0x60`×3, `0x61`..`0x63`×2; Type-0: 396. Próximo passo: instrumentar a condição do fence (`0x90102000`?) com watchpoint e conferir `COND_WRITE`, `EVENT_WRITE_SHD`, `INTERRUPT` e a entrega de interrupções contra o `command_processor.cc` do Xenia.

**Diagnóstico do fence (2026-10-24, Fase 2; só instrumentação, CP não alterado):**

1. **O que a thread principal espera.** Laço em `0x921B0008..0x921B0020`: `while ((issued − target) < (issued − *fence_ptr)) 0x921A40B0()`, com device D3D `r31 = 0x40002400`, `fence_ptr = [dev+0x2B10] = 0x90102000`, `issued = [dev+0x2B1C] = 0x15`, **`target = 0xD`** (frame 1233).
2. **O que a GPU escreveu.** Watchpoint em `0x90102000`: `1 → 3 → 5 → 7 → 9 → 0xB`, cada um por `EVENT_WRITE_SHD [3, 0x10102002, n]` (endereço físico `0x10102000`, endian 2). O último (`0xB`) vem do lote do frame 1176.
3. **O CP não está travado: não chega mais trabalho.** A última escrita em `CP_RB_WPTR` é no frame 1176 (`WPTR = 49`, ring `0x10104000`, 8192 dwords), e o CP consumiu tudo (`RPTR = 49`). O fence 13 já foi emitido pela D3D (`issued = 0x15`), mas os comandos dele nunca foram submetidos. O WPTR é escrito por `0x921AFBC0`, chamada por `0x921B4DC8`/`0x921B4FDC` (rotinas de kick).
4. **O callback de interrupção da GPU nunca fez nada.** Callback da dash: `0x921AFCA0(r3 = source, r4 = device)`. `source == 1` (CP): chama o handler em `[[dev+0x2B14]+0x10]` e limpa o bit da CPU em `[[dev+0x2B14]]`. `source == 0` (VBlank): lê o MMIO `0x7FC86544` (registrador `0x1951`, "interrupt status") e, com o bit 0 ligado, chama `0x921A4BF0(device)`, que mexe no MMIO da GPU e faz `KeSetEvent`. O nosso frame loop chama com **`r3 = data` (`0x40002400`), `r4 = 1`, `r5 = frame`**: nenhum ramo executa.

Diferenças em relação ao Xenia (`451f9155`: `graphics_system.cc`, `kernel_state.cc` `EmulateCPInterruptDPC`, `pm4_command_processor_implement.h`):

| Ponto | Xenia | Nosso | Efeito |
|---|---|---|---|
| Argumentos do callback | `(source, interrupt_callback_data)`, como DPC na thread atual (process type title) | `(data, 1, frame)`, como thread agendada | callback inerte |
| VBlank | `MarkVblank`: `counter_++` e callback `(0, cpu 2)` em toda VBlank | callback a cada 2 frames | sem VBlank para a D3D |
| `INTERRUPT` (`0x54`) | callback `(1, n)` para cada bit de `cpu_mask` | só `pending_interrupts += 1` | interrupção de CP perdida |
| MMIO `0x1951` | leitura devolve 1 (VBlank); também `0x0F00`, `0x0F01`, `0x0F2D`, `0x1961` fixos | devolve o último valor escrito (0) | ramo de VBlank não roda |
| `counter_` | incrementa em VBlank e `XE_SWAP` | incrementa a cada `EVENT_WRITE_SHD` | valor errado com bit 31 do initiator |
| Endianness em `EVENT_WRITE_SHD`/`COND_WRITE`/`MEM_WRITE` | `GpuSwap` pelos 2 bits baixos do endereço | ignora (certo só no modo 2, o que a dash usa hoje) | latente |
| Janela de writeback em `EVENT_WRITE_SHD` | endereço > `0x1FFFFFFF` vai para `0x7F000000 + (addr − WRITEBACK_START)` | ausente | latente |
| `WAIT_REG_MEM` | espera de verdade | força a condição | ok enquanto o CP for síncrono |

Hipótese principal (não confirmada): a submissão do trabalho seguinte depende do processamento de VBlank/interrupção da D3D (`0x921A4BF0` e o handler de CP), que nunca roda. Próximo passo proposto: corrigir argumentos e cadência do callback, `INTERRUPT`, `0x1951` e `counter_` segundo o Xenia e verificar se o `CP_RB_WPTR` volta a andar e o fence passa de `0xB`.

**Atualização 7 (2026-10-24, interrupções corrigidas segundo o Xenia):** callback chamado como `(source, context)` com a CPU ativa no KPCR (`+0x10C`), em toda VBlank (`source 0`, CPU 2, com `counter_++`) e para cada bit de `cpu_mask` de `INTERRUPT` (`source 1`); `counter_` deixou de avançar em `EVENT_WRITE_SHD` (avança em VBlank e `XE_SWAP`); leituras MMIO fixas `0x0F00`, `0x0F01`, `0x0F2D`, **`0x1951` = 1**, `0x1961`. Verificado: o handler de VBlank da D3D `0x921A4BF0` agora roda em todo frame e o ramo de interrupção de CP rodou no frame 1130.

**O `CP_RB_WPTR` continua parado** (lotes agora nos frames 1128/1129, WPTR 49; depois nada). A hipótese do callback não era a causa. Evidência nova, que **corrige a anterior**:

- A rotina de submissão real é `0x921B0480` (monta pacotes `INDIRECT_BUFFER` `0xC0013F00`/`INDIRECT_BUFFER_PFD` `0xC0013700` no ring e escreve o WPTR em `0x921B06FC`). `0x921AFBC0` (que o HANDOFF citava) não é chamada. Chamadores: `0x921B107C`, `0x921B142C`, `0x921B481C`, `0x921B4B7C`, `0x921C8DBC`, `0x921C9514`, `0x921C9548`.
- Watchpoint em `issued` (`dev+0x2B1C`): os fences `0xD`, `0xF`, `0x11`, `0x13`, `0x15` são emitidos (em `0x921B11BC`, via `0x921B14D0`) **nas mesmas rotinas de kick** dos frames 1128/1129, cujas 10 submissões levam o WPTR até 49; o CP consome tudo (`RPTR = 49`). Ou seja, o trabalho do fence 13 **foi submetido**, mas o CP só executou `EVENT_WRITE_SHD` até `0xB`.
- A espera (`0x921AFF68`) só faz kick quando `target == issued` e `[dev+0x3520] == 0`; com `target 0xD < issued 0x15` ela só espera (correto do ponto de vista da D3D).

Próximo passo: auditar o processamento dos indirect buffers dessas submissões (tamanhos `0xB`, `0x43`, `0xB`, `0x923`, `0xB`, `0x14` no frame 1129): despejar cada IB e listar onde estão os `EVENT_WRITE_SHD` com os valores `0xD..0x15`. Diferenças conhecidas do decodificador em relação ao Xenia: opcode com máscara `0xFF` (Xenia `0x7F`), bit de predicação (bit 0) e flag de registrador único do Type-0 (bit 15) ignorados; em erro de decodificação dentro de um IB o nosso CP pula 1 dword.

**Auditoria dos indirect buffers (2026-10-24, só instrumentação; CP não alterado):**

1. **O CP executou tudo o que recebeu.** Nos frames 1128/1129 o ring recebeu 8 IBs; os 4 `EVENT_WRITE_SHD` de fence estão nos IBs de conteúdo e foram executados:

   | Frame | IB | Tamanho | Conteúdo | Fence |
   |---|---|---|---|---|
   | 1128 | `0x1011C120` | 6 | 3 × T0 | — |
   | 1128 | `0x1010C120` | 0x198 | 101 pacotes (2 draws) | 5 |
   | 1129 | `0x1011C160` / `0x1011C1A0` / `0x1011C1E0` | 0xB | WAIT_REG_MEM + 2 × T0 (coerência) | — |
   | 1129 | `0x1010C780` | 0x43 | 21 (`INTERRUPT`, 5 × WAIT_REG_MEM) | 7 |
   | 1129 | `0x1010C8A0` | 0x923 | 524 (256 × COND_WRITE) | 9 |
   | 1129 | `0x1010ED40` | 0x14 | 8 | 0xB |

   Os fences `0xD..0x15` **não estão em nenhum IB submetido**. (O watch anterior de `issued` misturava execuções; o que vale é este.)
2. **Fluxo de kick da D3D.** Cada kick: IB de coerência por `0x921B14C8`; fence gravado no segmento (`0x921B0960`, retorno em `0x921B11BC`); conteúdo por `0x921B0EB0` → submissão `0x921B0480` (escreve WPTR em `0x921B06FC`). `0x921B0EB0` tem 3 caminhos: `[dev+0x3520] != 0` → lista adiada (`dev+0x35C0`); `[dev+0x2B90] != 0` → caminho assíncrono com spinlock (`0x921B0188`/`0x921C9BB8`) que incrementa `[dev+0x2B90]`; senão submissão direta.
3. **O segmento do fence 0xD foi para o caminho assíncrono.** Kicks dos fences 5..0xB: `[dev+0x2B90] = 0` (diretos). Kick do fence 0xD (via `0x921A6188`, caminho de present): **`[dev+0x2B90] = 1`**, sem submissão. A fila da thread 0x2 (`ctx 0x400051A4`, `[ctx+0x54] = [ctx+0x58] = 0`) está vazia; o segmento não foi para lá.
4. **O handler de interrupção de CP da D3D cai num trap.** O `INTERRUPT` do IB `0x1010C780` é entregue (frame 1130) e o callback chama `[[dev+0x2B14]+0x10] = 0x921A4D20` com `r3 = [0x90103014] = 0x100`. `0x921A4D20` (temporização do present) chama **`KeQueryPerformanceFrequency`** e faz `r11 = freq / r28` (`r28 = [dev+0x55CC]` ou 60), `twi 6, r28, 0`, `twi 6, r11, 0`. **Nosso `KeQueryPerformanceFrequency` (`0x0083`) trata `r3` como ponteiro, grava 50 000 000 (u64) nesse endereço e devolve 1.** No Xenia (`xboxkrnl_threading.cc`) a função **não tem argumento e devolve a frequência** (`Clock::guest_tick_frequency()`, 50 MHz). Resultado: `freq = 1`, `r11 = 0` em `0x921A4DB4` → trap (hook confirmou `r11 = 0`; `0x921A4DB8` nunca executa). A thread do callback não é contínua e é desativada em silêncio. Efeito colateral: cada chamada grava 8 bytes no endereço que estiver em `r3` (aqui `0x100`).

**Diagnóstico:** o CP não perde nada; o trabalho após o fence 0xB fica no caminho assíncrono da D3D, cuja continuação depende do handler de interrupção de CP, e esse handler cai num trap por causa da assinatura errada de `KeQueryPerformanceFrequency`. Elo ainda não verificado: que o handler `0x921A4D20` (ou o que ele dispara) seja quem submete o segmento pendente e zera `[dev+0x2B90]`.

**Atualização 8 (2026-10-24, `KeQueryPerformanceFrequency` corrigido: sem argumento, devolve 50 MHz):** o handler de interrupção de CP `0x921A4D20` agora completa (passa da divisão com `r11 = 0xCB735`, chama o callback registrado `0x92182CD0`, volta a `0x921AFCD4` e limpa o bit da CPU em `0x921AFD04`). **Não destravou**: WPTR continua em 49 e o fence em `0xB`. O elo "o handler de CP submete o segmento pendente" era **falso**.

Achados novos:

- **`[dev+0x2B90]` não é um contador de pendências.** No código da D3D ele só é incrementado (`0x921B0F8C`, `0x921B1038`, `0x921B1064`, `0x921B1398`) e só é zerado na recuperação de GPU travada (`0x921B5FA0`, que também reescreve o fence como `issued − 2`). Com ele ≠ 0, os kicks seguintes vão para `0x921B0188`, que encadeia o IB num outro buffer (`r25`, provável system command buffer) em vez do ring. A submissão direta em `0x921B1064` faz `[dev+0x2B90] += r26`; o kick de present do fence `0xB` é o que o liga.
- **`VdSwap` (xboxkrnl `0x025B`) está muito longe do Xenia.** A dash o chama uma vez (frame 1128, de `0x921A5E60`) com `buffer_ptr = 0x9010C640`, que fica **dentro do IB `0x1010C120`** (o do fence 5), e esse trecho está zerado. No Xenia (`xboxkrnl_video.cc`), `VdSwap(buffer_ptr, fetch_ptr, unk2, unk3 = buffer de VdGetSystemCommandBuffer, unk4, frontbuffer_ptr, texture_format_ptr, color_space_ptr, width, height)` escreve nos 64 dwords de `buffer_ptr`: Type-0 de `SHADER_CONSTANT_FETCH_00_0` (6 valores, com o endereço do frontbuffer convertido para físico), `XE_SWAP` (`0x64`, `kSwapSignature`, endereço físico, largura, altura) e NOPs Type-2. O nosso só conta o swap e guarda `args[0]` como frontbuffer; nosso CP lê os zeros como 32 escritas Type-0 no registrador 0 e o swap nunca acontece do lado da GPU.
- **O frame loop chama `VdSwap` sozinho a cada frame** (`app/frame_loop.rs`, com `[frontbuffer, frame]`), fora da assinatura real.
- **`mftb` é por thread** (`step_count × 50` em `cpu/ops/system.rs`): cada thread tem seu próprio relógio e as threads de callback começam em 0. O handler de CP compara `mftb` do callback com `[dev+0x423C]`, gravado pelo handler de VBlank em outra thread.

Próximo passo proposto: `VdSwap` segundo o Xenia (escrever fetch + `XE_SWAP` + NOPs em `buffer_ptr`), CP executando `XE_SWAP` (swap + `counter_`), e remover a chamada sintética de `VdSwap` do frame loop; depois verificar se a D3D sai do modo `[dev+0x2B90]` e volta a escrever o WPTR. Também pendente: `mftb` global (timebase única).

**Atualização 9 (2026-10-24, `VdSwap` segundo o Xenia):** `VdSwap` (`kernel/video.rs`) escreve nos 64 dwords de `buffer_ptr` o Type-0 do fetch do frontbuffer (`SHADER_CONSTANT_FETCH_00_0`, endereço convertido para físico), `XE_SWAP` (`0xC0036400`, `'SWAP'` = `0x53574150`, endereço físico, largura, altura) e NOPs; o CP executa `XE_SWAP` (registra o swap e `counter_++`); o frame loop não chama mais `VdSwap` sozinho e o frontbuffer apresentado é o do último `VdSwap` real. Corrigido junto: **argumentos de pilha** (`StackArgs`) passam a ler slots de 8 bytes em `r1 + 0x50 + 8·i` (valor de 32 bits em `r1 + 0x54 + 8·i`, como o `get_arg_32` do Xenia), confirmado pela pilha na chamada da dash (`r1+0x50 = 00000000 93017A80 00000000 93017A84` → ponteiros de largura/altura). Antes, os ponteiros de largura/altura liam 0 e o `VdSwap` não escrevia nada.

Verificado: o CP agora executa os pacotes de swap (total de pacotes 178 → 200, troca de 32 Type-0 de zeros por 54 pacotes). **Não destravou**: WPTR continua em 49 e o fence em `0xB`; a D3D continua no modo `[dev+0x2B90] = 1` depois do present do frame 1128.

Situação: já corrigidos e verificados, sem efeito no travamento: argumentos/cadência do callback, `INTERRUPT`, MMIO fixos, `counter_`, `KeQueryPerformanceFrequency`, `VdSwap`/`XE_SWAP`. O que falta entender é **o que tira a D3D do modo `[dev+0x2B90] != 0`** ou o que submete o buffer onde `0x921B0188` encadeia os IBs (`r25` no caminho de `0x921B0F1C`). Pontos de leitura de `0x2B90` ainda não examinados: `0x921B127C`, `0x921B12D8`, `0x921B17D8`. Também em aberto: `mftb` por thread e `VdQueryVideoFlags`.

**Diagnóstico do modo `[dev+0x2B90]` (2026-10-24, só instrumentação; código não alterado):**

1. **O modo é uma cadeia assíncrona de segmentos.** Em `0x921B1344..0x921B13DC`, com `[dev+0x2B90] == 0` e `r21 != 0`, a D3D abre uma cadeia: inicializa a lista em `dev+0x35C0` (`0x400059C0`), aloca o primeiro nó (`0x921B0D20`), copia o segmento (`0x921B0E30`), fecha o nó (`0x921B0140`), faz **`[dev+0x2B90]++`** e chama **`KeInsertQueueDpc(dev + 0x2D58 + 108·[dev+0x2CC8], gpu(head), 0)`**. Com o modo ligado, os kicks seguintes vão para `0x921B0188`, que emenda o novo nó no fim da cadeia (escreve o salto no slot do nó anterior e faz flush). Os nós têm `[?, 0x81000000|tamanho, endereço GPU, 0xC0000000]`. Observado no frame 1129: cabeça `0x9011C280`, nós `0x9011C32C` (segmento `0x1011C300`, tamanho 0xB) e `0x9010EDD0` (segmento `0x1010EDA0`, tamanho 0xC).
2. **Nenhum IB `0x1011C280` (a cabeça) é submetido ao ring**; por isso o trabalho do fence 0xD em diante nunca chega ao CP. `0x921B17D8` espera em laço `[dev+0x2B90] == 0`; no código da D3D não há decremento direto (só a recuperação de travamento `0x921B5FA0` zera).
3. **Quem deveria encaminhar a cadeia é o DPC.** `KeInitializeDpc` é chamado uma vez (frame 3, de `0x921C9DDC`) com `dpc = 0x400051C4`, **rotina `0x921C89B0`**, **contexto `0x400051A4`** (o contexto da thread trabalhadora 0x2). Logo depois de `0x921C89B0` fica `0x921C89E8`, que põe o item na fila do contexto da CPU (`dev + 0x2D38 + 108·cpu`, índices em `+0x54`/`+0x58`, itens a partir de `+0x5C`) e faz `KeSetEvent(ctx+0x3C)`; a thread 0x2 acorda, vê a fila não vazia e processa em `0x921C9570`.
4. **Causa raiz: `KeInsertQueueDpc` (xboxkrnl `0x007B`) não tem HLE** ("Unhandled xboxkrnl.exe ordinal 0x007B"): devolve 0 e a rotina do DPC nunca roda. No Xenia (`xboxkrnl_threading.cc`) ele grava `arg1`/`arg2` no DPC, insere na lista de DPC e **executa a rotina na hora, na thread que chamou**, com `(dpc, context, arg1, arg2)` sob impersonação de DPC, e devolve 1.
5. **Também errado: `KeInitializeDpc` (`0x006F`)** grava a rotina em `+8` e o contexto em `+12`. O `XDPC` do Xenia (`xthread.h`) é `type` (`+0x0`, = 19), `selected_cpu` (`+0x2`), `desired_cpu` (`+0x3`), `list_entry` (`+0x4`), **`routine` (`+0xC`)**, **`context` (`+0x10`)**, `arg1` (`+0x14`), `arg2` (`+0x18`). `KeRemoveQueueDpc` (`0x008E`) também não foi visto implementado.

Ainda não verificado: que a thread 0x2, ao processar o item, submeta a cabeça da cadeia ao ring e que o modo `[dev+0x2B90]` volte a 0 (o decremento pode estar no processamento do item ou num callback de conclusão). Próximo passo proposto: `KeInitializeDpc` com o layout do Xenia e `KeInsertQueueDpc` executando a rotina na thread chamadora (com KPCR `dpc_active` como no Xenia), e então medir WPTR/fence.

**Atualização 10 (2026-10-24): travamento da GPU resolvido (critério do M1 atingido).** Implementados segundo o Xenia: `KeInitializeDpc` (layout `XDPC`), `KeInsertQueueDpc` (grava `arg1`/`arg2`, devolve 1 e a rotina roda **inline na thread chamadora**, com impersonação de DPC: `KPCR+0x18` IRQL 2 e `KPCR+0x150` `dpc_active = 1`, restaurados na volta; DPC aninhado não reimpersona) e `KeRemoveQueueDpc`. Mecanismo: depois do syscall, o frame loop chama `routine(dpc, context, arg1, arg2)` com LR num trampolim (`0x80001010`, `sc; blr`, pseudo-ordinal `0xFFFE`); ao chegar nele, restaura o estado e devolve 1 ao chamador.

Resultado (3000 frames): **415 escritas de `CP_RB_WPTR`** (antes 3), fence de `0xB` até **`0x62D`**, 75 660 pacotes no CP; a thread principal sai do laço de fence. Em 5000 frames (~43 s headless) o ring dá a volta (WPTR 7775 → 4731) e a dash fica em regime: 177 711 pacotes; `XE_SWAP` ×1044 (um por present), `INTERRUPT` ×1044, `DRAW_INDX_2` ×2112, `EVENT_WRITE_SHD` ×6342, `WAIT_REG_MEM` ×6301, `COND_WRITE` ×8704, `INDIRECT_BUFFER` ×4251, Type-0 ×73 677. Nenhum trap.

Pendências que agora ficam visíveis: os draws não são rasterizados (Fase 4; desde o item 7 do fechamento da Fase 2 o log `[GPU CP]` mostra `real_draws` e `swaps` — o caminho falso antigo foi removido: estado de constantes/shaders/viewport/texturas em `XenosGpu`, `gpu/shader.rs`, as triangulações de `gpu/rasterizer.rs` (fica só `rasterize_rounded_card` para o `display.rs`), os construtores de pacotes inventados, a demo `emit_synthetic_stream` do caminho pré-Metro, o fallback que adivinhava o WPTR pelo conteúdo do ring e os binários `test_shader_pipeline`, `test_textured_quad` e `test_dash_boot_and_gpu`; antes, escritas reais da dash em 0x0A00..0x0A5F (FB_START, WRITEBACK_START...) disparavam a decodificação de "descritor de textura" inventada, 1577 vezes em 3000 frames); `mftb` por thread (**resolvido**: um TB global de 50 MHz no `MemoryBus`, `cpu/timebase.rs`, 833 333 ticks por frame distribuídos pelas instruções de todas as threads, monotônico; a dash 17559 tem um único `mftb`, em `0x92617738`, que não executa nos primeiros 3000 frames); `VdQueryVideoFlags` (**resolvido** no item 6: derivado do modo de vídeo como no Xenia, 3 para 1280x720 16:9; `XamLoaderTerminateTitle` agora encerra a emulação em vez de retornar; nenhum dos dois é chamado nos primeiros 3000 frames); esperas do kernel não bloqueiam (Fase 3; a thread 0x3 continua girando em critical sections).

**Atualização 11 (2026-10-25, Fase 4.0, diagnóstico de draws):** `XENON_DRAW_LOG=1` imprime cada draw com o initiator decodificado, os registradores de render target/viewport/shader/resolve, os demais registradores de contexto não nulos, a fetch constant 0 e os shaders ativos (cada blob uma vez); `XENON_MMIO_WRITE_LOG=1` imprime cada escrita da CPU na janela de registradores. O CP agora guarda os shaders de `IM_LOAD`/`IM_LOAD_IMMEDIATE` (`vertex_shader`/`pixel_shader`) e conta palavras de IB que não decodificam (`ib_decode_errors`, 0 na dash). Resultados (1400 frames):
- A CPU só escreve `CP_RB_WPTR` via MMIO (as "escritas extras" do contador eram várias por frame no WPTR).
- Draws #1..#24 (frame 28): point list de 1 vértice, `RB_MODECONTROL=4`, `RB_COLOR_MASK=0`, superfície/viewport zerados, VS `8BB7E189D92E3DEF` (24 dwords), PS `438C2AF84C78A133` (9 dwords): não escrevem cor.
- Regime (2 draws por frame a partir do frame 1193): RECTLIST de 3 vértices auto-indexados. (a) predicado, `RB_MODECONTROL=5` (depth), `RB_SURFACE_INFO=0A020280`, `RB_DEPTH_INFO=00000260`, `RB_DEPTHCONTROL=00008777`, VS `539EA9E08AA83F0C` (27 dwords); (b) `RB_MODECONTROL=6` (copy), `RB_SURFACE_INFO=14010500` (pitch 1280), `RB_COLOR_INFO=00000260`, scissor 1280x720, VS `A4AE7C2B7C1818C4` (15 dwords), mesmo PS. Fetch constant 0 aponta para os vértices (`0x1011C09C`, `0x101AD380`...).
- **Todos os `RB_COPY_*` (CONTROL, DEST_BASE, DEST_PITCH, DEST_INFO, FUNC) e `RB_*_CLEAR` estão zerados no resolve**, e nenhum outro registrador de contexto guarda um destino. Ou seja: o resolve que a dash emite nesse estágio não copia para o frontbuffer do `XE_SWAP` (`0x106AC000`). A UI ainda não é desenhada.

**Atualização 12 (2026-10-25, Fase 3.0, auditoria de ordinais):** 42 comentários `0xNNNN: Nome` do HLE não batiam com as tabelas geradas do Xenia, e vários braços implementavam outra função. Exemplos: 0x0140/0x0141 (`RtlTimeToTimeFields`/`RtlTryEnterCriticalSection`) zeravam 8 bytes do argumento como "free string" (corrompendo a critical section); `KeDelayExecutionThread` é 0x005A, mas o braço e a lista de fim de fatia usavam 0x0057 (`KeContextToKframes`) e 0x002B, nenhum importado, então as 375 chamadas caíam em "Unhandled" e não cediam a fatia; 0x0646 (`XamPackageManagerGetExperienceMode`) e 0x0220 (`XamUserFlushLogonQueue`) escreviam através de argumentos que não são ponteiros; `XamUserGetSigninInfo` é 0x0227 (0x0221 é `XamUserIsOnlineEnabled`). Corrigido pelos fontes do Xenia: `KeDelayExecutionThread`, `ExRegisterTitleTerminateNotification`, `ExSetXConfigSetting`, `HalGetCurrentAVPack` (8), `HalReturnToFirmware` (encerra), `RtlFreeAnsiString`/`RtlFreeUnicodeString`, `RtlTimeToTimeFields`, `RtlTryEnterCriticalSection` (TRUE, coerente com `RtlEnterCriticalSection` sem estado até a Fase 3.3), `VdGetCurrentDisplayGamma` (tipo 2, potência 2.22222233), `XamGetSystemVersion` (0), `XamSetDashContext`, `XamUserCheckPrivilege`, `XamUserIsOnlineEnabled`, `XamUserGetSigninInfo`, `XamUserGetName`; braços de funções que o Xenia não implementa ou que dependem de XConfig (idioma/locale) foram removidos e caem em "Unhandled" (mesmo retorno 0, agora visível no log). Teste `hle_comments_name_the_real_exports` impede rótulos errados. `XENON_LIST_IMPORTS=1` lista as 806 funções importadas.

Resultado (3000 frames): 614 escritas de `CP_RB_WPTR` (antes 411), fence até `0x9D5` (antes `0x621`), 611 `XE_SWAP` (antes 374); a thread principal gira menos (59,9 M de passos, antes 71,9 M). Os draws continuam só limpeza de profundidade + cópia (611 de cada); nenhum draw de UI. Não isolei qual correção dá o ganho (candidatas: `KeDelayExecutionThread` cedendo a fatia, `VdGetCurrentDisplayGamma`, `HalGetCurrentAVPack`).

**Atualização 13 (2026-10-25, Fase 3.5, CP bloqueante):** `WAIT_REG_MEM` não satisfeito deixa de forçar a condição: o CP para no pacote e o executa de novo na próxima vez que roda (uma vez por frame, depois das threads da CPU), como o laço de espera da thread do CP no Xenia. Os indirect buffers viraram uma pilha retomável (`ib_stack`, endereço/tamanho/posição; as palavras são lidas da memória à medida que executam). Junto, o writeback de scratch e o `COHER_STATUS_HOST` do Xenia (`HandleSpecialRegisterWrite`, `MakeCoherent`), que antes regrediam sem o bloqueio: a dash grava o endereço do handler (`0x921A4D20`) no scratch 4, emite `INTERRUPT` e espera a CPU zerar o scratch 0; agora o callback lê o handler, não o `0x0BADF00D` seguinte. Resultado (3000 frames): idêntico à Fase 3.0 (614 / `0x9D5` / 611 swaps / 111 462 pacotes), com 611 bloqueios, todos nessa espera (`mem 0x10103002 == 0`), e nenhum `WAIT_REG_MEM` forçado.

**Atualização 14 (2026-10-25, Fase 3.1, objetos e handles):** tabela de objetos única como a do Xenia (`kernel/objects.rs`): handles `0xF8000000 + 4·slot` (sockets sem a base), referências por handle, `NtCurrentThread()` (-2) traduzido para a thread que chama (`KernelDispatcher::current_thread`, definido pelo frame loop a cada syscall). O estado dos objetos de dispatcher fica no `X_DISPATCH_HEADER` na memória guest (tipo em +0, `signal_state` em +4): `NtCreateEvent`/`NtCreateSemaphore` criam o objeto nativo (`X_OBJECT_HEADER` + corpo, handle guardado na wait list com a assinatura `'XEN '`), `Ke*` operam no header do guest. Threads têm `X_KTHREAD` (0xAB0 bytes, campos de `XThread::InitializeGuestObject` que não dependem do KPROCESS) apontado por `KPCR+0x100`; a thread principal é o primeiro objeto (TID 1, handle `0xF8000000`), por isso **as threads dinâmicas agora começam em TID 2** (a antiga "Dynamic Thread 0x3" que gira em `NtRemoveIoCompletion` é a 0x4). Corrigidos junto: `NtCreateEvent` lia tipo/estado nos argumentos 3/4 (são 2/3); `ObReferenceObjectByHandle` escrevia `0x80010000` no 4º argumento (na dash, um endereço de código, `0x92198A80`) em vez de devolver o objeto no 3º; `ObLookupThreadByThreadId`, `ObOpenObjectByPointer`, `ObReferenceObject`, `ObDereferenceObject`, `NtDuplicateObject`, `NtCreateMutant`, `NtCreateTimer` seguem o Xenia; `KeSetEvent`/`KeResetEvent` devolvem 1 como `XEvent::Set/Reset`; `KeInitializeSemaphore`/`KeReleaseSemaphore`/`NtReleaseSemaphore` usam limite e contagem no header; `KeQuerySystemTime` deriva do timebase em vez do contador de handles. Não feito: nomes de objeto (`OBJECT_ATTRIBUTES`), checagem de tipo em `ObReferenceObjectByHandle` (as variáveis `Ex*ObjectType` não são exportadas ao guest; a dash passa o registro de import cru, `0x0001001B`). Resultado (3000 frames): igual à Fase 3.5 (614 / `0x9D5` / 611 swaps).

**Atualização 15 (2026-10-25, Fase 3.2, escalonador):** uma thread que espera fica estacionada pelo frame loop com um `WaitRequest` (`kernel/scheduler.rs`: objetos (headers de dispatcher), WaitAny/WaitAll, prazo no timebase global, status de timeout) e não executa instruções até a espera ser satisfeita (objetos adquiridos: evento de sincronização reseta, semáforo decrementa; `STATUS_WAIT_0 + i`) ou o prazo vencer. O kernel pede a espera com `pending_wait` (como o DPC); threads estacionadas não entram na conta de fatias do timebase; aviso `[SCHED]` quando toda thread espera sem timeout. `KeDelayExecutionThread` dorme de verdade (intervalo negativo = relativo em 100 ns, positivo = absoluto convertido pelo relógio de `KeQuerySystemTime`, onde o Xenia ainda tem um TODO; 0 = só cede a fatia). Resultado (3000 frames): igual à Fase 3.1; as 112 chamadas de `KeDelayExecutionThread` da dash (thread principal) usam intervalo 0, então só cedem a fatia. As esperas por objetos passam a usar o mecanismo na Fase 3.3.

**Atualização 16 (2026-10-26, Fase 3.3, esperas bloqueantes):** `KeWaitForSingleObject`/`KeWaitForMultipleObjects`/`NtWaitForSingleObjectEx`/`NtWaitForMultipleObjectsEx`/`NtSignalAndWaitForSingleObjectEx` estacionam a thread no escalonador (completam na hora quando podem; timeout nulo = sem prazo, 0 = só consulta). Eventos auto/manual, semáforos, mutantes (dono = KTHREAD de quem chama, recursão; `NtCreateMutant`/`NtCreateTimer` ganham header no guest, onde o Xenia os mantém só no host) e critical sections como no Xenia (`lock_count`/`recursion_count`/`owning_thread`; disputa = espera no evento da seção, que é entregue a quem acorda; `RtlTryEnterCriticalSection` real). O frame loop passa a thread atual em `current_kthread` (`KPCR+0x100`) e as esperas não encerram mais a fatia sozinhas. Também: variáveis de vídeo do kernel exportadas ao guest como no `RegisterVideoExports` do Xenia (`VdGlobalDevice`, `VdGlobalXamDevice`, `VdGpuClockInMHz`, `VdHSIOCalibrationLock` — a thread principal entrava nessa critical section com o registro de import cru `0x000101C1`); listeners de notificação reais (`kernel/notify.rs`: fila + evento manual; `XamNotifyCreateListenerInternal` 0x0291 não existia e devolvia handle 0, e a thread 0x13 girava esperando nele); `XamGetLanguage`/`XamGetLocale`/`XamGetOnlineLanguageAndCountry` pelos padrões do XConfig do Xenia (inglês, EUA, locale 36, `kernel/xconfig.rs`); só o usuário 0 está logado e `XamUserGetSigninState` devolve 0 para índice ≥ 4. Resultado (3000 frames): GPU igual à 3.2 (611 swaps, fence `0x9D5`); 18 threads dormem de verdade (0x6..0x9: 121..614 passos, antes ~377 mil cada; 0x13: 72); continuam girando a principal (59,9 M) e a 0x4 em `NtRemoveIoCompletion` (68,3 M, Fase 3.4).

**Atualização 17 (2026-10-26, Fase 3.4, timers e IO completion):** timers do kernel no timebase global (`kernel/timers.rs`): `KeInitializeTimerEx` (Xenia), `KeSetTimer`/`KeSetTimerEx`/`KeCancelTimer` (semântica NT; o Xenia não os implementa) e `NtSetTimerEx`/`NtCancelTimer`; ao vencer, o header é sinalizado (quem espera acorda pelo escalonador), o timer periódico é rearmado e o DPC roda numa thread curta em modo DPC (`timer_dpc_thread`, `routine(dpc, context, hora baixa, alta)`); a rotina APC de `NtSetTimerEx` não é entregue (APCs não emulados). Portas de IO completion com fila (`NtCreateIoCompletion` cria um header contador; `NtSetIoCompletion` enfileira; `NtRemoveIoCompletion` espera no escalonador e, ao acordar, o kernel escreve key/apc/status/bytes — `KernelDispatcher::complete_wait`); o timeout segue o intervalo com sinal (o Xenia trunca para 32 bits). Resultado (3000 frames): GPU igual à 3.3; a thread 0x4, que girava em `NtRemoveIoCompletion` (68,3 M de passos), dorme na porta sem prazo (887 passos); 19 threads estacionadas; resta girando a principal (59,9 M). A dash não arma timers nessa janela.

**Atualização 18 (2026-10-26, Fase 3.6, ciclo de vida e exports):** `ExTerminateThread` (e o retorno da função de entrada, via `0x80001000`) encerra a thread de vez como `XThread::Exit`: KTHREAD com `terminated = 1`, `exit_status` e header sinalizado (quem espera na thread acorda); trap também só para a thread; o código e as flags de reinício (`XENON_NO_RESTART`, `XENON_RESTART_MAIN`) saíram. Threads criadas com `CREATE_SUSPENDED` só rodam depois de `NtResumeThread` (contagem em `KTHREAD+0xBC`; `NtSuspendThread`/`KeResumeThread` também). Exports adicionados pelo Xenia: `XamIsNuiAutomationEnabled` (1224 chamadas), `ObReferenceObjectByName` (não acha: nomes de objeto não são registrados ainda), `NtResumeThread`/`NtSuspendThread`, `XamAvatarLoadAnimation` (overlapped completado na hora), `XamAvatarInitialize` (~0), `XamGetLanguageTypeface`, `XamBuildSharedSystemResourceLocator`, `XGetAudioFlags`, `XamFeatureEnabled`, `XamUserIsUnsafeProgrammingAllowed`, `XamLoaderRegisterLaunchRequestCallback`, `XamContentRegisterChangeCallback`, `NetDll_inet_addr`, `XamLogLocalizationEtx`; `KeQueryInterruptTime` pela semântica NT. Continuam sem implementação: `IoSynchronousDeviceIoControlRequest` (72 chamadas), `XamPackageManagerGetExperienceMode` e `XamUserFlushLogonQueue` (não existem no Xenia), `XamGetLanguageTypefacePatch`/`XamGetLanguageLocaleFallbackString` (dependem de tabelas do XAM) e `XMsgStartIORequestEx` (mensagens entre apps, Fase 5). Resultado (3000 frames): primeira submissão no frame 1164 (antes 1159; a dash segue outro caminho com os novos retornos e passa a chamar `XMsgStartIORequestEx`), 610 swaps, fence `0x9CF`; nenhuma thread terminou nem sofreu trap.

**Atualização 19 (2026-10-26, endereço efetivo de 32 bits):** a investigação das notificações de partida levou a um bug do bus. A dash embute Lua 5.1 (`common.lua` em bytecode); `0x927BEB50` cria o `lua_State` "Dash" (`0x928F4B20`) e guarda o handle em `[0x92A77F38+0x164]` = `0x92A7809C`. Abrir as bibliotecas (`luaL_register` -> `luaL_findtable(REGISTRY, "_LOADED")` -> `lua_settable(L, -4)`) falhava com `LUA_ERRRUN`: `index2adr` (`0x928DF0F0`) soma o deslocamento negativo estendido com zero (`rlwinm`) e obtém `0x1_42DFB660`; o bus (`window_addr`) recusava endereços com os 32 bits altos `!= 0/0xFFFFFFFF` e caía no caminho físico (`& 0x1FFFFFFF`, outra memória). Agora o endereço efetivo é truncado para 32 bits como em `ComputeMemoryAddress` do Xenia (`x64_seq_memory.cc`). Com o Lua de pé apareceu `KeGetImagePageTableEntry` (0x02A4) sem implementação -> `KeBugCheck(0xF7)` em `0x928EF490`; implementado pelo Xenia (`xboxkrnl_memory.cc`: índice da página nos heaps XEX, `0x40000001` para páginas de 4 KB). `XMsgInProcessCall` para o XLiveBase (app `0xFC`) segue `XLiveBaseApp::DispatchMessageSync` (`0x58003` -> `X_ONLINE_S_LOGON_DISCONNECTED`, que a dash compara em `0x92918A74`); os outros apps continuam devolvendo 0. Resultado (3000 frames): ver BASELINE; o objeto "Dash" é criado no frame 275.

**Atualização 20 (2026-10-26, `cmpldi` e formato MD):** com a Atualização 19 o objeto Lua "Dash" era criado, mas `0x927BEB50` ainda falhava: o caminho `section://%X,dashlua` saía `section://0,dashlua` porque o `swprintf` da dash (laço de dígitos em `0x92824D70..0x92824DC4`) parava no primeiro dígito. Causa: `cmpldi` (opcode 10, L = 1) comparava só os 32 bits altos com 0 e ignorava o imediato. Junto, o campo MB/ME de 6 bits dos formatos MD/MDS (`rldicl`, `rldicr`, `rldic`, `rldimi`, `rldcl`, `rldcr`) era lido cru; o Xenia usa `mb = (MB5 << 5) | MB` (`ppc_emit_alu.cc`), então `clrldi r,r,32` não zerava os bits altos. No caminho de falha a dash soltava a referência (`0x927BEC18`) mas deixava o handle em `0x92A7809C`; a tarefa Lua era destruída no frame 1152 e a notificação de sign-in usava o objeto liberado (salto para `pc = 0` no frame 1233). Corrigidos os dois, o "Dash" carrega no frame 703 e não é destruído. **Investigação abaixo resolvida:** com notificações ligadas a dash roda 3000 frames sem trap. Avaliação de ligá-las por padrão: o Xenia sempre as entrega; o resultado de GPU fica praticamente igual (2125 contra 2132 draws), e elas exercitam caminhos novos (thread 0x15, armazenamento/conteúdo) com quatro exports ainda sem implementação. Observação: `diag::flag` só testa presença, então `XENON_STARTUP_NOTIFICATIONS=0` também liga.

**Atualização 21 (2026-10-26, notificações de partida ligadas):** as notificações de partida do Xenia agora são entregues por padrão; `XENON_STARTUP_NOTIFICATIONS=0` desliga. `diag::flag` passou a tratar `0`/`false`/`off`/`no` (qualquer caixa) como desligado, em vez de só testar a presença da variável (vale para todas as flags). Resultado (3000 frames): padrão = frame 1622, 2125 draws, 303 swaps; com `=0` = frame 1621, 2132 draws, 304 swaps; sem trap nem `KeBugCheck`.

**Atualização 22 (2026-10-26, Fase 4.2, decoder de ucode):** `gpu/ucode.rs` porta os layouts do `ucode.h` do Xenia (controle de fluxo em pares de 3 dwords, fetch de vértice/textura, ALU vetor+escalar, registradores de export, tabelas de opcodes do `ucode.cc`); `gpu/ucode_disasm.rs` porta `Shader::AnalyzeUcode` e os `Parse*`/`Disassemble` do Xenia e gera o mesmo texto da disassembly do Xenia, além de uma lista de problemas (opcode de fetch desconhecido, export sem destino para o tipo de shader, exec fora do shader, nenhum exec final). Diagnóstico: `XENON_SHADER_DISASM=1` imprime cada shader distinto carregado por `IM_LOAD`/`IM_LOAD_IMMEDIATE` (`gpu/shader_log.rs`); `XENON_SHADER_DUMP=<dir>` grava os dwords em `<dir>/{vs,ps}_<hash>.bin` (dados do guest: diretório fora do repositório, ex. `..\_shader_dump`); `XENON_SHADER_DIR=<dir> cargo test --release --lib dumped_shaders -- --ignored --nocapture` valida os dumps. Em 3000 frames a dash carrega 9 shaders distintos (4 vertex, 5 pixel), todos decodificados sem problemas: vertex shaders com `vfetch_full`/`vfetch_mini` (posição `FMT_32_32_32_FLOAT` + cor `FMT_32_32_32_32_FLOAT`, stride 7) e posição por `dp3`/`dp2add` com constantes; pixel shaders com `tfetch2D` e conversão de cor por `dp4`/`mulsc`, `cndeq` e `sqrt`. Próximo: 4.3, interpretador de shaders em CPU.

**Atualização 23 (2026-10-26, Fase 4.3a, interpretador de shaders):** `gpu/shader_interp.rs` executa microcódigo Xenos em CPU, um vértice ou pixel por vez: controle de fluxo completo (`exec` incondicional/condicional por bool/predicado, `loop`/`endloop` com contagem, aL inicial e passo da constante de laço e break predicado, `call`/`ret`, `jmp`, limite de passos), os 32 opcodes vetoriais e os 51 escalares pelo pseudocódigo do `ucode.h` (multiplicação do Shader Model 3 como no Xenia: `+0` ou denormal vezes qualquer coisa = `+0`; `mad` = mul + add; `cube` sobre `.z_xy`), previous scalar (`ps`), `p0`, `a0`, aL, kill, `_sat`, negação/abs, endereçamento relativo (`a0`/`aL`), constantes do pixel shader a partir de `c256`, exports (posição, interpoladores, point size, cores, profundidade; componentes constantes 0/1; memexport só contado). Constantes e fetches passam por `ShaderEnv` (o ambiente real com o banco de registradores e a memória vem na 4.3b). Desvio documentado: um `loop` com contagem 0 desempilha ao saltar (o Xenia deixa a entrada). Os encoders de teste ficaram em `gpu/ucode_test.rs`. Verificação: 14 testes por opcode/fluxo; `XENON_SHADER_DIR=<dir> cargo test --release --lib dumped_shaders -- --ignored` roda os 9 shaders da dash com um ambiente de teste: todos terminam, os VS exportam posição e os PS exportam `oC0`.

**Atualização 24 (2026-10-26, Fase 4.3b, ambiente de shaders):** `gpu/shader_env.rs` dá ao interpretador o ambiente real: constantes float (`0x4000`, pixel a partir de `c256`), bool (`0x4900`), loop (`0x4908`) e fetch constants (`0x4800`) do banco de registradores; vertex fetch como no Xenia (base `(word0 >> 2) & 0x07FFFFFF` em dwords, `índice × stride` com floor ou arredondamento, offset, palavras além de `base + size` leem 0, swap pelo endian do fetch constant com `guest_endian_swap`, formatos `8_8_8_8`, `2_10_10_10`, `10_11_11`, `11_11_10`, `16_16(_16_16)`, `16_16(_16_16)_FLOAT`, `32*`, `32*_FLOAT`, normalização signed/unsigned com os dois modos de repeating fraction, `exp_adjust`); tfetch pelo trait `TextureSampler` com o fetch constant de textura decodificado (`TextureFetchConstant`: tipo, formato, endian, base, tamanho 2D, tiled, pitch) e um `StubSampler` (texel branco) até a 4.4. `XENON_SHADER_EXEC=<n>` (`gpu/shader_exec.rs`): nos n primeiros draws, roda o VS nos 4 primeiros vértices (índices auto ou DMA de 16/32 bits com swap, mais `VGT_INDX_OFFSET`), imprime posição de clip e de tela (`PA_CL_VTE_CNTL`, viewport) e interpoladores, roda o PS com os interpoladores do primeiro vértice e imprime `oC#` e os tfetch. Resultado em 3000 frames: os 2125 draws rodam sem erro; os quads de UI saem em NDC e caem em pixels de 1280x720 pelo viewport (fundo de -204,5..1523,5 em x), os retângulos de clear/resolve já vêm em pixels; todas as cores são finitas e em [0,1], com animações de alfa visíveis entre frames; as texturas pedidas são formato 6 (`k_8_8_8_8`), 1x1 tiled e 128x1 linear. Próximo: 4.4 (texturas: formatos e untile) e 4.5 (montagem de primitivas e rasterização).

**Investigação (resolvida nas Atualizações 19-21) — notificações de partida (antes desligadas por padrão, `XENON_STARTUP_NOTIFICATIONS=1` liga):** o Xenia entrega ao primeiro listener de cada área `SystemUI(0)`, `SignInChanged(1)`, `StorageDevicesChanged(0)` (dashboard), `LiveConnectionChanged(0x001510F1)`, `LiveLinkStateChanged(0)` e `XmpPlaybackControllerChanged(1)` (`KernelState::RegisterNotifyListener`). Com elas, a GPU para no frame 27: no frame 1155 a thread principal (depois de `XamGetLanguage`/`XamGetLocale`/`XamGetOnlineLanguageAndCountry`/`XamUserGetSigninState(0xFF)` em `0x927BDB98`) entra em `0x928E65F0` com objeto nulo (`r3 = 0`) e percorre para sempre uma lista em memória baixa (`r31 = [r3+0x10]`, laço `0x928E6610..0x928E6654`, cabeça `r31+0x180`). Pilha: `0x928E69C4 0x928DFE10 0x928F2C24 0x927BED48 0x927BF72C 0x927BDBB4 0x921859C8 0x921865D8 0x92186A28 0x92180D14 0x92185848 0x921871F0 0x921967A0`. Por grupo: as notificações de sistema sozinhas e as de Live sozinhas travam; a de XMP sozinha não. Corrigir idioma/locale e o índice de usuário não mudou o resultado. Hipótese: falta estado de perfil/XUI que a dash espera depois de "sign-in changed"; próximo passo é descobrir de onde vem o objeto nulo (quem preenche `[r3+0x10]` no caminho sem notificações).

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

VMX128: `vpkd3d128` e `vupkd3d128` completos (todos os tipos) em `src/cpu/ops/d3d_pack.rs`.

Callback de interrupção da GPU: argumentos e cadência como no Xenia (`app/threads.rs` `interrupt_callback_thread`, `app/frame_loop.rs`).

Exports HLE ainda não tratados (vistos em 400 frames depois da família printf): xboxkrnl `0x0015, 0x004B, 0x0082, 0x00F5, 0x0111`; xam `0x001A, 0x0200, 0x0282, 0x0291, 0x03CA, 0x058A, 0x058C, 0x058D, 0x059F, 0x05DC, 0x05E7, 0x07E2`. Família printf (`0x013A`, `0x013B`, `0x013D`, `0x013E`, `0x014D`..`0x0151`) implementada em `src/kernel/strings.rs`, com testes (`cargo test --release --lib kernel::strings`).

## 4. Ferramentas de depuração existentes

Variáveis de ambiente (todas lidas uma única vez em `src/diag/mod.rs` → `diag::config()`; variável nova entra lá): `XENON_HEADLESS`, `XENON_WATCHDOG` (4s sem progresso imprime frame/pc/lr/syscall, exit 86), `XENON_HEARTBEAT`, `XENON_SYSCALL_LOG`, `XENON_LOG_SYSCALLS` (log de cada despacho HLE no kernel), `XENON_SYSCALL_SUMMARY`, `XENON_PC_PROFILE`, `XENON_PC_HOOK=0xA,0xB` (+`_LIMIT/_MIN_FRAME/_ALL/_MEM=reg:hexoff:words/_LIST=reg/_STRREGS/_R3STR`), `XENON_WATCH=0xADDR` (+`XENON_WATCH_MEM`, `XENON_WATCH_DUMP_HIT`), `XENON_TRAP_TRACE` (+`_FULL`, `XENON_DUMP_RING_FRAME`, `XENON_DUMP_RING_N`), `XENON_PM4_LOG`, `XENON_GPU_MMIO_LOG`, `XENON_FB_SOURCE=reference`, `XENON_SKIP_BOOTANIM`, `XENON_MAX_FRAMES`. Novas: `XENON_DRAW_LOG`, `XENON_MMIO_WRITE_LOG`, `XENON_LIST_IMPORTS`, `XENON_STARTUP_NOTIFICATIONS` (ligada por padrão; `=0` desliga). `XENON_SHADER_DISASM`, `XENON_SHADER_DUMP=<dir>` (Fase 4.2). `XENON_SHADER_EXEC=<n>` (Fase 4.3b). Flags booleanas: `0`/`false`/`off`/`no` = desligada. Removidas na Fase 3.6: `XENON_NO_RESTART`, `XENON_RESTART_MAIN` (threads nunca são reiniciadas).

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

Já implementado em `gpu/xenos.rs`: contadores `real_opcode_counts`, `real_draws`, `pending_interrupts`, arms para 0x58/0x46/0x45/0x54/0x36/0x35/0x22/0x34 e opcodes de estado como no-op; `WAIT_REG_MEM` com formato `[wait_info, poll, ref, mask, wait]`; `REG_RMW` com 3 palavras; WPTR lido de `bus.gpu_mmio[0x714]` (hoje `bus.cp_rb_wptr`; desde o item 4 do fechamento da Fase 2 a tabela de opcodes é a do Xenia, `gpu/pm4.rs`, com opcode de 7 bits, bit de predicado (`bin_mask`/`bin_select`) e flag one-reg do Type-0; os braços com opcodes inventados que colidiam com reais — 0x22 tratado como SET_CONTEXT_REG, 0x23 como desenho no rasterizador antigo, 0x2F com constantes embutidas, 0x34 como WAIT_FOR_IDLE, 0x21 como SET_CONSTANT — foram removidos; desde o fechamento da Fase 2, item 3, CP e MMIO usam um único banco de registradores, `bus.gpu_regs` / `gpu/register_file.rs`, 0x5003 registros como o Xenia). `EVENT_WRITE_SHD` grava o valor lógico sem byte-swap (a memória guest já é big-endian). **Refinado no item 5 do fechamento da Fase 2:** o Xenia grava `GpuSwap(v, endian)` com store little-endian do host; visto pelo guest big-endian isso dá: endian 2 (k8in32) = valor inalterado (o que a dash usa nos fences, endereços `...2`/`...6`), 0 = bswap32, 1 = metades trocadas, 3 = bytes trocados dentro das metades (`guest_endian_swap` em `gpu/xenos.rs`). Vale para `EVENT_WRITE_SHD`, `COND_WRITE`, `WAIT_REG_MEM` (leitura), `MEM_WRITE` (N dwords) e `REG_TO_MEM`; `EVENT_WRITE_SHD` respeita a janela `WRITEBACK_START`/`SIZE` (0x0A04/0x0A05 → virtual `0x7F000000 + offset`) e grava `VGT_EVENT_INITIATOR`; `REG_RMW` aceita operandos de registrador (bits 31/30).

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
- Reiniciar a thread Metro reexecutava o CRT sobre globals inicializadas (InsertTailList duplo). Não reiniciar por padrão; desde a Fase 3.6 nenhuma thread é reiniciada.
- HLE adicionados: `XamGetOnlineSchema (0x01CB)`, `XamUserGetUserFlags 0x021C`, `GetUserTenure 0x03C3`, `SubscriptionType 0x03C4`, `ReadProfileSettings 0x0219`, `XamLoaderGetMediaInfo 0x01A3`, `XamQueryLiveHiveW 0x070D`, `WSAStartup`, `XNetStartup 0x33`, `XHttpStartup/Shutdown 0xC9/0xCA` (=1), `XNetGetEthernetLinkStatus 0x4B` (=0), `XeCryptRandom 0x018A` (inferido da tabela de exports gerada).
