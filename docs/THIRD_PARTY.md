# Código e fontes de terceiros

Este arquivo deve ser atualizado **sempre** que código, tabelas ou algoritmos forem copiados ou portados de outro projeto.

## Política (decisão do usuário)

- **Estratégia A + B pontual.** Portar para Rust usando Xenia/Canary como especificação. Chamar C/C++ somente em módulos isolados e de baixo acoplamento, via FFI.
- **Sem fork do Xenia** e sem embutir seu runtime (JIT, `Processor`, `Memory`, `GraphicsSystem`, backends D3D12/Vulkan).
- Módulos C/C++ ficam em `vendor/<nome>/` com: README (origem, commit, motivo), cópia da licença, API C mínima e wrapper Rust seguro. Cada módulo ganha uma linha na tabela abaixo.
- Cada arquivo portado para Rust leva o cabeçalho de atribuição e entra na tabela com o commit de origem.

## Registro de uso

| Projeto | Licença | Como é usado | Commit fixado | Arquivos/áreas |
|---|---|---|---|---|
| Xenia Canary (https://github.com/xenia-canary/xenia-canary) | BSD 3-Clause, Copyright (c) 2015 Ben Vanik (verificado no `LICENSE` do branch `canary_experimental`) | Referência de semântica e tabelas (opcodes PM4, `vmx128.txt`, exports). Port para Rust (principal) e FFI apenas de módulos isolados (pontual). Sem fork. | `451f915566d57436555c6eb4ee77bb4717eb01de` (`canary_experimental`, 2026-10-24) | `src/kernel/strings.rs` ← `src/xenia/kernel/xboxkrnl/xboxkrnl_strings.{h,cc}` + `util::get_arg_64`/`get_arg_stack_ptr` de `src/xenia/kernel/util/shim_utils.h` (família printf). Desvios documentados no arquivo: `%llx`/`%llu` com aritmética sem sinal; `%hu` estende com zero. `src/kernel/modules.rs` ← `xboxkrnl_modules.cc` (`XexGetModuleHandle`, `XexGetModuleSection`), `xmodule.{h,cc}` (`X_LDR_DATA_TABLE_ENTRY`, `Matches`, `GetSection`), `user_module.cc` (`UserModule::GetSection`, preenchimento da entrada LDR); `src/loader/xex.rs` (`XexResource`/`parse_resource_info`) ← layout `xex2_resource`/`xex2_opt_resource_info` de `src/xenia/kernel/util/xex2_info.h`. Constantes de status de `src/xenia/xbox.h`. |
| Xenia (https://github.com/xenia-project/xenia) | BSD 3-Clause (mesma licença) | Idem, documentação (`docs/cpu.md`, `docs/gpu.md`) | A preencher | |
| Free60 (https://free60.org) | Documentação da comunidade (verificar licença da página usada) | Formatos XEX/STFS/hardware | n/a | |

Já aproveitado até agora (como **referência de dados**, sem cópia de arquivo): lista de opcodes PM4 de `gpu/xenos.h`; encodings VMX128 de `vmx128.txt` (opcodes 5/6); nomes de exports em `src/kernel/exports/{xboxkrnl,xam}_names.rs`, gerados por `tools/src/bin/gen_exports.rs` a partir das tabelas `xboxkrnl_table.inc`/`xam_table.inc` do Xenia (commit de origem não registrado; regerar a partir do commit fixado acima quando o gerador for usado de novo).

O cabeçalho de copyright dos arquivos do Canary nesse commit diz "Copyright 2026 Xenia Canary"; o `LICENSE` do repositório é BSD 3-Clause, "Copyright (c) 2015, Ben Vanik" (verificado no mesmo commit). Os arquivos portados citam os dois.

## Alvos planejados no Xenia Canary (`src/xenia/`)

Pastas do repositório: `app, apu, base, cpu, debug, gpu, hid, kernel, patcher, ui, vfs`.

- `gpu/xenos.h`, `gpu/register_table.inc`: opcodes e registros. Prioridade 1.
- `gpu/command_processor.cc`: lógica do CP. Prioridade 1 (referência para reescrita).
- `gpu/ucode.h`, `gpu/shader*`: decoder de microcódigo. Prioridade 2.
- `gpu/texture_*`: tiling e formatos. Prioridade 2.
- `kernel/xboxkrnl/*`, `kernel/xam/*`: assinaturas e comportamento dos exports. Contínuo.
- `cpu/ppc/ppc_emit_*.cc`, `cpu/ppc/testing/`: semântica e casos de teste de instruções.
- `hid/`, `apu/`, `kernel/xsocket.*`, `kernel/xam/xam_net.cc`: modelo para a ponte de host.

(Os nomes acima vêm da estrutura do repositório e do conhecimento geral do projeto. Confirmar caminho exato no commit fixado antes de usar.)

## Obrigações da licença BSD 3-Clause

1. Manter o aviso de copyright, a lista de condições e o disclaimer em redistribuição de **código-fonte**.
2. Reproduzir o aviso na documentação/materiais em redistribuição **binária**.
3. Não usar o nome do projeto ou dos contribuidores para endossar o produto derivado sem permissão.

Prática no repo: arquivo derivado recebe no topo um comentário `// Derived from Xenia (BSD-3-Clause), Copyright (c) 2015 Ben Vanik. See docs/THIRD_PARTY.md.` e uma linha nova na tabela acima com o commit.

## Cuidados

- `third_party/` do Xenia contém bibliotecas com licenças próprias (ex.: ffmpeg para XMA é LGPL/GPL conforme o build). **Não copiar sem verificar** cada licença.
- Binários e dados da Microsoft (dash.xex, kernel, shaders do sistema, `$SystemUpdate`, NAND dump, chaves `cpukey.txt`/`kv.bin`) são proprietários e **não** devem ser redistribuídos com o projeto nem entrar em commits públicos.
