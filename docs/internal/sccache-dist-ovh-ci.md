# OVH sccache-dist CI builder

This repo uses an OVH dedicated server as a distributed `sccache` scheduler plus
one Linux build server.

## Server

- Host: `<private runbook>`
- SSH user: `<private runbook>`
- OS: Ubuntu 24.04 LTS
- Hardware observed at setup: 24 logical CPUs, 62 GiB RAM, NVMe RAID root
- Scheduler: `SCCACHE_DIST_SCHEDULER_URL` GitHub repository variable
- Builder: `<private runbook>`

## Services

On the server:

```sh
sudo systemctl status sccache-dist-scheduler.service
sudo systemctl status sccache-dist-server.service
sccache --dist-status
```

Config files:

```text
/etc/sccache-dist/scheduler.conf
/etc/sccache-dist/server.conf
/etc/sccache-dist/client.env
```

The client token is stored in `/etc/sccache-dist/client.env` and in the GitHub
secret `SCCACHE_DIST_AUTH_TOKEN`.

## GitHub Settings

Repository variable:

```text
SCCACHE_DIST_SCHEDULER_URL=https://<scheduler-host>
```

Repository secret:

```text
SCCACHE_DIST_AUTH_TOKEN=<token from /etc/sccache-dist/client.env>
```

## Smoke Test

Run the manual workflow:

```text
sccache Dist Smoke
```

Expected status:

```json
{"SchedulerStatus":["https://<scheduler-host>/",{"num_servers":1,"num_cpus":24,"in_progress":0}]}
```

## Rollout

After the smoke workflow succeeds, add the same `Install sccache` and
`Configure distributed sccache` steps to Linux Rust jobs, starting with:

1. `Tests (Reborn)` package-crate matrix
2. `Tests (Reborn)` root partition tests
3. `Code Style` Linux clippy jobs
4. Legacy heavy integration jobs

Keep `Swatinem/rust-cache` during the first rollout so dependency downloads and
non-cacheable target artifacts stay warm.
