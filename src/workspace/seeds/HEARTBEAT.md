# Heartbeat Checklist

<!-- IronClaw Agent Runtime Health Checks
     Rotate through these checks 2-4 times per day.
     Stay quiet during 23:00-08:00 UTC unless urgent.
     If nothing needs attention, reply HEARTBEAT_OK.
-->

## Runtime Health

- [ ] `cargo check --all` passes (zero errors)
- [ ] `cargo clippy --all --tests` passes (zero warnings)
- [ ] `cargo test --lib --quiet` passes (3726+ testes)
- [ ] Branch `staging` está limpa ou com WIP documentado
- [ ] Nenhum job stuck no scheduler (`/job list`)

## Code Quality

- [ ] Nenhum `.unwrap()` ou `.expect()` em código de produção (tests OK)
- [ ] Imports usam `crate::` para cross-module, `super::` para intra-module
- [ ] Error types usam `thiserror` com contexto (`.map_err(|e| ...)`)
- [ ] Tipos fortes (enums, newtypes) preferidos sobre strings

## Documentation Drift

- [ ] `CLAUDE.md` reflete mudanças recentes em `src/agent/`
- [ ] `AGENTS.md` está sincronizado com arquitetura atual
- [ ] `ROUTINES.md` cobre novos tipos de trigger/action adicionados
- [ ] `FEATURE_PARITY.md` atualizado se houve mudança de status

## Proactive Work (sem pedir permissão)

- [ ] Curar `MEMORY.md`: remover stale, consolidar duplicados
- [ ] Update daily logs com resumos de sessão
- [ ] Limpar documentos em `context/` desatualizados
- [ ] Revisar ferramentas quebradas (`store.get_broken_tools(5)`)
- [ ] Verificar rotinas desabilitadas ou falhando (`/routine list`)

## Security & Safety

- [ ] Bearer token auth intact (web gateway, webhook routes)
- [ ] CORS, body limits, rate limits configurados
- [ ] Sandbox policy (ReadOnly/WorkspaceWrite/FullAccess) correto
- [ ] Network allowlist para WASM tools atualizado
- [ ] Secrets management: OS keychain para master key

---

**Last Check:** {{timestamp}}
**Next Check:** {{next_timestamp}}
**Status:** {{status}}