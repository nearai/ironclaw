# Rotinas (Routines)

Rotinas são tarefas automatizadas que executam periodicamente ou em resposta a eventos.

## Criar uma Rotina

Use o comando `/routine create` ou a ferramenta `routine_create` com:

```json
{
  "name": "daily-standup",
  "description": "Resumo diário de atividades",
  "trigger": {
    "type": "cron",
    "schedule": "0 9 * * MON-FRI"
  },
  "action": {
    "type": "lightweight",
    "prompt": "Revise as últimas 24h e liste: (1) tarefas completadas, (2) bloqueios, (3) prioridades de hoje"
  },
  "notify": {
    "on_findings": true,
    "on_success": false,
    "on_failure": true
  }
}
```

## Tipos de Trigger

### Cron (agendamento)

```json
{ "type": "cron", "schedule": "0 9 * * MON-FRI" }
```

**Exemplos de schedule:**
- `0 9 * * *` — Todo dia às 9h
- `0 9 * * MON-FRI` — Dias úteis às 9h
- `0 */2 * * *` — A cada 2 horas
- `0 0 * * 0` — Todo domingo à meia-noite

### Event (mensagem)

```json
{
  "type": "event",
  "channel": "telegram",
  "pattern": "(urgente|prioridade|ajuda)"
}
```

Dispara quando uma mensagem corresponde ao regex.

### SystemEvent (evento do sistema)

```json
{
  "type": "system_event",
  "source": "github",
  "event_type": "issue.opened",
  "filters": { "label": "bug" }
}
```

Dispara quando um evento estruturado é emitido.

### Webhook (HTTP POST)

```json
{
  "type": "webhook",
  "path": "ci-complete",
  "secret": "meu-segredo-compartilhado"
}
```

Dispara quando recebe POST em `/api/webhooks/{routine-id}/ci-complete`.

### Manual

```json
{ "type": "manual" }
```

Apenas via comando `/routine run <nome>` ou ferramenta.

## Tipos de Ação

### Lightweight (recomendado para checks rápidos)

Executa inline, sem job separado. Ideal para:
- Checks de status
- Resumos curtos
- Validações simples

```json
{
  "type": "lightweight",
  "prompt": "Verifique se há builds falhando no repositório"
}
```

### Full Job (para tarefas longas)

Dispatcha um job completo no scheduler. Ideal para:
- Refatorações
- Análise profunda
- Tarefas com múltiplos passos

```json
{
  "type": "full_job",
  "prompt": "Analise o diff dos últimos 10 commits e sugira melhorias de código"
}
```

## Exemplos Práticos

### 1. Daily Standup

```json
{
  "name": "daily-standup",
  "description": "Resumo diário de atividades às 9h",
  "trigger": { "type": "cron", "schedule": "0 9 * * MON-FRI" },
  "action": {
    "type": "lightweight",
    "prompt": "Revise as últimas 24h. Liste: (1) o que foi feito, (2) bloqueios, (3) prioridades de hoje. Seja conciso."
  },
  "notify": { "on_findings": true, "on_success": false, "on_failure": true },
  "guardrails": { "max_duration_secs": 60, "quiet_hours": { "start": 23, "end": 8 } }
}
```

### 2. Monitor CI/CD

```json
{
  "name": "ci-monitor",
  "description": "Verifica builds falhando a cada 30min",
  "trigger": { "type": "cron", "schedule": "0 */30 * * *" },
  "action": {
    "type": "lightweight",
    "prompt": "Verifique o status dos últimos builds. Se houver falha, reporte: repositório, branch, erro, link."
  },
  "notify": { "on_findings": true, "on_success": false, "on_failure": true }
}
```

### 3. Alerta de Mensagens Urgentes

```json
{
  "name": "urgent-alert",
  "description": "Notifica mensagens urgentes no Telegram",
  "trigger": {
    "type": "event",
    "channel": "telegram",
    "pattern": "(urgente|emergência|prioridade máxima|socorro)"
  },
  "action": {
    "type": "lightweight",
    "prompt": "Uma mensagem urgente foi recebida. Encaminhe imediatamente com contexto."
  },
  "notify": { "on_findings": true, "on_success": true, "on_failure": true }
}
```

### 4. Weekly Report

```json
{
  "name": "weekly-report",
  "description": "Relatório semanal toda sexta às 17h",
  "trigger": { "type": "cron", "schedule": "0 17 * * FRI" },
  "action": {
    "type": "full_job",
    "prompt": "Gere um relatório semanal completo: (1) sessões da semana, (2) ferramentas mais usadas, (3) padrões observados, (4) recomendações para próxima semana."
  },
  "notify": { "on_findings": true, "on_success": true, "on_failure": true }
}
```

### 5. GitHub PR Monitor

```json
{
  "name": "pr-monitor",
  "description": "Monitora PRs abertos no GitHub",
  "trigger": {
    "type": "system_event",
    "source": "github",
    "event_type": "pull_request.opened"
  },
  "action": {
    "type": "lightweight",
    "prompt": "Um novo PR foi aberto. Resuma: título, autor, arquivos principais, descrição."
  },
  "notify": { "on_findings": true, "on_success": false, "on_failure": true }
}
```

### 6. Health Check Noturno

```json
{
  "name": "nightly-health",
  "description": "Check de saúde noturno do sistema",
  "trigger": { "type": "cron", "schedule": "0 2 * * *" },
  "action": {
    "type": "lightweight",
    "prompt": "Execute health checks: (1) disco disponível, (2) memória, (3) serviços ativos, (4) logs de erro recentes. Reporte apenas anomalias."
  },
  "guardrails": { "quiet_hours": { "start": 23, "end": 7 } },
  "notify": { "on_findings": true, "on_success": false, "on_failure": true }
}
```

## Comandos

| Comando | Descrição |
|---------|-----------|
| `/routine list` | Lista todas as rotinas |
| `/routine create` | Cria nova rotina (interativo) |
| `/routine run <nome>` | Executa rotina manualmente |
| `/routine enable <nome>` | Habilita rotina |
| `/routine disable <nome>` | Desabilita rotina |
| `/routine delete <nome>` | Remove rotina |

## Guardrails

```json
{
  "max_duration_secs": 300,
  "max_cost_cents": 50,
  "max_actions_per_hour": 10,
  "quiet_hours": { "start": 23, "end": 7, "timezone": "America/Sao_Paulo" },
  "require_approval_for": ["shell", "file_write"]
}
```

## Configuração de Notificação

```json
{
  "on_findings": true,
  "on_success": false,
  "on_failure": true,
  "summary_mode": "batch",
  "batch_window_mins": 60
}
```

- `on_findings`: notifica apenas se houver algo relevante
- `on_success`: notifica mesmo se tudo OK (raro, use false)
- `on_failure`: notifica se a rotina falhar
- `summary_mode`: `batch` agrupa notificações em janelas

## Boas Práticas

1. **Comece com lightweight** — Full job só se necessário
2. **Seja específico no prompt** — Quanto mais claro, melhor o resultado
3. **Use quiet_hours** — Evite notificações noturnas
4. **Monitore falhas** — `consecutive_failures` desabilita automaticamente
5. **Revise periodicamente** — Remova rotinas não utilizadas
