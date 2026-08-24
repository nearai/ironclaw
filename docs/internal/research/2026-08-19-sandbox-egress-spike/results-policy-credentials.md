# Spike 7732 — Task B Results

Environment: macOS host, OrbStack Docker server 29.4.0. Resource prefix: `s7b-`.

Final verdicts:

- Item 3: **PASS**
- Item 4: **PASS**
- Item 6: **PASS** (all four sub-checks passed)

The echo service used `HTTP_PORT=80`, so the proxy could use the normal HTTP upstream port. The `upstream_deny_cidrs: []` override in `proxy.yaml` is test-only. It lets the proxy reach the private echo-container address and must not become a production default.

Image evidence: the spike recorded immutable pull digests for
`mendhak/http-https-echo` (`sha256:2046be25…`) and `curlimages/curl`
(`sha256:7c12af72…`), listed beside their historical pull commands below.
It did not record immutable identities for `alpine:3.20` or
`alpine/openssl:latest`; those command blocks remain truthful historical
evidence rather than being rewritten with a digest fetched later. The value
`real-secret-value-42` is a deterministic local substitution canary, not an
operative credential.

## Setup

Cleanup ran before all other work:

```sh
docker rm -f s7b-proxy s7b-client s7b-echo 2>/dev/null; docker network rm s7b-int s7b-egress 2>/dev/null; rm -rf /tmp/spike-7732/taskB; mkdir -p /tmp/spike-7732/taskB
```

Output: empty (no old assigned resources remained).

Images:

```sh
docker pull mendhak/http-https-echo:latest
```

```text
Digest: sha256:2046be25f4a2c0bdda662ebfb7c2b7b60fc95c31d97987be143645a8a2194a40
Status: Downloaded newer image for mendhak/http-https-echo:latest
```

```sh
docker pull curlimages/curl:latest
```

```text
Digest: sha256:7c12af72ceb38b7432ab85e1a265cff6ae58e06f95539d539b654f2cfa64bb13
Status: Downloaded newer image for curlimages/curl:latest
```

CA generation used the containerized OpenSSL implementation, not host LibreSSL:

```sh
docker run --rm -v /tmp/spike-7732/taskB:/work -w /work alpine/openssl:latest genrsa -out ca.key 4096
docker run --rm -v /tmp/spike-7732/taskB:/work -w /work alpine/openssl:latest req -x509 -new -nodes -key ca.key -sha256 -days 3650 -subj /CN=iron-proxy-spike-CA -addext basicConstraints=critical,CA:TRUE -addext keyUsage=critical,keyCertSign -out ca.crt
```

Output: empty; both commands exited successfully. Artifacts: `ca.key`, `ca.crt`.

The exact working configuration is saved as `/tmp/spike-7732/taskB/proxy.yaml`.

Topology commands:

```sh
docker network create --internal --subnet 172.28.20.0/24 s7b-int
docker network create --subnet 172.29.20.0/24 s7b-egress
docker run -d --name s7b-echo --network s7b-egress --network-alias capture.test --network-alias other.test -e HTTP_PORT=80 mendhak/http-https-echo:latest
```

```text
63efee5fb11c95f2b67c83889eb64c2f02eb81d94bcb7c1b288ca2efc37bbde4
9e4ae66a20111a119e7dc1d2d781702c330bd13059f33b43d0d72fb8fdc28341
09c995dfca9a6ee255cb168feb4ec27cf2921eb60647bab31ed962d71c399841
```

```sh
docker run -d --name s7b-proxy --network s7b-int --ip 172.28.20.2 \
  -v /tmp/spike-7732/taskB/proxy.yaml:/etc/iron-proxy/proxy.yaml:ro \
  -v /tmp/spike-7732/taskB/ca.crt:/etc/iron-proxy/ca.crt:ro \
  -v /tmp/spike-7732/taskB/ca.key:/etc/iron-proxy/ca.key:ro \
  -e REAL_TOKEN=real-secret-value-42 \
  ironsh/iron-proxy@sha256:c4628019c24f4cc8d77564a26b7c9cedb00accee6f93d06270e85fb8f9c6a7da \
  -config /etc/iron-proxy/proxy.yaml
docker network connect s7b-egress s7b-proxy
```

```text
be939534e5e8a831a9a22fc82c293c2b6b9a8db2344794aeae3163eba721ae8b
```

Startup check:

```sh
docker logs s7b-proxy
docker inspect -f '{{.State.Status}}' s7b-proxy
```

```text
{"level":"INFO","msg":"iron-proxy starting","dns_listen":":53","http_listen":":80","https_listen":":443"}
{"level":"INFO","msg":"transform pipeline","transforms":"allowlist → secrets"}
{"level":"INFO","msg":"dns server starting","addr":":53"}
{"level":"INFO","msg":"https proxy starting","addr":"[::]:443"}
{"level":"INFO","msg":"http proxy starting","addr":"[::]:80"}
running
```

```sh
docker run -d --name s7b-client --network s7b-int --dns 172.28.20.2 alpine:3.20 sleep 900
```

```text
37ef748d6f45cbf42e45112e42639ea5e9302a0fce300195fe5e65724c062a23
```

## Item 3 — PASS: allowlisted TLS egress

Command (also rerun through `tee` to save `item3-status.txt`):

```sh
docker run --rm --network s7b-int --dns 172.28.20.2 \
  -v /tmp/spike-7732/taskB/ca.crt:/ca.crt:ro \
  curlimages/curl:latest -sS --cacert /ca.crt \
  https://example.com -o /dev/null -w '%{http_code}'
```

```text
200
```

MITM certificate evidence command (full output saved in `item3-verbose.txt`):

```sh
docker run --rm --network s7b-int --dns 172.28.20.2 \
  -v /tmp/spike-7732/taskB/ca.crt:/ca.crt:ro \
  curlimages/curl:latest -sS -v --cacert /ca.crt \
  https://example.com -o /dev/null 2>&1 | tee /tmp/spike-7732/taskB/item3-verbose.txt
```

Trimmed output:

```text
* Host example.com:443 was resolved.
* IPv4: 172.28.20.2
* Server certificate:
*   subject: CN=example.com
*   issuer: CN=iron-proxy-spike-CA
*   subjectAltName: "example.com" matches cert's "example.com"
* OpenSSL verify result: 0
< HTTP/1.1 200 OK
```

This proves DNS sent the client to iron-proxy, the proxy presented a leaf certificate issued by the spike CA, curl trusted that CA, and the allowlisted upstream returned 200.

## Item 4 — PASS: default deny and audit

Status command (also rerun through `tee` to save `item4-status.txt`):

```sh
docker run --rm --network s7b-int --dns 172.28.20.2 \
  -v /tmp/spike-7732/taskB/ca.crt:/ca.crt:ro \
  curlimages/curl:latest -sS --cacert /ca.crt \
  https://denied.invalid/ -o /dev/null -w '%{http_code}'
```

```text
403
```

A verbose rerun saved to `item4-verbose.txt` proved this was a proxy denial rather than a DNS failure:

```sh
docker run --rm --network s7b-int --dns 172.28.20.2 \
  -v /tmp/spike-7732/taskB/ca.crt:/ca.crt:ro \
  curlimages/curl:latest -sS -v --cacert /ca.crt \
  https://denied.invalid/ -o /dev/null 2>&1 | tee /tmp/spike-7732/taskB/item4-verbose.txt
```

```text
* Host denied.invalid:443 was resolved.
* IPv4: 172.28.20.2
* Established connection to denied.invalid (172.28.20.2 port 443)
< HTTP/1.1 403 Forbidden
```

Audit capture command:

```sh
docker logs s7b-proxy 2>&1 | tee /tmp/spike-7732/taskB/proxy-logs-after-denial.jsonl
```

Rejected-request evidence (also saved alone in `item4-denial-audit.jsonl`):

```json
{"time":"2026-08-19T11:26:00.564519806Z","level":"WARN","msg":"request","audit":{"host":"denied.invalid","method":"GET","path":"/","remote_addr":"172.28.20.4:34558","sni":"denied.invalid","mode":"mitm","action":"reject","status_code":403,"duration_ms":0.005},"rejected_by":"allowlist","request_transforms":[{"name":"allowlist","action":"reject","duration_ms":0.001}]}
```

The line records `action: reject`, status 403, `rejected_by: allowlist`, and the rejecting transform trace.

## Item 6 — PASS: placeholder credential swap

All four required sub-checks passed.

### 6a — PASS: placeholder swaps only for the bound host

Command (rerun through `tee` to save `item6a-echo.json`):

```sh
docker run --rm --network s7b-int --dns 172.28.20.2 \
  curlimages/curl:latest -sS \
  -H 'Authorization: Bearer icsbx_spike_placeholder_123' \
  http://capture.test/
```

Trimmed echoed JSON:

```json
{
  "headers": {
    "host": "capture.test",
    "authorization": "Bearer real-secret-value-42"
  },
  "hostname": "capture.test",
  "protocol": "http"
}
```

A literal search of `item6a-echo.json` found `real-secret-value-42` on the authorization line and found no occurrence of `icsbx_spike_placeholder_123`.

### 6b — PASS: `require: true` rejects an alternate token

Command (also rerun through `tee` to save `item6b-status.txt`):

```sh
docker run --rm --network s7b-int --dns 172.28.20.2 \
  curlimages/curl:latest -sS \
  -H 'Authorization: Bearer some-other-token' \
  http://capture.test/ -o /dev/null -w '%{http_code}'
```

```text
403
```

The proxy audit additionally recorded `rejected_by: secrets` and a secrets-transform rejection annotation for `REAL_TOKEN`.

### 6c — PASS: the same placeholder stays unchanged on a non-bound allowlisted host

Command (output saved to `item6c-echo.json`):

```sh
docker run --rm --network s7b-int --dns 172.28.20.2 \
  curlimages/curl:latest -sS \
  -H 'Authorization: Bearer icsbx_spike_placeholder_123' \
  http://other.test/ | tee /tmp/spike-7732/taskB/item6c-echo.json
```

Trimmed echoed JSON:

```json
{
  "headers": {
    "host": "other.test",
    "authorization": "Bearer icsbx_spike_placeholder_123"
  },
  "hostname": "other.test",
  "protocol": "http"
}
```

### 6d — PASS: real credential never appears in proxy logs

Final log capture after all requests:

```sh
docker logs s7b-proxy 2>&1 | tee /tmp/spike-7732/taskB/proxy-logs-final.jsonl
```

A literal, case-sensitive search of the saved complete log for `real-secret-value-42` returned no matches. Count: `0` (saved in `item6d-log-count.txt`). The successful swap audit instead names only the source variable and location:

```json
{"name":"secrets","action":"allow","annotations":{"swapped":[{"secret":"REAL_TOKEN","locations":["header:Authorization"]}]}}
```

## Cleanup — PASS

Commands:

```sh
docker rm -f s7b-proxy s7b-client s7b-echo
docker network rm s7b-int s7b-egress
```

```text
s7b-proxy
s7b-client
s7b-echo
s7b-int
s7b-egress
```

Verification:

```sh
docker ps -a --filter 'name=^/s7b-' --format '{{.Names}}'
docker network ls --filter 'name=^s7b-(int|egress)$' --format '{{.Name}}'
```

Output: empty. All assigned containers and networks were removed. `/tmp/spike-7732/taskB/` and its evidence files were kept.
