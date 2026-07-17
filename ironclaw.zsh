#compdef ironclaw

autoload -U is-at-least

_ironclaw() {
    typeset -A opt_args
    typeset -a _arguments_options
    local ret=1

    if is-at-least 5.2; then
        _arguments_options=(-s -S -C)
    else
        _arguments_options=(-s -C)
    fi

    local context curcontext="$curcontext" state line
    _arguments "${_arguments_options[@]}" : \
'-h[Print help]' \
'--help[Print help]' \
'-V[Print version]' \
'--version[Print version]' \
":: :_ironclaw_commands" \
"*::: :->ironclaw" \
&& ret=0
    case $state in
    (ironclaw)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:ironclaw-command-$line[1]:"
        case $line[1] in
            (channels)
_arguments "${_arguments_options[@]}" : \
'-h[Print help]' \
'--help[Print help]' \
":: :_ironclaw__subcmd__channels_commands" \
"*::: :->channels" \
&& ret=0

    case $state in
    (channels)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:ironclaw-channels-command-$line[1]:"
        case $line[1] in
            (list)
_arguments "${_arguments_options[@]}" : \
'-v[Show extra status details]' \
'--verbose[Show extra status details]' \
'--json[Output channels as JSON]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
":: :_ironclaw__subcmd__channels__subcmd__help_commands" \
"*::: :->help" \
&& ret=0

    case $state in
    (help)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:ironclaw-channels-help-command-$line[1]:"
        case $line[1] in
            (list)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
        esac
    ;;
esac
;;
(completion)
_arguments "${_arguments_options[@]}" : \
'--shell=[The shell to generate completions for]:SHELL:(bash elvish fish powershell zsh)' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(config)
_arguments "${_arguments_options[@]}" : \
'-h[Print help]' \
'--help[Print help]' \
":: :_ironclaw__subcmd__config_commands" \
"*::: :->config" \
&& ret=0

    case $state in
    (config)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:ironclaw-config-command-$line[1]:"
        case $line[1] in
            (path)
_arguments "${_arguments_options[@]}" : \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(init)
_arguments "${_arguments_options[@]}" : \
'--force[Overwrite existing files]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(list)
_arguments "${_arguments_options[@]}" : \
'--json[Output as JSON]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(get)
_arguments "${_arguments_options[@]}" : \
'--json[Output as JSON]' \
'-h[Print help]' \
'--help[Print help]' \
':key -- Dot-separated config key (e.g. boot.profile, llm.default.model):_default' \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
":: :_ironclaw__subcmd__config__subcmd__help_commands" \
"*::: :->help" \
&& ret=0

    case $state in
    (help)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:ironclaw-config-help-command-$line[1]:"
        case $line[1] in
            (path)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(init)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(list)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(get)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
        esac
    ;;
esac
;;
(doctor)
_arguments "${_arguments_options[@]}" : \
'--json[Output as JSON]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(extension)
_arguments "${_arguments_options[@]}" : \
'--confirm-host-access[Confirm trusted-laptop host filesystem access for local-dev-yolo]' \
'-h[Print help]' \
'--help[Print help]' \
":: :_ironclaw__subcmd__extension_commands" \
"*::: :->extension" \
&& ret=0

    case $state in
    (extension)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:ironclaw-extension-command-$line[1]:"
        case $line[1] in
            (search)
_arguments "${_arguments_options[@]}" : \
'--json[Output the lifecycle response as JSON]' \
'--confirm-host-access[Confirm trusted-laptop host filesystem access for local-dev-yolo]' \
'-h[Print help]' \
'--help[Print help]' \
'::query -- Query extension id, name, or description. Omit to list all local packages:_default' \
&& ret=0
;;
(install)
_arguments "${_arguments_options[@]}" : \
'--json[Output the lifecycle response as JSON]' \
'--confirm-host-access[Confirm trusted-laptop host filesystem access for local-dev-yolo]' \
'-h[Print help]' \
'--help[Print help]' \
':id -- Extension id from `ironclaw extension search`:_default' \
&& ret=0
;;
(activate)
_arguments "${_arguments_options[@]}" : \
'--json[Output the lifecycle response as JSON]' \
'--confirm-host-access[Confirm trusted-laptop host filesystem access for local-dev-yolo]' \
'-h[Print help]' \
'--help[Print help]' \
':id -- Extension id from `ironclaw extension search`:_default' \
&& ret=0
;;
(remove)
_arguments "${_arguments_options[@]}" : \
'--json[Output the lifecycle response as JSON]' \
'--confirm-host-access[Confirm trusted-laptop host filesystem access for local-dev-yolo]' \
'-h[Print help]' \
'--help[Print help]' \
':id -- Extension id from `ironclaw extension search`:_default' \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
":: :_ironclaw__subcmd__extension__subcmd__help_commands" \
"*::: :->help" \
&& ret=0

    case $state in
    (help)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:ironclaw-extension-help-command-$line[1]:"
        case $line[1] in
            (search)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(install)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(activate)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(remove)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
        esac
    ;;
esac
;;
(hooks)
_arguments "${_arguments_options[@]}" : \
'-h[Print help]' \
'--help[Print help]' \
":: :_ironclaw__subcmd__hooks_commands" \
"*::: :->hooks" \
&& ret=0

    case $state in
    (hooks)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:ironclaw-hooks-command-$line[1]:"
        case $line[1] in
            (list)
_arguments "${_arguments_options[@]}" : \
'-v[Show extra status details]' \
'--verbose[Show extra status details]' \
'--json[Output hooks as JSON]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
":: :_ironclaw__subcmd__hooks__subcmd__help_commands" \
"*::: :->help" \
&& ret=0

    case $state in
    (help)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:ironclaw-hooks-help-command-$line[1]:"
        case $line[1] in
            (list)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
        esac
    ;;
esac
;;
(logs)
_arguments "${_arguments_options[@]}" : \
'-v[Show extra status details]' \
'--verbose[Show extra status details]' \
'--json[Output log status as JSON]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(models)
_arguments "${_arguments_options[@]}" : \
'-h[Print help]' \
'--help[Print help]' \
":: :_ironclaw__subcmd__models_commands" \
"*::: :->models" \
&& ret=0

    case $state in
    (models)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:ironclaw-models-command-$line[1]:"
        case $line[1] in
            (list)
_arguments "${_arguments_options[@]}" : \
'-v[Show provider protocol and credential metadata]' \
'--verbose[Show provider protocol and credential metadata]' \
'--json[Output providers as JSON]' \
'-h[Print help]' \
'--help[Print help]' \
'::provider -- Show only a specific provider by id or alias:_default' \
&& ret=0
;;
(status)
_arguments "${_arguments_options[@]}" : \
'--json[Output model status as JSON]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(set)
_arguments "${_arguments_options[@]}" : \
'-h[Print help]' \
'--help[Print help]' \
':model -- Model name (for example, gpt-5-mini or claude-sonnet-4-6-20250514):_default' \
&& ret=0
;;
(set-provider)
_arguments "${_arguments_options[@]}" : \
'--model=[Also set the model. Defaults to the provider'\''s catalog default]:MODEL:_default' \
'-h[Print help]' \
'--help[Print help]' \
':provider -- Provider id or alias (for example, openai, anthropic, ollama):_default' \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
":: :_ironclaw__subcmd__models__subcmd__help_commands" \
"*::: :->help" \
&& ret=0

    case $state in
    (help)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:ironclaw-models-help-command-$line[1]:"
        case $line[1] in
            (list)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(status)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(set)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(set-provider)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
        esac
    ;;
esac
;;
(onboard)
_arguments "${_arguments_options[@]}" : \
'--force[Overwrite generated config.toml, providers.json, and the completion marker]' \
'--dry-run[Show what would be initialized without writing files]' \
'--import-history[Reserve the history-import step in the onboarding summary]' \
'-h[Print help (see more with '\''--help'\'')]' \
'--help[Print help (see more with '\''--help'\'')]' \
&& ret=0
;;
(profile)
_arguments "${_arguments_options[@]}" : \
'-h[Print help]' \
'--help[Print help]' \
":: :_ironclaw__subcmd__profile_commands" \
"*::: :->profile" \
&& ret=0

    case $state in
    (profile)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:ironclaw-profile-command-$line[1]:"
        case $line[1] in
            (list)
_arguments "${_arguments_options[@]}" : \
'--json[Output profiles as JSON]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
":: :_ironclaw__subcmd__profile__subcmd__help_commands" \
"*::: :->help" \
&& ret=0

    case $state in
    (help)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:ironclaw-profile-help-command-$line[1]:"
        case $line[1] in
            (list)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
        esac
    ;;
esac
;;
(repl)
_arguments "${_arguments_options[@]}" : \
'--confirm-host-access[Confirm trusted-laptop host filesystem access for local-dev-yolo]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(run)
_arguments "${_arguments_options[@]}" : \
'-m+[Send a single message, print the assistant reply, and exit. Without this flag, the CLI reads lines from stdin in a loop]:MESSAGE:_default' \
'--message=[Send a single message, print the assistant reply, and exit. Without this flag, the CLI reads lines from stdin in a loop]:MESSAGE:_default' \
'--dry-run[Print the substrate readiness snapshot and exit without starting the agent. Preserves the legacy \`run\` diagnostic shape so existing smoke tests keep passing]' \
'--confirm-host-access[Confirm trusted-laptop host filesystem access for local-dev-yolo]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(serve)
_arguments "${_arguments_options[@]}" : \
'--host=[Host interface for the Reborn WebChat v2 HTTP listener. Overrides \`\[webui\].listen_host\` from the boot config file. Default (when neither is set) is \`127.0.0.1\`]:HOST:_default' \
'--port=[Port for the Reborn WebChat v2 HTTP listener. \`0\` lets the kernel pick a free port (useful for tests). Overrides \`\[webui\].listen_port\` from the boot config file. Default (when neither is set) is 3000]:PORT:_default' \
'--confirm-host-access[Confirm trusted-laptop host filesystem access for local-dev-yolo]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(service)
_arguments "${_arguments_options[@]}" : \
'-h[Print help]' \
'--help[Print help]' \
":: :_ironclaw__subcmd__service_commands" \
"*::: :->service" \
&& ret=0

    case $state in
    (service)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:ironclaw-service-command-$line[1]:"
        case $line[1] in
            (install)
_arguments "${_arguments_options[@]}" : \
'-h[Print help (see more with '\''--help'\'')]' \
'--help[Print help (see more with '\''--help'\'')]' \
&& ret=0
;;
(start)
_arguments "${_arguments_options[@]}" : \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(stop)
_arguments "${_arguments_options[@]}" : \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(restart)
_arguments "${_arguments_options[@]}" : \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(status)
_arguments "${_arguments_options[@]}" : \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(uninstall)
_arguments "${_arguments_options[@]}" : \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
":: :_ironclaw__subcmd__service__subcmd__help_commands" \
"*::: :->help" \
&& ret=0

    case $state in
    (help)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:ironclaw-service-help-command-$line[1]:"
        case $line[1] in
            (install)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(start)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(stop)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(restart)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(status)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(uninstall)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
        esac
    ;;
esac
;;
(skills)
_arguments "${_arguments_options[@]}" : \
'-h[Print help]' \
'--help[Print help]' \
":: :_ironclaw__subcmd__skills_commands" \
"*::: :->skills" \
&& ret=0

    case $state in
    (skills)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:ironclaw-skills-command-$line[1]:"
        case $line[1] in
            (list)
_arguments "${_arguments_options[@]}" : \
'-v[Show extra status details]' \
'--verbose[Show extra status details]' \
'--json[Output skills as JSON]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
":: :_ironclaw__subcmd__skills__subcmd__help_commands" \
"*::: :->help" \
&& ret=0

    case $state in
    (help)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:ironclaw-skills-help-command-$line[1]:"
        case $line[1] in
            (list)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
        esac
    ;;
esac
;;
(status)
_arguments "${_arguments_options[@]}" : \
'--json[Output as JSON]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(traces)
_arguments "${_arguments_options[@]}" : \
'-h[Print help]' \
'--help[Print help]' \
":: :_ironclaw__subcmd__traces_commands" \
"*::: :->traces" \
&& ret=0

    case $state in
    (traces)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:ironclaw-traces-command-$line[1]:"
        case $line[1] in
            (opt-in)
_arguments "${_arguments_options[@]}" : \
'--endpoint=[Explicit private ingestion endpoint URL]:ENDPOINT:_default' \
'--user-scope=[Runtime/web user scope to configure; defaults to this instance'\''s owner_id]:USER_SCOPE:_default' \
'--bearer-token-env=[Environment variable containing the bearer token for the endpoint]:BEARER_TOKEN_ENV:_default' \
'--upload-token-issuer-url=[HTTPS issuer URL that returns short-lived EdDSA upload claims]:UPLOAD_TOKEN_ISSUER_URL:_default' \
'*--upload-token-issuer-allowed-hosts=[Exact allowed issuer hostnames for upload claim refresh]:UPLOAD_TOKEN_ISSUER_ALLOWED_HOSTS:_default' \
'--upload-token-audience=[Audience to request from the upload claim issuer]:UPLOAD_TOKEN_AUDIENCE:_default' \
'--upload-token-tenant-id=[Tenant ID to request from the upload claim issuer]:UPLOAD_TOKEN_TENANT_ID:_default' \
'--upload-token-workload-token-env=[Environment variable containing workload credentials for the issuer]:UPLOAD_TOKEN_WORKLOAD_TOKEN_ENV:_default' \
'--upload-token-invite-code=[Operator-issued pilot invite code. When set, included in upload-claim refresh requests so the issuer'\''s allowlist gate can match it. Required only when the configured issuer runs with TRACE_COMMONS_ALLOWLIST_SOURCE — omit otherwise]:UPLOAD_TOKEN_INVITE_CODE:_default' \
'--upload-token-issuer-timeout-ms=[Upload claim issuer timeout in milliseconds]:UPLOAD_TOKEN_ISSUER_TIMEOUT_MS:_default' \
'--scope=[Consent scope to include in autonomous envelopes]:SCOPE:(debugging-evaluation benchmark-only ranking-training model-training)' \
'*--selected-tools=[Only auto-submit traces that use these tool names]:SELECTED_TOOLS:_default' \
'--min-submission-score=[Minimum local score required for autonomous submission]:MIN_SUBMISSION_SCORE:_default' \
'--include-message-text[Include locally redacted user/assistant message text]' \
'--include-tool-payloads[Include locally redacted tool arguments, tool results, and HTTP bodies]' \
'--allow-pii-review-bypass[Submit medium-risk traces without holding them for manual review]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(opt-out)
_arguments "${_arguments_options[@]}" : \
'--user-scope=[Runtime/web user scope to disable (scoped-only opt-out); defaults to this instance'\''s owner_id AND disables the global policy]:USER_SCOPE:_default' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(enroll-instance)
_arguments "${_arguments_options[@]}" : \
'--invite=[Operator invite link (\`https\://<host>#<code>\`, or \`<code>@<host>\`)]:INVITE:_default' \
'--include-message-text[Include locally redacted user/assistant message text in envelopes (applies instance-wide to every inheriting user)]' \
'--include-tool-payloads[Include locally redacted tool arguments, tool results, and HTTP bodies in envelopes (applies instance-wide)]' \
'--json[Output the enrollment outcome as JSON]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(status)
_arguments "${_arguments_options[@]}" : \
'--user-scope=[Show the runtime/web policy for this user scope instead of the global CLI policy]:USER_SCOPE:_default' \
'--json[Output as JSON]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(preview)
_arguments "${_arguments_options[@]}" : \
'--recorded-trace=[Recorded trace JSON file produced by IRONCLAW_RECORD_TRACE]:PATH:_files' \
'--scope=[Consent scope to include in the envelope]:SCOPE:(debugging-evaluation benchmark-only ranking-training model-training)' \
'--channel=[Source channel for this trace]:CHANNEL:(web cli telegram slack routine other)' \
'--engine-version=[Optional engine version metadata]:ENGINE_VERSION:_default' \
'--contributor-id=[Optional pseudonymous contributor ID]:CONTRIBUTOR_ID:_default' \
'--credit-account-ref=[Optional separate credit account reference]:CREDIT_ACCOUNT_REF:_default' \
'-o+[Write envelope JSON to a file instead of stdout]:PATH:_files' \
'--output=[Write envelope JSON to a file instead of stdout]:PATH:_files' \
'--include-message-text[Include locally redacted user/assistant message text]' \
'--include-tool-payloads[Include locally redacted tool arguments, tool results, and HTTP bodies]' \
'--enqueue[Add the redacted envelope to the autonomous submission queue]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(enqueue)
_arguments "${_arguments_options[@]}" : \
'--envelope=[Redacted contribution envelope JSON file]:PATH:_files' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(flush-queue)
_arguments "${_arguments_options[@]}" : \
'--limit=[Maximum queued envelopes to submit]:LIMIT:_default' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(queue-status)
_arguments "${_arguments_options[@]}" : \
'--scope=[Local tenant/user trace scope to inspect]:SCOPE:_default' \
'--json[Output as JSON]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(credit)
_arguments "${_arguments_options[@]}" : \
'--notice-scope=[Local tenant/user trace scope to check for a due periodic notice]:NOTICE_SCOPE:_default' \
'(--ack)--snooze-hours=[Snooze the current credit notice for this many hours]:SNOOZE_HOURS:_default' \
'--json[Output as JSON]' \
'--notice[Print and mark a due periodic credit notice instead of the full credit report]' \
'(--snooze-hours)--ack[Acknowledge the current credit notice until credit changes again]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(submit)
_arguments "${_arguments_options[@]}" : \
'--envelope=[Redacted contribution envelope JSON file]:PATH:_files' \
'--endpoint=[Explicit private ingestion endpoint URL]:ENDPOINT:_default' \
'--bearer-token-env=[Environment variable containing the bearer token for the endpoint]:BEARER_TOKEN_ENV:_default' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(list-submissions)
_arguments "${_arguments_options[@]}" : \
'--json[Output as JSON]' \
'--summary[Include aggregate submission and credit totals]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(revoke)
_arguments "${_arguments_options[@]}" : \
'--endpoint=[Optional private revocation endpoint URL]:ENDPOINT:_default' \
'--bearer-token-env=[Environment variable containing the bearer token for the endpoint]:BEARER_TOKEN_ENV:_default' \
'-h[Print help]' \
'--help[Print help]' \
':submission_id -- Submission ID to revoke:_default' \
&& ret=0
;;
(ingest-health)
_arguments "${_arguments_options[@]}" : \
'--endpoint=[Trace Commons ingestion base URL, /health URL, or /v1/traces URL]:ENDPOINT:_default' \
'--json[Output as JSON]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(profile)
_arguments "${_arguments_options[@]}" : \
'-h[Print help]' \
'--help[Print help]' \
":: :_ironclaw__subcmd__traces__subcmd__profile_commands" \
"*::: :->profile" \
&& ret=0

    case $state in
    (profile)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:ironclaw-traces-profile-command-$line[1]:"
        case $line[1] in
            (token)
_arguments "${_arguments_options[@]}" : \
'--user-scope=[Runtime/web user scope; defaults to this instance'\''s owner_id]:USER_SCOPE:_default' \
'--json[Output as JSON]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(set)
_arguments "${_arguments_options[@]}" : \
'--handle=[Pseudonymous display handle (3-32 chars\: ASCII letters, digits, '\''-'\'', '\''_'\'')]:HANDLE:_default' \
'--bio=[Optional short bio (max 280 bytes)]:BIO:_default' \
'--user-scope=[Runtime/web user scope; defaults to this instance'\''s owner_id]:USER_SCOPE:_default' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(withdraw)
_arguments "${_arguments_options[@]}" : \
'--user-scope=[Runtime/web user scope; defaults to this instance'\''s owner_id]:USER_SCOPE:_default' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
":: :_ironclaw__subcmd__traces__subcmd__profile__subcmd__help_commands" \
"*::: :->help" \
&& ret=0

    case $state in
    (help)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:ironclaw-traces-profile-help-command-$line[1]:"
        case $line[1] in
            (token)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(set)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(withdraw)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
        esac
    ;;
esac
;;
(help)
_arguments "${_arguments_options[@]}" : \
":: :_ironclaw__subcmd__traces__subcmd__help_commands" \
"*::: :->help" \
&& ret=0

    case $state in
    (help)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:ironclaw-traces-help-command-$line[1]:"
        case $line[1] in
            (opt-in)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(opt-out)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(enroll-instance)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(status)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(preview)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(enqueue)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(flush-queue)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(queue-status)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(credit)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(submit)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(list-submissions)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(revoke)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(ingest-health)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(profile)
_arguments "${_arguments_options[@]}" : \
":: :_ironclaw__subcmd__traces__subcmd__help__subcmd__profile_commands" \
"*::: :->profile" \
&& ret=0

    case $state in
    (profile)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:ironclaw-traces-help-profile-command-$line[1]:"
        case $line[1] in
            (token)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(set)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(withdraw)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
(help)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
        esac
    ;;
esac
;;
(help)
_arguments "${_arguments_options[@]}" : \
":: :_ironclaw__subcmd__help_commands" \
"*::: :->help" \
&& ret=0

    case $state in
    (help)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:ironclaw-help-command-$line[1]:"
        case $line[1] in
            (channels)
_arguments "${_arguments_options[@]}" : \
":: :_ironclaw__subcmd__help__subcmd__channels_commands" \
"*::: :->channels" \
&& ret=0

    case $state in
    (channels)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:ironclaw-help-channels-command-$line[1]:"
        case $line[1] in
            (list)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
(completion)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(config)
_arguments "${_arguments_options[@]}" : \
":: :_ironclaw__subcmd__help__subcmd__config_commands" \
"*::: :->config" \
&& ret=0

    case $state in
    (config)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:ironclaw-help-config-command-$line[1]:"
        case $line[1] in
            (path)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(init)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(list)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(get)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
(doctor)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(extension)
_arguments "${_arguments_options[@]}" : \
":: :_ironclaw__subcmd__help__subcmd__extension_commands" \
"*::: :->extension" \
&& ret=0

    case $state in
    (extension)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:ironclaw-help-extension-command-$line[1]:"
        case $line[1] in
            (search)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(install)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(activate)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(remove)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
(hooks)
_arguments "${_arguments_options[@]}" : \
":: :_ironclaw__subcmd__help__subcmd__hooks_commands" \
"*::: :->hooks" \
&& ret=0

    case $state in
    (hooks)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:ironclaw-help-hooks-command-$line[1]:"
        case $line[1] in
            (list)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
(logs)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(models)
_arguments "${_arguments_options[@]}" : \
":: :_ironclaw__subcmd__help__subcmd__models_commands" \
"*::: :->models" \
&& ret=0

    case $state in
    (models)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:ironclaw-help-models-command-$line[1]:"
        case $line[1] in
            (list)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(status)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(set)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(set-provider)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
(onboard)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(profile)
_arguments "${_arguments_options[@]}" : \
":: :_ironclaw__subcmd__help__subcmd__profile_commands" \
"*::: :->profile" \
&& ret=0

    case $state in
    (profile)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:ironclaw-help-profile-command-$line[1]:"
        case $line[1] in
            (list)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
(repl)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(run)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(serve)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(service)
_arguments "${_arguments_options[@]}" : \
":: :_ironclaw__subcmd__help__subcmd__service_commands" \
"*::: :->service" \
&& ret=0

    case $state in
    (service)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:ironclaw-help-service-command-$line[1]:"
        case $line[1] in
            (install)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(start)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(stop)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(restart)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(status)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(uninstall)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
(skills)
_arguments "${_arguments_options[@]}" : \
":: :_ironclaw__subcmd__help__subcmd__skills_commands" \
"*::: :->skills" \
&& ret=0

    case $state in
    (skills)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:ironclaw-help-skills-command-$line[1]:"
        case $line[1] in
            (list)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
(status)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(traces)
_arguments "${_arguments_options[@]}" : \
":: :_ironclaw__subcmd__help__subcmd__traces_commands" \
"*::: :->traces" \
&& ret=0

    case $state in
    (traces)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:ironclaw-help-traces-command-$line[1]:"
        case $line[1] in
            (opt-in)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(opt-out)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(enroll-instance)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(status)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(preview)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(enqueue)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(flush-queue)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(queue-status)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(credit)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(submit)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(list-submissions)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(revoke)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(ingest-health)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(profile)
_arguments "${_arguments_options[@]}" : \
":: :_ironclaw__subcmd__help__subcmd__traces__subcmd__profile_commands" \
"*::: :->profile" \
&& ret=0

    case $state in
    (profile)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:ironclaw-help-traces-profile-command-$line[1]:"
        case $line[1] in
            (token)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(set)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(withdraw)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
        esac
    ;;
esac
;;
(help)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
        esac
    ;;
esac
}

(( $+functions[_ironclaw_commands] )) ||
_ironclaw_commands() {
    local commands; commands=(
'channels:Inspect configured Reborn channels' \
'completion:Generate shell completion scripts' \
'config:Inspect Reborn configuration paths without creating state' \
'doctor:Check Reborn binary configuration without creating state' \
'extension:Manage local Reborn extension lifecycle' \
'hooks:Inspect configured Reborn hooks' \
'logs:Inspect Reborn logs' \
'models:Inspect Reborn model slots and route status' \
'onboard:Initialize the standalone Reborn home and first-run setup marker' \
'profile:Inspect supported Reborn boot profiles' \
'repl:Start the composed Reborn CLI REPL' \
'run:Initialize the minimal Reborn runtime shell and exit' \
'serve:Start the Reborn WebUI service. Available only when the binary is built with the \`webui-v2-beta\` Cargo feature; off by default because the beta HTTP/auth gateway requires explicit opt-in before being linked into a production binary' \
'service:Install/start/stop/status/uninstall the standalone Reborn binary as an OS-native service (launchd on macOS, systemd on Linux). Available only when built with the \`webui-v2-beta\` Cargo feature, since the installed unit runs \`serve\`' \
'skills:Inspect configured Reborn skills' \
'status:Show Reborn runtime status snapshot' \
'traces:Manage trace contributions to TraceCommons' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'ironclaw commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__channels_commands] )) ||
_ironclaw__subcmd__channels_commands() {
    local commands; commands=(
'list:List configured Reborn channels' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'ironclaw channels commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__channels__subcmd__help_commands] )) ||
_ironclaw__subcmd__channels__subcmd__help_commands() {
    local commands; commands=(
'list:List configured Reborn channels' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'ironclaw channels help commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__channels__subcmd__help__subcmd__help_commands] )) ||
_ironclaw__subcmd__channels__subcmd__help__subcmd__help_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw channels help help commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__channels__subcmd__help__subcmd__list_commands] )) ||
_ironclaw__subcmd__channels__subcmd__help__subcmd__list_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw channels help list commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__channels__subcmd__list_commands] )) ||
_ironclaw__subcmd__channels__subcmd__list_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw channels list commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__completion_commands] )) ||
_ironclaw__subcmd__completion_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw completion commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__config_commands] )) ||
_ironclaw__subcmd__config_commands() {
    local commands; commands=(
'path:Show resolved Reborn configuration paths without creating state' \
'init:Write a commented stub \`config.toml\` and \`providers.json\` into the Reborn home directory. Refuses to clobber unless --force' \
'list:List all configuration keys and their values' \
'get:Get a single configuration value by dot-separated key' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'ironclaw config commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__config__subcmd__get_commands] )) ||
_ironclaw__subcmd__config__subcmd__get_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw config get commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__config__subcmd__help_commands] )) ||
_ironclaw__subcmd__config__subcmd__help_commands() {
    local commands; commands=(
'path:Show resolved Reborn configuration paths without creating state' \
'init:Write a commented stub \`config.toml\` and \`providers.json\` into the Reborn home directory. Refuses to clobber unless --force' \
'list:List all configuration keys and their values' \
'get:Get a single configuration value by dot-separated key' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'ironclaw config help commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__config__subcmd__help__subcmd__get_commands] )) ||
_ironclaw__subcmd__config__subcmd__help__subcmd__get_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw config help get commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__config__subcmd__help__subcmd__help_commands] )) ||
_ironclaw__subcmd__config__subcmd__help__subcmd__help_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw config help help commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__config__subcmd__help__subcmd__init_commands] )) ||
_ironclaw__subcmd__config__subcmd__help__subcmd__init_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw config help init commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__config__subcmd__help__subcmd__list_commands] )) ||
_ironclaw__subcmd__config__subcmd__help__subcmd__list_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw config help list commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__config__subcmd__help__subcmd__path_commands] )) ||
_ironclaw__subcmd__config__subcmd__help__subcmd__path_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw config help path commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__config__subcmd__init_commands] )) ||
_ironclaw__subcmd__config__subcmd__init_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw config init commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__config__subcmd__list_commands] )) ||
_ironclaw__subcmd__config__subcmd__list_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw config list commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__config__subcmd__path_commands] )) ||
_ironclaw__subcmd__config__subcmd__path_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw config path commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__doctor_commands] )) ||
_ironclaw__subcmd__doctor_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw doctor commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__extension_commands] )) ||
_ironclaw__subcmd__extension_commands() {
    local commands; commands=(
'search:Search local Reborn extension packages' \
'install:Install a local Reborn extension package' \
'activate:Activate an installed local Reborn extension package' \
'remove:Remove an installed local Reborn extension package' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'ironclaw extension commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__extension__subcmd__activate_commands] )) ||
_ironclaw__subcmd__extension__subcmd__activate_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw extension activate commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__extension__subcmd__help_commands] )) ||
_ironclaw__subcmd__extension__subcmd__help_commands() {
    local commands; commands=(
'search:Search local Reborn extension packages' \
'install:Install a local Reborn extension package' \
'activate:Activate an installed local Reborn extension package' \
'remove:Remove an installed local Reborn extension package' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'ironclaw extension help commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__extension__subcmd__help__subcmd__activate_commands] )) ||
_ironclaw__subcmd__extension__subcmd__help__subcmd__activate_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw extension help activate commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__extension__subcmd__help__subcmd__help_commands] )) ||
_ironclaw__subcmd__extension__subcmd__help__subcmd__help_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw extension help help commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__extension__subcmd__help__subcmd__install_commands] )) ||
_ironclaw__subcmd__extension__subcmd__help__subcmd__install_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw extension help install commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__extension__subcmd__help__subcmd__remove_commands] )) ||
_ironclaw__subcmd__extension__subcmd__help__subcmd__remove_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw extension help remove commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__extension__subcmd__help__subcmd__search_commands] )) ||
_ironclaw__subcmd__extension__subcmd__help__subcmd__search_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw extension help search commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__extension__subcmd__install_commands] )) ||
_ironclaw__subcmd__extension__subcmd__install_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw extension install commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__extension__subcmd__remove_commands] )) ||
_ironclaw__subcmd__extension__subcmd__remove_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw extension remove commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__extension__subcmd__search_commands] )) ||
_ironclaw__subcmd__extension__subcmd__search_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw extension search commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__help_commands] )) ||
_ironclaw__subcmd__help_commands() {
    local commands; commands=(
'channels:Inspect configured Reborn channels' \
'completion:Generate shell completion scripts' \
'config:Inspect Reborn configuration paths without creating state' \
'doctor:Check Reborn binary configuration without creating state' \
'extension:Manage local Reborn extension lifecycle' \
'hooks:Inspect configured Reborn hooks' \
'logs:Inspect Reborn logs' \
'models:Inspect Reborn model slots and route status' \
'onboard:Initialize the standalone Reborn home and first-run setup marker' \
'profile:Inspect supported Reborn boot profiles' \
'repl:Start the composed Reborn CLI REPL' \
'run:Initialize the minimal Reborn runtime shell and exit' \
'serve:Start the Reborn WebUI service. Available only when the binary is built with the \`webui-v2-beta\` Cargo feature; off by default because the beta HTTP/auth gateway requires explicit opt-in before being linked into a production binary' \
'service:Install/start/stop/status/uninstall the standalone Reborn binary as an OS-native service (launchd on macOS, systemd on Linux). Available only when built with the \`webui-v2-beta\` Cargo feature, since the installed unit runs \`serve\`' \
'skills:Inspect configured Reborn skills' \
'status:Show Reborn runtime status snapshot' \
'traces:Manage trace contributions to TraceCommons' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'ironclaw help commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__help__subcmd__channels_commands] )) ||
_ironclaw__subcmd__help__subcmd__channels_commands() {
    local commands; commands=(
'list:List configured Reborn channels' \
    )
    _describe -t commands 'ironclaw help channels commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__help__subcmd__channels__subcmd__list_commands] )) ||
_ironclaw__subcmd__help__subcmd__channels__subcmd__list_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw help channels list commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__help__subcmd__completion_commands] )) ||
_ironclaw__subcmd__help__subcmd__completion_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw help completion commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__help__subcmd__config_commands] )) ||
_ironclaw__subcmd__help__subcmd__config_commands() {
    local commands; commands=(
'path:Show resolved Reborn configuration paths without creating state' \
'init:Write a commented stub \`config.toml\` and \`providers.json\` into the Reborn home directory. Refuses to clobber unless --force' \
'list:List all configuration keys and their values' \
'get:Get a single configuration value by dot-separated key' \
    )
    _describe -t commands 'ironclaw help config commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__help__subcmd__config__subcmd__get_commands] )) ||
_ironclaw__subcmd__help__subcmd__config__subcmd__get_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw help config get commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__help__subcmd__config__subcmd__init_commands] )) ||
_ironclaw__subcmd__help__subcmd__config__subcmd__init_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw help config init commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__help__subcmd__config__subcmd__list_commands] )) ||
_ironclaw__subcmd__help__subcmd__config__subcmd__list_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw help config list commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__help__subcmd__config__subcmd__path_commands] )) ||
_ironclaw__subcmd__help__subcmd__config__subcmd__path_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw help config path commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__help__subcmd__doctor_commands] )) ||
_ironclaw__subcmd__help__subcmd__doctor_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw help doctor commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__help__subcmd__extension_commands] )) ||
_ironclaw__subcmd__help__subcmd__extension_commands() {
    local commands; commands=(
'search:Search local Reborn extension packages' \
'install:Install a local Reborn extension package' \
'activate:Activate an installed local Reborn extension package' \
'remove:Remove an installed local Reborn extension package' \
    )
    _describe -t commands 'ironclaw help extension commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__help__subcmd__extension__subcmd__activate_commands] )) ||
_ironclaw__subcmd__help__subcmd__extension__subcmd__activate_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw help extension activate commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__help__subcmd__extension__subcmd__install_commands] )) ||
_ironclaw__subcmd__help__subcmd__extension__subcmd__install_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw help extension install commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__help__subcmd__extension__subcmd__remove_commands] )) ||
_ironclaw__subcmd__help__subcmd__extension__subcmd__remove_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw help extension remove commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__help__subcmd__extension__subcmd__search_commands] )) ||
_ironclaw__subcmd__help__subcmd__extension__subcmd__search_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw help extension search commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__help__subcmd__help_commands] )) ||
_ironclaw__subcmd__help__subcmd__help_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw help help commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__help__subcmd__hooks_commands] )) ||
_ironclaw__subcmd__help__subcmd__hooks_commands() {
    local commands; commands=(
'list:List configured Reborn hooks' \
    )
    _describe -t commands 'ironclaw help hooks commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__help__subcmd__hooks__subcmd__list_commands] )) ||
_ironclaw__subcmd__help__subcmd__hooks__subcmd__list_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw help hooks list commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__help__subcmd__logs_commands] )) ||
_ironclaw__subcmd__help__subcmd__logs_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw help logs commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__help__subcmd__models_commands] )) ||
_ironclaw__subcmd__help__subcmd__models_commands() {
    local commands; commands=(
'list:List Reborn LLM providers, or show one provider' \
'status:Show Reborn model route status' \
'set:Set the default Reborn model for the active provider' \
'set-provider:Set the default Reborn LLM provider' \
    )
    _describe -t commands 'ironclaw help models commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__help__subcmd__models__subcmd__list_commands] )) ||
_ironclaw__subcmd__help__subcmd__models__subcmd__list_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw help models list commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__help__subcmd__models__subcmd__set_commands] )) ||
_ironclaw__subcmd__help__subcmd__models__subcmd__set_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw help models set commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__help__subcmd__models__subcmd__set-provider_commands] )) ||
_ironclaw__subcmd__help__subcmd__models__subcmd__set-provider_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw help models set-provider commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__help__subcmd__models__subcmd__status_commands] )) ||
_ironclaw__subcmd__help__subcmd__models__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw help models status commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__help__subcmd__onboard_commands] )) ||
_ironclaw__subcmd__help__subcmd__onboard_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw help onboard commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__help__subcmd__profile_commands] )) ||
_ironclaw__subcmd__help__subcmd__profile_commands() {
    local commands; commands=(
'list:List supported Reborn boot profiles' \
    )
    _describe -t commands 'ironclaw help profile commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__help__subcmd__profile__subcmd__list_commands] )) ||
_ironclaw__subcmd__help__subcmd__profile__subcmd__list_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw help profile list commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__help__subcmd__repl_commands] )) ||
_ironclaw__subcmd__help__subcmd__repl_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw help repl commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__help__subcmd__run_commands] )) ||
_ironclaw__subcmd__help__subcmd__run_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw help run commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__help__subcmd__serve_commands] )) ||
_ironclaw__subcmd__help__subcmd__serve_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw help serve commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__help__subcmd__service_commands] )) ||
_ironclaw__subcmd__help__subcmd__service_commands() {
    local commands; commands=(
'install:Install the OS service (launchd on macOS, systemd on Linux)' \
'start:Start the installed service' \
'stop:Stop the running service' \
'restart:Restart the service\: stop then start if running, or just start if stopped. Errors if the service is not installed' \
'status:Show service status' \
'uninstall:Uninstall the OS service and remove the unit file' \
    )
    _describe -t commands 'ironclaw help service commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__help__subcmd__service__subcmd__install_commands] )) ||
_ironclaw__subcmd__help__subcmd__service__subcmd__install_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw help service install commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__help__subcmd__service__subcmd__restart_commands] )) ||
_ironclaw__subcmd__help__subcmd__service__subcmd__restart_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw help service restart commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__help__subcmd__service__subcmd__start_commands] )) ||
_ironclaw__subcmd__help__subcmd__service__subcmd__start_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw help service start commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__help__subcmd__service__subcmd__status_commands] )) ||
_ironclaw__subcmd__help__subcmd__service__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw help service status commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__help__subcmd__service__subcmd__stop_commands] )) ||
_ironclaw__subcmd__help__subcmd__service__subcmd__stop_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw help service stop commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__help__subcmd__service__subcmd__uninstall_commands] )) ||
_ironclaw__subcmd__help__subcmd__service__subcmd__uninstall_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw help service uninstall commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__help__subcmd__skills_commands] )) ||
_ironclaw__subcmd__help__subcmd__skills_commands() {
    local commands; commands=(
'list:List configured Reborn skills' \
    )
    _describe -t commands 'ironclaw help skills commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__help__subcmd__skills__subcmd__list_commands] )) ||
_ironclaw__subcmd__help__subcmd__skills__subcmd__list_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw help skills list commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__help__subcmd__status_commands] )) ||
_ironclaw__subcmd__help__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw help status commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__help__subcmd__traces_commands] )) ||
_ironclaw__subcmd__help__subcmd__traces_commands() {
    local commands; commands=(
'opt-in:Enable autonomous trace contribution after local redaction' \
'opt-out:Disable autonomous trace contribution. With --user-scope\: opt out ONLY that user (instance-level enrollment untouched). Without\: disable the global/instance policy AND the owner scope (full off switch)' \
'enroll-instance:Enroll this ENTIRE INSTANCE in Trace Commons with an operator invite link (admin operation — requires shell access to the instance host). Every user without a personal enrollment inherits it, attributed via a salted per-user pseudonym. Exclude a single user with \`traces opt-out --user-scope <tenant-id>/<user-id>\` (bare \`traces opt-out\` disables the entire instance enrollment)' \
'status:Show local standing trace contribution policy' \
'preview:Preview a redacted contribution envelope from a recorded trace file' \
'enqueue:Add an already-previewed envelope to the autonomous submission queue' \
'flush-queue:Submit eligible queued envelopes using the standing opt-in policy' \
'queue-status:Show local autonomous trace queue diagnostics' \
'credit:Show local credit totals and recent credit explanations' \
'submit:Submit an already-previewed redacted contribution envelope' \
'list-submissions:List local trace contribution submission records' \
'revoke:Revoke a trace contribution locally and, optionally, at an ingestion API' \
'ingest-health:Check a Trace Commons ingestion service /health endpoint' \
'profile:Manage the optional public community profile (second opt-in)' \
    )
    _describe -t commands 'ironclaw help traces commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__help__subcmd__traces__subcmd__credit_commands] )) ||
_ironclaw__subcmd__help__subcmd__traces__subcmd__credit_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw help traces credit commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__help__subcmd__traces__subcmd__enqueue_commands] )) ||
_ironclaw__subcmd__help__subcmd__traces__subcmd__enqueue_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw help traces enqueue commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__help__subcmd__traces__subcmd__enroll-instance_commands] )) ||
_ironclaw__subcmd__help__subcmd__traces__subcmd__enroll-instance_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw help traces enroll-instance commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__help__subcmd__traces__subcmd__flush-queue_commands] )) ||
_ironclaw__subcmd__help__subcmd__traces__subcmd__flush-queue_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw help traces flush-queue commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__help__subcmd__traces__subcmd__ingest-health_commands] )) ||
_ironclaw__subcmd__help__subcmd__traces__subcmd__ingest-health_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw help traces ingest-health commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__help__subcmd__traces__subcmd__list-submissions_commands] )) ||
_ironclaw__subcmd__help__subcmd__traces__subcmd__list-submissions_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw help traces list-submissions commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__help__subcmd__traces__subcmd__opt-in_commands] )) ||
_ironclaw__subcmd__help__subcmd__traces__subcmd__opt-in_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw help traces opt-in commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__help__subcmd__traces__subcmd__opt-out_commands] )) ||
_ironclaw__subcmd__help__subcmd__traces__subcmd__opt-out_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw help traces opt-out commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__help__subcmd__traces__subcmd__preview_commands] )) ||
_ironclaw__subcmd__help__subcmd__traces__subcmd__preview_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw help traces preview commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__help__subcmd__traces__subcmd__profile_commands] )) ||
_ironclaw__subcmd__help__subcmd__traces__subcmd__profile_commands() {
    local commands; commands=(
'token:Mint a short-lived profile token for the Trace Commons web profile page' \
'set:Create or update the public community profile' \
'withdraw:Withdraw the public community profile' \
    )
    _describe -t commands 'ironclaw help traces profile commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__help__subcmd__traces__subcmd__profile__subcmd__set_commands] )) ||
_ironclaw__subcmd__help__subcmd__traces__subcmd__profile__subcmd__set_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw help traces profile set commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__help__subcmd__traces__subcmd__profile__subcmd__token_commands] )) ||
_ironclaw__subcmd__help__subcmd__traces__subcmd__profile__subcmd__token_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw help traces profile token commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__help__subcmd__traces__subcmd__profile__subcmd__withdraw_commands] )) ||
_ironclaw__subcmd__help__subcmd__traces__subcmd__profile__subcmd__withdraw_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw help traces profile withdraw commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__help__subcmd__traces__subcmd__queue-status_commands] )) ||
_ironclaw__subcmd__help__subcmd__traces__subcmd__queue-status_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw help traces queue-status commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__help__subcmd__traces__subcmd__revoke_commands] )) ||
_ironclaw__subcmd__help__subcmd__traces__subcmd__revoke_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw help traces revoke commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__help__subcmd__traces__subcmd__status_commands] )) ||
_ironclaw__subcmd__help__subcmd__traces__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw help traces status commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__help__subcmd__traces__subcmd__submit_commands] )) ||
_ironclaw__subcmd__help__subcmd__traces__subcmd__submit_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw help traces submit commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__hooks_commands] )) ||
_ironclaw__subcmd__hooks_commands() {
    local commands; commands=(
'list:List configured Reborn hooks' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'ironclaw hooks commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__hooks__subcmd__help_commands] )) ||
_ironclaw__subcmd__hooks__subcmd__help_commands() {
    local commands; commands=(
'list:List configured Reborn hooks' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'ironclaw hooks help commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__hooks__subcmd__help__subcmd__help_commands] )) ||
_ironclaw__subcmd__hooks__subcmd__help__subcmd__help_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw hooks help help commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__hooks__subcmd__help__subcmd__list_commands] )) ||
_ironclaw__subcmd__hooks__subcmd__help__subcmd__list_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw hooks help list commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__hooks__subcmd__list_commands] )) ||
_ironclaw__subcmd__hooks__subcmd__list_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw hooks list commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__logs_commands] )) ||
_ironclaw__subcmd__logs_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw logs commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__models_commands] )) ||
_ironclaw__subcmd__models_commands() {
    local commands; commands=(
'list:List Reborn LLM providers, or show one provider' \
'status:Show Reborn model route status' \
'set:Set the default Reborn model for the active provider' \
'set-provider:Set the default Reborn LLM provider' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'ironclaw models commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__models__subcmd__help_commands] )) ||
_ironclaw__subcmd__models__subcmd__help_commands() {
    local commands; commands=(
'list:List Reborn LLM providers, or show one provider' \
'status:Show Reborn model route status' \
'set:Set the default Reborn model for the active provider' \
'set-provider:Set the default Reborn LLM provider' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'ironclaw models help commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__models__subcmd__help__subcmd__help_commands] )) ||
_ironclaw__subcmd__models__subcmd__help__subcmd__help_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw models help help commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__models__subcmd__help__subcmd__list_commands] )) ||
_ironclaw__subcmd__models__subcmd__help__subcmd__list_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw models help list commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__models__subcmd__help__subcmd__set_commands] )) ||
_ironclaw__subcmd__models__subcmd__help__subcmd__set_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw models help set commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__models__subcmd__help__subcmd__set-provider_commands] )) ||
_ironclaw__subcmd__models__subcmd__help__subcmd__set-provider_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw models help set-provider commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__models__subcmd__help__subcmd__status_commands] )) ||
_ironclaw__subcmd__models__subcmd__help__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw models help status commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__models__subcmd__list_commands] )) ||
_ironclaw__subcmd__models__subcmd__list_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw models list commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__models__subcmd__set_commands] )) ||
_ironclaw__subcmd__models__subcmd__set_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw models set commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__models__subcmd__set-provider_commands] )) ||
_ironclaw__subcmd__models__subcmd__set-provider_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw models set-provider commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__models__subcmd__status_commands] )) ||
_ironclaw__subcmd__models__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw models status commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__onboard_commands] )) ||
_ironclaw__subcmd__onboard_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw onboard commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__profile_commands] )) ||
_ironclaw__subcmd__profile_commands() {
    local commands; commands=(
'list:List supported Reborn boot profiles' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'ironclaw profile commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__profile__subcmd__help_commands] )) ||
_ironclaw__subcmd__profile__subcmd__help_commands() {
    local commands; commands=(
'list:List supported Reborn boot profiles' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'ironclaw profile help commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__profile__subcmd__help__subcmd__help_commands] )) ||
_ironclaw__subcmd__profile__subcmd__help__subcmd__help_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw profile help help commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__profile__subcmd__help__subcmd__list_commands] )) ||
_ironclaw__subcmd__profile__subcmd__help__subcmd__list_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw profile help list commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__profile__subcmd__list_commands] )) ||
_ironclaw__subcmd__profile__subcmd__list_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw profile list commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__repl_commands] )) ||
_ironclaw__subcmd__repl_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw repl commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__run_commands] )) ||
_ironclaw__subcmd__run_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw run commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__serve_commands] )) ||
_ironclaw__subcmd__serve_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw serve commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__service_commands] )) ||
_ironclaw__subcmd__service_commands() {
    local commands; commands=(
'install:Install the OS service (launchd on macOS, systemd on Linux)' \
'start:Start the installed service' \
'stop:Stop the running service' \
'restart:Restart the service\: stop then start if running, or just start if stopped. Errors if the service is not installed' \
'status:Show service status' \
'uninstall:Uninstall the OS service and remove the unit file' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'ironclaw service commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__service__subcmd__help_commands] )) ||
_ironclaw__subcmd__service__subcmd__help_commands() {
    local commands; commands=(
'install:Install the OS service (launchd on macOS, systemd on Linux)' \
'start:Start the installed service' \
'stop:Stop the running service' \
'restart:Restart the service\: stop then start if running, or just start if stopped. Errors if the service is not installed' \
'status:Show service status' \
'uninstall:Uninstall the OS service and remove the unit file' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'ironclaw service help commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__service__subcmd__help__subcmd__help_commands] )) ||
_ironclaw__subcmd__service__subcmd__help__subcmd__help_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw service help help commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__service__subcmd__help__subcmd__install_commands] )) ||
_ironclaw__subcmd__service__subcmd__help__subcmd__install_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw service help install commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__service__subcmd__help__subcmd__restart_commands] )) ||
_ironclaw__subcmd__service__subcmd__help__subcmd__restart_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw service help restart commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__service__subcmd__help__subcmd__start_commands] )) ||
_ironclaw__subcmd__service__subcmd__help__subcmd__start_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw service help start commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__service__subcmd__help__subcmd__status_commands] )) ||
_ironclaw__subcmd__service__subcmd__help__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw service help status commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__service__subcmd__help__subcmd__stop_commands] )) ||
_ironclaw__subcmd__service__subcmd__help__subcmd__stop_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw service help stop commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__service__subcmd__help__subcmd__uninstall_commands] )) ||
_ironclaw__subcmd__service__subcmd__help__subcmd__uninstall_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw service help uninstall commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__service__subcmd__install_commands] )) ||
_ironclaw__subcmd__service__subcmd__install_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw service install commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__service__subcmd__restart_commands] )) ||
_ironclaw__subcmd__service__subcmd__restart_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw service restart commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__service__subcmd__start_commands] )) ||
_ironclaw__subcmd__service__subcmd__start_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw service start commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__service__subcmd__status_commands] )) ||
_ironclaw__subcmd__service__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw service status commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__service__subcmd__stop_commands] )) ||
_ironclaw__subcmd__service__subcmd__stop_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw service stop commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__service__subcmd__uninstall_commands] )) ||
_ironclaw__subcmd__service__subcmd__uninstall_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw service uninstall commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__skills_commands] )) ||
_ironclaw__subcmd__skills_commands() {
    local commands; commands=(
'list:List configured Reborn skills' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'ironclaw skills commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__skills__subcmd__help_commands] )) ||
_ironclaw__subcmd__skills__subcmd__help_commands() {
    local commands; commands=(
'list:List configured Reborn skills' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'ironclaw skills help commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__skills__subcmd__help__subcmd__help_commands] )) ||
_ironclaw__subcmd__skills__subcmd__help__subcmd__help_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw skills help help commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__skills__subcmd__help__subcmd__list_commands] )) ||
_ironclaw__subcmd__skills__subcmd__help__subcmd__list_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw skills help list commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__skills__subcmd__list_commands] )) ||
_ironclaw__subcmd__skills__subcmd__list_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw skills list commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__status_commands] )) ||
_ironclaw__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw status commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__traces_commands] )) ||
_ironclaw__subcmd__traces_commands() {
    local commands; commands=(
'opt-in:Enable autonomous trace contribution after local redaction' \
'opt-out:Disable autonomous trace contribution. With --user-scope\: opt out ONLY that user (instance-level enrollment untouched). Without\: disable the global/instance policy AND the owner scope (full off switch)' \
'enroll-instance:Enroll this ENTIRE INSTANCE in Trace Commons with an operator invite link (admin operation — requires shell access to the instance host). Every user without a personal enrollment inherits it, attributed via a salted per-user pseudonym. Exclude a single user with \`traces opt-out --user-scope <tenant-id>/<user-id>\` (bare \`traces opt-out\` disables the entire instance enrollment)' \
'status:Show local standing trace contribution policy' \
'preview:Preview a redacted contribution envelope from a recorded trace file' \
'enqueue:Add an already-previewed envelope to the autonomous submission queue' \
'flush-queue:Submit eligible queued envelopes using the standing opt-in policy' \
'queue-status:Show local autonomous trace queue diagnostics' \
'credit:Show local credit totals and recent credit explanations' \
'submit:Submit an already-previewed redacted contribution envelope' \
'list-submissions:List local trace contribution submission records' \
'revoke:Revoke a trace contribution locally and, optionally, at an ingestion API' \
'ingest-health:Check a Trace Commons ingestion service /health endpoint' \
'profile:Manage the optional public community profile (second opt-in)' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'ironclaw traces commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__traces__subcmd__credit_commands] )) ||
_ironclaw__subcmd__traces__subcmd__credit_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw traces credit commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__traces__subcmd__enqueue_commands] )) ||
_ironclaw__subcmd__traces__subcmd__enqueue_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw traces enqueue commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__traces__subcmd__enroll-instance_commands] )) ||
_ironclaw__subcmd__traces__subcmd__enroll-instance_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw traces enroll-instance commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__traces__subcmd__flush-queue_commands] )) ||
_ironclaw__subcmd__traces__subcmd__flush-queue_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw traces flush-queue commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__traces__subcmd__help_commands] )) ||
_ironclaw__subcmd__traces__subcmd__help_commands() {
    local commands; commands=(
'opt-in:Enable autonomous trace contribution after local redaction' \
'opt-out:Disable autonomous trace contribution. With --user-scope\: opt out ONLY that user (instance-level enrollment untouched). Without\: disable the global/instance policy AND the owner scope (full off switch)' \
'enroll-instance:Enroll this ENTIRE INSTANCE in Trace Commons with an operator invite link (admin operation — requires shell access to the instance host). Every user without a personal enrollment inherits it, attributed via a salted per-user pseudonym. Exclude a single user with \`traces opt-out --user-scope <tenant-id>/<user-id>\` (bare \`traces opt-out\` disables the entire instance enrollment)' \
'status:Show local standing trace contribution policy' \
'preview:Preview a redacted contribution envelope from a recorded trace file' \
'enqueue:Add an already-previewed envelope to the autonomous submission queue' \
'flush-queue:Submit eligible queued envelopes using the standing opt-in policy' \
'queue-status:Show local autonomous trace queue diagnostics' \
'credit:Show local credit totals and recent credit explanations' \
'submit:Submit an already-previewed redacted contribution envelope' \
'list-submissions:List local trace contribution submission records' \
'revoke:Revoke a trace contribution locally and, optionally, at an ingestion API' \
'ingest-health:Check a Trace Commons ingestion service /health endpoint' \
'profile:Manage the optional public community profile (second opt-in)' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'ironclaw traces help commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__traces__subcmd__help__subcmd__credit_commands] )) ||
_ironclaw__subcmd__traces__subcmd__help__subcmd__credit_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw traces help credit commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__traces__subcmd__help__subcmd__enqueue_commands] )) ||
_ironclaw__subcmd__traces__subcmd__help__subcmd__enqueue_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw traces help enqueue commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__traces__subcmd__help__subcmd__enroll-instance_commands] )) ||
_ironclaw__subcmd__traces__subcmd__help__subcmd__enroll-instance_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw traces help enroll-instance commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__traces__subcmd__help__subcmd__flush-queue_commands] )) ||
_ironclaw__subcmd__traces__subcmd__help__subcmd__flush-queue_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw traces help flush-queue commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__traces__subcmd__help__subcmd__help_commands] )) ||
_ironclaw__subcmd__traces__subcmd__help__subcmd__help_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw traces help help commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__traces__subcmd__help__subcmd__ingest-health_commands] )) ||
_ironclaw__subcmd__traces__subcmd__help__subcmd__ingest-health_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw traces help ingest-health commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__traces__subcmd__help__subcmd__list-submissions_commands] )) ||
_ironclaw__subcmd__traces__subcmd__help__subcmd__list-submissions_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw traces help list-submissions commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__traces__subcmd__help__subcmd__opt-in_commands] )) ||
_ironclaw__subcmd__traces__subcmd__help__subcmd__opt-in_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw traces help opt-in commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__traces__subcmd__help__subcmd__opt-out_commands] )) ||
_ironclaw__subcmd__traces__subcmd__help__subcmd__opt-out_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw traces help opt-out commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__traces__subcmd__help__subcmd__preview_commands] )) ||
_ironclaw__subcmd__traces__subcmd__help__subcmd__preview_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw traces help preview commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__traces__subcmd__help__subcmd__profile_commands] )) ||
_ironclaw__subcmd__traces__subcmd__help__subcmd__profile_commands() {
    local commands; commands=(
'token:Mint a short-lived profile token for the Trace Commons web profile page' \
'set:Create or update the public community profile' \
'withdraw:Withdraw the public community profile' \
    )
    _describe -t commands 'ironclaw traces help profile commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__traces__subcmd__help__subcmd__profile__subcmd__set_commands] )) ||
_ironclaw__subcmd__traces__subcmd__help__subcmd__profile__subcmd__set_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw traces help profile set commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__traces__subcmd__help__subcmd__profile__subcmd__token_commands] )) ||
_ironclaw__subcmd__traces__subcmd__help__subcmd__profile__subcmd__token_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw traces help profile token commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__traces__subcmd__help__subcmd__profile__subcmd__withdraw_commands] )) ||
_ironclaw__subcmd__traces__subcmd__help__subcmd__profile__subcmd__withdraw_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw traces help profile withdraw commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__traces__subcmd__help__subcmd__queue-status_commands] )) ||
_ironclaw__subcmd__traces__subcmd__help__subcmd__queue-status_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw traces help queue-status commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__traces__subcmd__help__subcmd__revoke_commands] )) ||
_ironclaw__subcmd__traces__subcmd__help__subcmd__revoke_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw traces help revoke commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__traces__subcmd__help__subcmd__status_commands] )) ||
_ironclaw__subcmd__traces__subcmd__help__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw traces help status commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__traces__subcmd__help__subcmd__submit_commands] )) ||
_ironclaw__subcmd__traces__subcmd__help__subcmd__submit_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw traces help submit commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__traces__subcmd__ingest-health_commands] )) ||
_ironclaw__subcmd__traces__subcmd__ingest-health_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw traces ingest-health commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__traces__subcmd__list-submissions_commands] )) ||
_ironclaw__subcmd__traces__subcmd__list-submissions_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw traces list-submissions commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__traces__subcmd__opt-in_commands] )) ||
_ironclaw__subcmd__traces__subcmd__opt-in_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw traces opt-in commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__traces__subcmd__opt-out_commands] )) ||
_ironclaw__subcmd__traces__subcmd__opt-out_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw traces opt-out commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__traces__subcmd__preview_commands] )) ||
_ironclaw__subcmd__traces__subcmd__preview_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw traces preview commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__traces__subcmd__profile_commands] )) ||
_ironclaw__subcmd__traces__subcmd__profile_commands() {
    local commands; commands=(
'token:Mint a short-lived profile token for the Trace Commons web profile page' \
'set:Create or update the public community profile' \
'withdraw:Withdraw the public community profile' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'ironclaw traces profile commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__traces__subcmd__profile__subcmd__help_commands] )) ||
_ironclaw__subcmd__traces__subcmd__profile__subcmd__help_commands() {
    local commands; commands=(
'token:Mint a short-lived profile token for the Trace Commons web profile page' \
'set:Create or update the public community profile' \
'withdraw:Withdraw the public community profile' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'ironclaw traces profile help commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__traces__subcmd__profile__subcmd__help__subcmd__help_commands] )) ||
_ironclaw__subcmd__traces__subcmd__profile__subcmd__help__subcmd__help_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw traces profile help help commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__traces__subcmd__profile__subcmd__help__subcmd__set_commands] )) ||
_ironclaw__subcmd__traces__subcmd__profile__subcmd__help__subcmd__set_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw traces profile help set commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__traces__subcmd__profile__subcmd__help__subcmd__token_commands] )) ||
_ironclaw__subcmd__traces__subcmd__profile__subcmd__help__subcmd__token_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw traces profile help token commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__traces__subcmd__profile__subcmd__help__subcmd__withdraw_commands] )) ||
_ironclaw__subcmd__traces__subcmd__profile__subcmd__help__subcmd__withdraw_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw traces profile help withdraw commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__traces__subcmd__profile__subcmd__set_commands] )) ||
_ironclaw__subcmd__traces__subcmd__profile__subcmd__set_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw traces profile set commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__traces__subcmd__profile__subcmd__token_commands] )) ||
_ironclaw__subcmd__traces__subcmd__profile__subcmd__token_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw traces profile token commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__traces__subcmd__profile__subcmd__withdraw_commands] )) ||
_ironclaw__subcmd__traces__subcmd__profile__subcmd__withdraw_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw traces profile withdraw commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__traces__subcmd__queue-status_commands] )) ||
_ironclaw__subcmd__traces__subcmd__queue-status_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw traces queue-status commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__traces__subcmd__revoke_commands] )) ||
_ironclaw__subcmd__traces__subcmd__revoke_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw traces revoke commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__traces__subcmd__status_commands] )) ||
_ironclaw__subcmd__traces__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw traces status commands' commands "$@"
}
(( $+functions[_ironclaw__subcmd__traces__subcmd__submit_commands] )) ||
_ironclaw__subcmd__traces__subcmd__submit_commands() {
    local commands; commands=()
    _describe -t commands 'ironclaw traces submit commands' commands "$@"
}

if [ "$funcstack[1]" = "_ironclaw" ]; then
    _ironclaw "$@"
else
    (( $+functions[compdef] )) && compdef _ironclaw ironclaw
fi
