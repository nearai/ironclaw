# Sandbox egress spike — Step 0 results (#7732)

**Date:** 2026-08-19 · **Status:** spike complete, all 9 items evidenced · **Environment:** macOS + OrbStack, Docker server 29.4.0

Throwaway spike proving the per-thread sandbox topology from
[#7732](https://github.com/nearai/ironclaw/issues/7732): a command container on
an `--internal` Docker network whose only route out is a dual-homed
`iron-proxy` sidecar. No Rust changes; stock containers only.

**Pinned proxy image:**
`ironsh/iron-proxy@sha256:c4628019c24f4cc8d77564a26b7c9cedb00accee6f93d06270e85fb8f9c6a7da`

### Recorded image identities and evidence limits

| Role | Immutable identity recorded during the spike |
|---|---|
| iron-proxy | `ironsh/iron-proxy@sha256:c4628019c24f4cc8d77564a26b7c9cedb00accee6f93d06270e85fb8f9c6a7da` |
| HTTP echo fixture | `mendhak/http-https-echo@sha256:2046be25f4a2c0bdda662ebfb7c2b7b60fc95c31d97987be143645a8a2194a40` |
| curl fixture | `curlimages/curl@sha256:7c12af72ceb38b7432ab85e1a265cff6ae58e06f95539d539b654f2cfa64bb13` |
| Python/Node runtime | `nikolaik/python-nodejs@sha256:88c41488c175453b29007809b82c3059c9a55b721f14f5a5a4ea64cb995e26e7` |

Historical command blocks below remain exactly what was run. The spike did
**not** record immutable runtime identities for `alpine:3.20` or
`alpine/openssl:latest`; exact reproduction of those two inputs is therefore
an evidence gap, not something to reconstruct from today's registry state.
Future spikes must record `RepoDigests` before execution. The value
`real-secret-value-42` is a deterministic local substitution canary, not an
operative credential.

## Verdicts

| # | Item | Verdict | Evidence |
|---|---|---|---|
| 1 | Internal net + dual-homed proxy topology | PASS | [results-topology-dns.md](results-topology-dns.md) |
| 2 | `--dns <proxy>` takes effect through Docker embedded DNS | PASS | same |
| 3 | Allowlisted TLS egress through MITM (spike CA) | PASS | [results-policy-credentials.md](results-policy-credentials.md) |
| 4 | Default-deny 403 + structured JSON audit | PASS | same + [audit-denial-example.jsonl](audit-denial-example.jsonl) |
| 5 | Direct egress dead (public, metadata IP, no default route) | PASS | results-topology-dns.md |
| 6 | Placeholder→real credential swap, `require: true`, host binding, exact literal absent from captured logs | PASS (4/4 sub-checks) | results-policy-credentials.md |
| 7 | Per-runtime TLS trust matrix (curl/git/node/python/pip) | PASS | [results-tls-trust.md](results-tls-trust.md) |
| 8 | `docker exec` stream/kill/zombie/latency mechanics | PASS with findings | [results-exec-mechanics.md](results-exec-mechanics.md) |
| 9 | No IPv6 route off the internal network | PASS | results-topology-dns.md |

## Key findings for Step 1 implementation

1. **DNS**: `/etc/resolv.conf` in the client shows `127.0.0.11` (embedded DNS),
   but `--dns <proxy-ip>` is honored as the forwarding target — every name
   (including `.invalid`) resolves to the proxy IP. The topology works as
   designed on user-defined networks.
2. **Exec stream**: the feared indefinite hang from a backgrounded child
   inheriting stdout did **not** reproduce on Docker 29.4 — it costs ~2 s
   instead. **Redirection alone** (`>/dev/null 2>&1 </dev/null`) closes the
   stream immediately (0.07 s); `setsid` is needed only for kill-tree
   isolation, not stream closure. `ironclaw-exec` therefore needs **both**:
   redirection for latency, setsid for group-kill.
3. **Group kill works from a sibling exec** (BusyBox syntax: `kill -TERM
   -<PGID>`, no `--`), but a **non-reaping PID 1 leaves zombies** — the worker
   image needs tini or an equivalent reaping init.
4. **Exec latency**: 30–60 ms observed range, 30 ms median, 38 ms mean —
   confirms per-thread container + exec-per-command economics.
5. **TLS trust is per-runtime**: system store (`update-ca-certificates`)
   covers curl/git/python-stdlib and (surprisingly) pip 26.x; **Node ignores
   the system store** and requires `NODE_EXTRA_CA_CERTS`. The worker image must
   verify that this path exists. Do not globally point `SSL_CERT_FILE` at the
   proxy-only CA; use a merged CA bundle or scope that override to the runtime
   that needs it.
6. **Credential swap verified end-to-end**: placeholder swapped only at the
   bound host; unswapped when sent to a non-bound host; requests to the bound
   host without the placeholder are rejected (`require: true`). A literal,
   case-sensitive search found the exact real value zero times in captured
   proxy logs; encoded, escaped, or transformed representations were not tested.
7. **A shallow `git clone` of a public repo contacted only `github.com`** —
   the full asset-host constellation (codeload, objects.githubusercontent)
   was allowlisted but unused in this fixture; deeper clones/LFS will need it.
8. **IPv6**: `EnableIPv6=false` on the spike networks; no v6 route exists off
   the internal network. Docker 29.4 exposes no `.IPv6` field to `docker info`
   templates — check per-network `EnableIPv6` instead.

## Fixture files

- [proxy.credentials.yaml](proxy.credentials.yaml) — working config with
  allowlist + `secrets` transform (`require: true`, host-bound placeholder).
  `upstream_deny_cidrs: []` is a **test-only** override for the private echo
  upstream; production keeps the default deny.
- [proxy.allowlist.yaml](proxy.allowlist.yaml) — working allowlist-only config
  with the github/pypi host set used by the TLS matrix.
- [audit-denial-example.jsonl](audit-denial-example.jsonl) — the shape of a
  structured denial audit record (`rejected_by: allowlist`, transform trace).

CA material was generated per-run and deliberately **not** committed.

## Reproduction

Standard recipe (unique subnets per concurrent instance):

```bash
docker network create --internal --subnet 172.28.N.0/24 <pfx>-int
docker network create --subnet 172.29.N.0/24 <pfx>-egress
docker run -d --name <pfx>-proxy --network <pfx>-int --ip 172.28.N.2 \
  -v $DIR/proxy.yaml:/etc/iron-proxy/proxy.yaml:ro \
  -v $DIR/ca.crt:/etc/iron-proxy/ca.crt:ro -v $DIR/ca.key:/etc/iron-proxy/ca.key:ro \
  ironsh/iron-proxy@sha256:c4628019c24f4cc8d77564a26b7c9cedb00accee6f93d06270e85fb8f9c6a7da \
  -config /etc/iron-proxy/proxy.yaml
docker network connect <pfx>-egress <pfx>-proxy
docker run -d --name <pfx>-client --network <pfx>-int --dns 172.28.N.2 alpine:3.20 sleep 900 # historical tag; immutable digest was not recorded
```

CA generation (LibreSSL on macOS hosts misbehaves — use the container):

```bash
docker run --rm -v $DIR:/work -w /work alpine/openssl genrsa -out ca.key 4096
docker run --rm -v $DIR:/work -w /work alpine/openssl req -x509 -new -nodes \
  -key ca.key -sha256 -days 3650 -subj /CN=iron-proxy-spike-CA \
  -addext basicConstraints=critical,CA:TRUE -addext keyUsage=critical,keyCertSign \
  -out ca.crt
```
