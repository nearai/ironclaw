# Spike item 7: per-runtime TLS trust through iron-proxy

**Overall verdict: PASS**

The command container had only `s7c-int` attached. `s7c-proxy` was dual-homed on `s7c-int` and `s7c-egress`. All successful HTTPS requests were intercepted by iron-proxy and validated with the spike CA.

## Environment and setup

- Runtime image: `nikolaik/python-nodejs:python3.12-nodejs22`
- Pulled image digest: `sha256:88c41488c175453b29007809b82c3059c9a55b721f14f5a5a4ea64cb995e26e7`
- Proxy image: `ironsh/iron-proxy@sha256:c4628019c24f4cc8d77564a26b7c9cedb00accee6f93d06270e85fb8f9c6a7da`
- Runtime versions: curl 8.14.1, git 2.47.3, Node v22.23.2, Python 3.12.13, pip 26.1.2.

For immutable replay, the recorded runtime identity is
`nikolaik/python-nodejs@sha256:88c41488c175453b29007809b82c3059c9a55b721f14f5a5a4ea64cb995e26e7`.
The exact setup block below intentionally retains the tag that was actually
executed. The Alpine OpenSSL helper's runtime digest was not recorded.

Exact setup commands:

```sh
docker rm -f s7c-proxy s7c-client 2>/dev/null; docker network rm s7c-int s7c-egress 2>/dev/null; rm -rf /tmp/spike-7732/taskC; mkdir -p /tmp/spike-7732/taskC
docker pull nikolaik/python-nodejs:python3.12-nodejs22
docker run --rm -v /tmp/spike-7732/taskC:/work -w /work alpine/openssl:latest genrsa -out ca.key 4096
docker run --rm -v /tmp/spike-7732/taskC:/work -w /work alpine/openssl:latest req -x509 -new -nodes -key ca.key -sha256 -days 3650 -subj /CN=iron-proxy-spike-CA -addext basicConstraints=critical,CA:TRUE -addext keyUsage=critical,keyCertSign -out ca.crt
docker network create --internal --subnet 172.28.30.0/24 s7c-int
docker network create --subnet 172.29.30.0/24 s7c-egress
docker run -d --name s7c-proxy --network s7c-int --ip 172.28.30.2 -v /tmp/spike-7732/taskC/proxy.yaml:/etc/iron-proxy/proxy.yaml:ro -v /tmp/spike-7732/taskC/ca.crt:/etc/iron-proxy/ca.crt:ro -v /tmp/spike-7732/taskC/ca.key:/etc/iron-proxy/ca.key:ro ironsh/iron-proxy@sha256:c4628019c24f4cc8d77564a26b7c9cedb00accee6f93d06270e85fb8f9c6a7da -config /etc/iron-proxy/proxy.yaml
docker network connect s7c-egress s7c-proxy
docker run -d --name s7c-client --network s7c-int --dns 172.28.30.2 -v /tmp/spike-7732/taskC/ca.crt:/spike/ca.crt:ro nikolaik/python-nodejs:python3.12-nodejs22 sleep 1800
```

Proxy startup evidence:

```text
iron-proxy starting dns_listen=:53 http_listen=:80 https_listen=:443
transform pipeline transforms=allowlist
dns server starting addr=:53
https proxy starting addr=[::]:443
http proxy starting addr=[::]:80
container state: running
```

## Trust matrix

| Runtime / client | Works via Debian system store? | Extra environment variable needed? | Variable |
|---|---:|---:|---|
| curl | Yes | No | None; `--cacert /spike/ca.crt` also works before system installation |
| git HTTPS | Yes | No | None |
| Node `fetch` | No | Yes | `NODE_EXTRA_CA_CERTS=/spike/ca.crt` |
| Python `urllib.request` | Yes | No | None |
| pip 26.1.2 | Yes | No | None when the CA is in the system store; `SSL_CERT_FILE=/spike/ca.crt` is a working alternative |

## 7a. curl with explicit CA — PASS

No-trust control, before installing the CA:

```sh
docker exec s7c-client sh -c 'timeout 30 curl -sS https://example.com -o /dev/null -w "%{http_code}\n"; rc=$?; echo EXIT=$rc; exit 0'
```

```text
curl: (60) SSL certificate problem: self-signed certificate in certificate chain
000
EXIT=60
```

Explicit CA command:

```sh
docker exec s7c-client sh -c 'timeout 30 curl -sS --cacert /spike/ca.crt https://example.com -o /dev/null -w "%{http_code}\n"; rc=$?; echo EXIT=$rc; exit 0'
```

```text
200
EXIT=0
```

## 7b. curl through the system CA store — PASS

```sh
docker exec s7c-client sh -c 'timeout 30 sh -c "cp /spike/ca.crt /usr/local/share/ca-certificates/spike.crt && update-ca-certificates"; rc=$?; echo UPDATE_EXIT=$rc; timeout 30 curl -sS https://example.com -o /dev/null -w "%{http_code}\n"; rc=$?; echo CURL_EXIT=$rc; exit 0'
```

```text
Updating certificates in /etc/ssl/certs...
2 added, 0 removed; done.
UPDATE_EXIT=0
200
CURL_EXIT=0
```

## 7c. git through the system CA store — PASS

```sh
docker exec s7c-client sh -c 'rm -rf /tmp/hw; timeout 30 git clone --depth 1 https://github.com/octocat/Hello-World.git /tmp/hw; rc=$?; echo EXIT=$rc; if [ $rc -eq 0 ]; then git -C /tmp/hw rev-parse --short HEAD; fi; exit 0'
```

```text
Cloning into '/tmp/hw'...
EXIT=0
7fd1a60
```

The proxy log recorded three allowed smart-HTTP requests to `github.com`: `GET .../info/refs` and two `POST .../git-upload-pack` requests, all with HTTP 200. No request had `action=deny`, so no restart or allowlist addition was required.

Final configured host list, preserved in `proxy.yaml`:

```text
example.com
github.com
*.github.com
*.githubusercontent.com
codeload.github.com
pypi.org
files.pythonhosted.org
```

For this specific `--depth 1` fixture, the only GitHub host actually observed was `github.com`.

## 7d. Node without extra CA — PASS (expected rejection observed)

This ran after `update-ca-certificates` succeeded.

```sh
docker exec s7c-client sh -c 'timeout 30 node -e "fetch(\"https://example.com\").then(r=>console.log(\"status\",r.status)).catch(e=>{console.error(\"ERR\",e.cause?.code||e.message);process.exit(1)})"; rc=$?; echo EXIT=$rc; exit 0'
```

```text
ERR SELF_SIGNED_CERT_IN_CHAIN
EXIT=1
```

Node v22.23.2 did not use the updated Debian system store for this `fetch` call.

## 7e. Node with `NODE_EXTRA_CA_CERTS` — PASS

```sh
docker exec s7c-client sh -c 'timeout 30 env NODE_EXTRA_CA_CERTS=/spike/ca.crt node -e "fetch(\"https://example.com\").then(r=>console.log(\"status\",r.status)).catch(e=>{console.error(\"ERR\",e.cause?.code||e.message);process.exit(1)})"; rc=$?; echo EXIT=$rc; exit 0'
```

```text
status 200
EXIT=0
```

## 7f. Python standard library through the system store — PASS

```sh
docker exec s7c-client sh -c 'timeout 30 python3 -c "import urllib.request;print(urllib.request.urlopen(\"https://example.com\").status)"; rc=$?; echo EXIT=$rc; exit 0'
```

```text
200
EXIT=0
```

Python 3.12.13 used the updated Debian system store.

## 7g. pip — PASS

### Required no-env run after system-store installation

```sh
docker exec s7c-client sh -c 'rm -rf /tmp/pipdl; timeout 30 pip download six -d /tmp/pipdl --no-cache-dir; rc=$?; echo EXIT=$rc; if [ -d /tmp/pipdl ]; then printf "FILES="; find /tmp/pipdl -maxdepth 1 -type f -printf "%f "; echo; fi; exit 0'
```

```text
Collecting six
Downloading six-1.17.0-py2.py3-none-any.whl (11 kB)
Successfully downloaded six
EXIT=0
FILES=six-1.17.0-py2.py3-none-any.whl
```

This was a success, contrary to the expected certificate error. pip 26.1.2 used the installed system trust in this image.

### Required `SSL_CERT_FILE` run

```sh
docker exec s7c-client sh -c 'rm -rf /tmp/pipdl2; timeout 30 env SSL_CERT_FILE=/spike/ca.crt pip download six -d /tmp/pipdl2 --no-cache-dir; rc=$?; echo EXIT=$rc; if [ -d /tmp/pipdl2 ]; then printf "FILES="; find /tmp/pipdl2 -maxdepth 1 -type f -printf "%f "; echo; fi; exit 0'
```

```text
Collecting six
Downloading six-1.17.0-py2.py3-none-any.whl (11 kB)
Successfully downloaded six
EXIT=0
FILES=six-1.17.0-py2.py3-none-any.whl
```

### Extra isolation control

To prove the no-env success came from system trust, the spike CA was temporarily removed from the system store. The exact command was:

```sh
docker exec s7c-client sh -c 'rm -f /usr/local/share/ca-certificates/spike.crt; update-ca-certificates --fresh >/dev/null; rm -rf /tmp/pipdl-no-trust; timeout 30 pip download six -d /tmp/pipdl-no-trust --no-cache-dir; rc=$?; echo EXIT=$rc; exit 0'
```

Trimmed evidence:

```text
SSLError(SSLCertVerificationError(1, '[SSL: CERTIFICATE_VERIFY_FAILED] certificate verify failed: self-signed certificate in certificate chain'))
ERROR: Could not find a version that satisfies the requirement six
EXIT=1
```

With the system CA still absent, this exact command proved that `SSL_CERT_FILE` alone is sufficient:

```sh
docker exec s7c-client sh -c 'rm -rf /tmp/pipdl-env-only; timeout 30 env SSL_CERT_FILE=/spike/ca.crt pip download six -d /tmp/pipdl-env-only --no-cache-dir; rc=$?; echo EXIT=$rc; if [ -d /tmp/pipdl-env-only ]; then printf "FILES="; find /tmp/pipdl-env-only -maxdepth 1 -type f -printf "%f "; echo; fi; exit 0'
```

```text
Successfully downloaded six
EXIT=0
FILES=six-1.17.0-py2.py3-none-any.whl
```

The spike CA was then restored with `cp /spike/ca.crt /usr/local/share/ca-certificates/spike.crt && update-ca-certificates`. A final curl returned HTTP 200.

## Proxy evidence and surprising behavior

- `proxy.log` contains the full iron-proxy log.
- DNS interception was observed for `example.com`, `github.com`, `pypi.org`, and `files.pythonhosted.org`.
- The proxy logged allowed HTTP 200 requests for each runtime's successful path.
- No `"action":"deny"` entry exists in `proxy.log`.
- Surprise: pip 26.1.2 succeeded without `SSL_CERT_FILE` after the CA entered the Debian system store. Removing the CA made pip fail, and setting `SSL_CERT_FILE` made it pass again.
