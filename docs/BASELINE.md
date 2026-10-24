# Baseline (Fase 0)

Referência de comportamento antes da reestruturação. Toda etapa da Fase 1 deve reproduzir estes números exatamente.

- Data: 2026-10-23
- Toolchain: rustc 1.99.1, cargo 1.99.1 (Windows 11, MSVC)
- Build: `cargo build --release` → `Finished` (2 warnings no lib, 2 duplicadas no bin)
- Imagem: `dash.xex` 17559 (`$SystemUpdate`), `XENON_SKIP_BOOTANIM=1`
- Tempo de execução (400 frames, headless): ~2,6 s

## Como reproduzir

A partir de `D:\XBOX 360 - Firmeware Emulator`:

```powershell
.\xenon-emu\scripts\trapscan.ps1 -Frames 400 -TimeoutSec 150
.\xenon-emu\scripts\iter.ps1 -Frames 400 -NoBuild -TimeoutSec 150 -Env 'XENON_HEADLESS=1','XENON_PM4_LOG=1'
.\xenon-emu\scripts\iter.ps1 -Frames 400 -NoBuild -TimeoutSec 150 -Env 'XENON_HEADLESS=1','XENON_PC_PROFILE=1'
```

As três execuções deram números idênticos (execução determinística).

## Resultado do trapscan (400 frames)

```
NO TRAP: primary thread did not stop within 400 frames.
Primary at end: "Metro 17559 Primary Thread" | PC: 0x921967E0 | LR: 0x921967D8 | Steps: 8608205
GPU: [GPU CP] Frame 00265 | Processed 2 PM4 packets (Total: 120, Draw Calls: 0, Primitives: 0) | WPTR: 49, RPTR: 49
[*] Reached XENON_MAX_FRAMES limit (400 frames) - terminating loop cleanly.
```

Saída `.err` vazia; sem `THREAD TRAP`, `WATCHDOG` ou `panicked`.

## Threads no frame 400

| Thread | PC | LR | Steps |
|---|---|---|---|
| Metro 17559 Primary | 0x921967E0 | 0x921967D8 | 8608205 |
| 0x1 | 0x922618D8 | 0x922618D8 | 14371 |
| 0x2 | 0x921C9830 | 0x80001000 | 30549 |
| 0x3 | 0x921979B4 | 0x92492C20 | 6884850 |
| 0x4 | 0x9281130C | 0x9281130C | 51200 |
| 0x5 | 0x92267C58 | 0x80001000 | 27275 |
| 0x6 | 0x92267C58 | 0x80001000 | 27234 |
| 0x7 | 0x92267C58 | 0x80001000 | 27296 |
| 0x8 | 0x92267C58 | 0x80001000 | 27708 |
| 0x9 | 0x9226A1F0 | 0x9226A1F0 | 38963 |
| 0xA | 0x927C6C7C | 0x927C6C7C | 7694 |
| 0xB | 0x9226E410 | 0x80001000 | 9899 |
| 0xC | 0x9226E410 | 0x80001000 | 9899 |
| 0xD | 0x9225D978 | 0x80001000 | 11105 |
| 0xE | 0x9225C210 | 0x80001000 | 45315 |
| 0xF | 0x9225D070 | 0x80001000 | 9956 |
| 0x10 | 0x92264E28 | 0x80001000 | 7533 |
| 0x11 | 0x92265188 | 0x80001000 | 11428 |
| 0x12 | 0x92197A00 | 0x92197A00 | 7365 |
| 0x13 | 0x922628D0 | 0x80001000 | 13774 |

## PM4 (`XENON_PM4_LOG=1`, 400 frames)

Pacotes Type-0: 49. Type-2: 0. Type-3 por opcode (nome como impresso pela tabela atual de `pm4.rs`, que é inventada):

| Opcode | Nome atual no log | Nome real (Xenia `xenos.h`) | Qtde |
|---|---|---|---|
| 0x10 | PM4_NOP | NOP | 5 |
| 0x21 | PM4_SET_CONSTANT | REG_RMW | 2 |
| 0x2B | UNKNOWN | IM_LOAD_IMMEDIATE | 2 |
| 0x36 | UNKNOWN | DRAW_INDX_2 | 24 |
| 0x3B | UNKNOWN | INVALIDATE_STATE | 6 |
| 0x3C | PM4_WAIT_REG_MEM | WAIT_REG_MEM | 4 |
| 0x3F | PM4_EXECUTE_INDIRECT | INDIRECT_BUFFER | 13 |
| 0x48 | PM4_ME_INIT | ME_INIT | 1 |
| 0x58 | UNKNOWN | EVENT_WRITE_SHD | 10 |
| 0x60-0x63 | UNKNOWN | SET_BIN_MASK/SELECT | 1 cada |

Contador do CP: Total 120 pacotes, Draw Calls 0, Primitives 0, WPTR = RPTR = 49.

## PC profile (`XENON_PC_PROFILE=1`, 400 frames)

Pontos mais quentes da thread principal:

- `0x92198CD8..0x92198CEC`: 177615 hits por instrução (6 instruções).
- `0x92198658..0x921986E4` e `0x921967C8..0x921967E8`: 102773 hits por instrução.
- Thread 0x3: `0x92198C2C..0x92198C34`, 91289 hits.

Syscalls mais chamadas: thread 0x3 `xboxkrnl #0x0125` e `#0x0130` (121712 cada), `#0x00F4` (30428); principal `#0x0125` (5925).

**Observação:** no frame 400 o laço quente da principal está em `0x92198xxx`/`0x921967xx`, e não no laço de fence `0x921AFFF8..0x921B0020` citado no `HANDOFF.md`. Investigar na Fase 2 antes de assumir que o bloqueio é o fence. **Resolvido na Fase 2:** é o fim do processo (o `main` retornou E_INVALIDARG no frame 255 por falta de `_snwprintf`; ver `HANDOFF.md` seção 2).

## Não determinismo conhecido

Tabelas montadas a partir de `HashMap` mudam de ordem entre execuções: o dump de campos opcionais do cabeçalho XEX no boot e, nas tabelas top-N (`[PROFILE]`, `[SYSCALL SUMMARY]`), a ordem e o último item entre entradas empatadas. Comparar essas seções como conjunto, ignorando empates no corte.

## Comparar com um build anterior

Para refactors que não devem mudar comportamento (a partir de `D:\XBOX 360 - Firmeware Emulator`):

```powershell
$old = .\xenon-emu\scripts\build_ref.ps1 -Ref HEAD   # build do commit de referência num worktree temporário
.\xenon-emu\scripts\cmp_diag.ps1 -Old $old          # 3 conjuntos de XENON_* (logs, hooks, watch): stdout e stderr
.\xenon-emu\scripts\time_runs.ps1 -Old $old         # tempo de 400 frames, alternando old/new
.\xenon-emu\scripts\build_ref.ps1 -Clean            # remove o worktree
```

## Depois da família printf (Fase 2, 2026-10-24)

Mudança de comportamento intencional (não é refactor). Trapscan de 400 frames:

```
NO TRAP: primary thread did not stop within 400 frames.
Primary at end: "Metro 17559 Primary Thread" | PC: 0x9219865C | LR: 0x921967D8 | Steps: 8593026
GPU: [GPU CP] Frame 00268 | Processed 2 PM4 packets (Total: 120, Draw Calls: 0, Primitives: 0) | WPTR: 49, RPTR: 49
```

O `main` agora retorna `0x8030001C` no frame 258 (antes: `0x80070057` no frame 255). Ver `HANDOFF.md` seção 2.

## Depois de XexGetModuleHandle/XexGetModuleSection (Fase 2, 2026-10-24)

```
NO TRAP: primary thread did not stop within 400 frames.
Primary at end: "Metro 17559 Primary Thread" | PC: 0x9219867C | LR: 0x921967D8 | Steps: 8597958
GPU: [GPU CP] Frame 00267 | Processed 2 PM4 packets (Total: 120, Draw Calls: 0, Primitives: 0) | WPTR: 49, RPTR: 49
```

O `main` retorna `0x80004005` no frame 257.

## Depois de RtlUnicodeToMultiByteN (Fase 2, 2026-10-24)

```
NO TRAP: primary thread did not stop within 400 frames.
Primary at end: "Metro 17559 Primary Thread" | PC: 0x9281D7D4 | LR: 0x9281C518 | Steps: 8951503
GPU: [GPU CP] Frame 00028 | Processed 3 PM4 packets (Total: 72, Draw Calls: 0, Primitives: 0) | WPTR: 25, RPTR: 25
```

O `main` não retorna mais. Em 1500 frames a thread principal para no frame 1100: `Unimplemented VMX128 vpkd3d128 (insn=0x1BE166D4)` em `0x9221FA28`.

## Depois de vpkd3d128 (Fase 2, 2026-10-24)

3000 frames (~23 s headless): a thread principal para no frame 1175 com `Trap condition met (twi TO=6 r9=0 SIMM=0) at PC=0x921BEB30`. Último PM4: frame 1175, total 178 pacotes, 0 draws. Frontbuffer a partir do frame 1175: `0x90314000`.

## Depois de VdQueryVideoMode (Fase 2, 2026-10-24)

4000 frames (~28 s headless): a thread principal para no frame 1175 com `Unimplemented opcode 31 xo 567 (insn=0x7DAA4C6E)` em `0x921BE5A0` (`lfsux`). PM4 igual ao anterior (último no frame 1175, total 178, 0 draws); frontbuffer `0x90314000`.

## Depois de lfsux/lfdux/stfsux/stfdux (Fase 2, 2026-10-24)

4000 frames (~34 s headless): **sem trap**. Thread principal ativa no fim (`PC 0x921A413C`, 98 895 764 passos), girando em `0x921A40B0` (espera de fence chamada do laço principal da aplicação). PM4: frame 1176 com 568 pacotes, total 746; histograma até o frame 1400 em `HANDOFF.md`.

## Depois do DPC (Fase 2, 2026-10-24): GPU destravada

3000 frames (~28 s headless): 415 escritas de `CP_RB_WPTR` (último `7775`), fence até `0x62D`, 75 660 pacotes PM4, sem trap; thread principal em `0x921C69B4`.
5000 frames (~43 s): o ring dá a volta (WPTR `4731`), 177 711 pacotes, `XE_SWAP` ×1044, `DRAW_INDX_2` ×2112; thread principal em `0x921AEE20`, 121 322 324 passos.

## Depois de tirar as pilhas da imagem (fechamento da Fase 2, item 1)

3000 frames: 411 escritas de `CP_RB_WPTR` (último `7715`), fence até `0x621`, sem trap. A sequência de submissões é a mesma do baseline anterior (que o HEAD reproduz exatamente: 415 / `0x62D`), só que atrasada: a primeira submissão depois da carga passa do frame 1128 para 1158 e o regime fica ~11 frames atrás. A pilha não cai mais sobre `hubapp`.
Comparar com outro build: `iter.ps1 -Exe <exe>` (por exemplo o de `build_ref.ps1`).

## Depois do `mftb` global (fechamento da Fase 2, item 2)

3000 frames: idêntico ao item 1 (411 escritas de `CP_RB_WPTR`, último `7715`, fence `0x621`). Esperado: o único `mftb` da dash (`0x92617738`) não executa nessa janela (`XENON_PC_HOOK`).

## Depois do banco de registradores único (fechamento da Fase 2, item 3)

3000 frames: idêntico ao item 2 (411 / `7715` / fence `0x621`).

## Depois da tabela PM4 real (fechamento da Fase 2, item 4)

3000 frames: idêntico ao item 3 (411 / `7715` / fence `0x621`). A dash não emite nenhum dos opcodes que colidiam com os inventados.
`XENON_PM4_LOG=1`, 1400 frames (6374 pacotes): Type-0 ×3283 (1 one-reg), `COND_WRITE` ×2560, `EVENT_WRITE_SHD` ×92, `WAIT_REG_MEM` ×74, `INDIRECT_BUFFER` ×69, `INVALIDATE_STATE` ×54, `DRAW_INDX_2` ×34 (+10 predicados), `NOP` ×33, `IM_LOAD_IMMEDIATE` ×32, `SET_BIN_MASK_LO` ×30, `SET_BIN_MASK_HI`/`SET_BIN_SELECT_LO`/`SET_BIN_SELECT_HI` ×20 cada, `REG_RMW` ×12, `XE_SWAP`/`INTERRUPT`/`EVENT_WRITE` ×10, `ME_INIT` ×1.

## Depois de endianness/writeback (fechamento da Fase 2, item 5)

3000 frames: idêntico ao item 4 (411 / `7715` / fence `0x621`). A dash usa endian 2 (k8in32) nos endereços de memória do CP.

## Critério de regressão (Fase 1)

Após cada etapa, com o mesmo comando do trapscan:

- Principal: `PC 0x921967E0`, `Steps 8608205`.
- GPU: `Total: 120`, `WPTR: 49, RPTR: 49`, `Draw Calls: 0`.
- Histograma PM4 igual à tabela acima.
- Sem trap, watchdog ou panic.

As saídas completas desta baseline não ficam no repositório (contêm caminhos locais e dados derivados da dash).
