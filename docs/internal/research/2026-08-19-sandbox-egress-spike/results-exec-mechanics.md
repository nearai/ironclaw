# Spike 7732 — Item 8: Docker exec process and stream behavior

Environment: Docker on macOS with OrbStack; container image `alpine:3.20`; container PID 1 is deliberately the non-reaping command `sleep 1800`.

Evidence limit: the run used the historical tag `alpine:3.20` but did not
record its immutable runtime digest. The commands below remain the exact
commands executed; an exact image replay cannot be claimed from this artifact.

Setup:

```sh
docker rm -f s7d-thread 2>/dev/null; rm -rf /tmp/spike-7732/taskD; mkdir -p /tmp/spike-7732/taskD
docker run -d --name s7d-thread alpine:3.20 sleep 1800
```

```text
662699132ba94b61f183fe63ecc8d3db4d5d9c9c192258255742a25aa1f3ff49
```

## 2. Hang test — FINDING (hypothesis not reproduced)

Exact command:

```sh
time timeout 20 docker exec s7d-thread sh -c 'sleep 300 & echo done'; echo exit=$?
```

Output:

```text
done

real    0m2.081s
user    0m0.564s
sys     0m0.238s
exit=0
```

The inherited stream did **not** hang until the 20-second timeout on this Docker version. The command returned exit code 0 after about 2.08 seconds. This is a key version-specific finding: leaving stdout inherited added roughly two seconds, but did not wait for the 300-second child and did not produce timeout exit code 124.

## 3. Fix test (`setsid` plus full redirection) — PASS

Exact command:

```sh
time timeout 20 docker exec s7d-thread sh -c 'setsid sleep 300 >/dev/null 2>&1 </dev/null & echo done'; echo exit=$?
```

Output:

```text
done

real    0m0.127s
user    0m0.045s
sys     0m0.047s
exit=0
```

It printed `done`, returned 0, and completed in 0.127 seconds, below the required two seconds.

## 4. Redirection without `setsid` — PASS / FINDING

Exact command:

```sh
time timeout 20 docker exec s7d-thread sh -c 'sleep 300 >/dev/null 2>&1 </dev/null & echo done'; echo exit=$?
```

Output:

```text
done

real    0m0.067s
user    0m0.028s
sys     0m0.047s
exit=0
```

Redirection alone sufficed for immediate exec-stream completion. Therefore, `setsid` is not required for stream closure in this topology; it is required to form an independently addressable process group for kill-tree behavior.

## 5. Exit-code propagation — PASS

Exact command:

```sh
docker exec s7d-thread sh -c 'exit 7'; echo code=$?
```

Output:

```text
code=7
```

Docker propagated the exec process exit code exactly.

## 6. Process-group kill from a sibling exec — FAIL under strict “gone” criterion; signal delivery works

Tree start and initial inspection:

```sh
docker exec -d s7d-thread sh -c 'setsid sh -c "sleep 400 & sleep 400 & wait" >/dev/null 2>&1 </dev/null'
sleep 1
docker exec s7d-thread ps -o pid,pgid,comm
```

Output:

```text
PID   PGID  COMMAND
    1     1 sleep
   11     6 sleep
   17    17 sleep
   23    18 sleep
   34    34 sh
   35    34 sleep
   36    34 sleep
   37    37 ps
```

The new tree used PGID 34. PID 17 was the detached `setsid sleep 300` from check 3 and was in its own PGID 17.

The requested kill spelling was attempted exactly:

```sh
docker exec s7d-thread sh -c 'kill -TERM -- -34'; echo kill_code=$?; sleep 1; docker exec s7d-thread ps -o pid,pgid,comm
```

Output:

```text
sh: invalid number '--'
kill_code=1
PID   PGID  COMMAND
    1     1 sleep
   11     6 sleep
   17    17 sleep
   23    18 sleep
   34    34 sh
   35    34 sleep
   36    34 sleep
   47    47 ps
```

Alpine BusyBox `sh` uses a `kill` builtin that rejects `--`. The compatible spelling was then used:

```sh
docker exec s7d-thread sh -c 'kill -TERM -34'; echo kill_code=$?; sleep 1; docker exec s7d-thread ps -o pid,pgid,comm
```

Output:

```text
kill_code=0
PID   PGID  COMMAND
    1     1 sleep
   11     6 sleep
   17    17 sleep
   23    18 sleep
   34    34 sh
   35    34 sleep
   36    34 sleep
   57    57 ps
```

State inspection:

```sh
docker exec s7d-thread ps -o pid,pgid,stat,comm
```

Output:

```text
PID   PGID  STAT COMMAND
    1     1 S    sleep
   11     6 S    sleep
   17    17 S    sleep
   23    18 S    sleep
   34    34 Z    sh
   35    34 Z    sleep
   36    34 Z    sleep
   62    62 R    ps
```

The sibling exec successfully sent SIGTERM to the complete PGID 34 tree: all three members changed from sleeping (`S`) to zombie (`Z`). The unrelated detached process PID 17 / PGID 17 remained sleeping and untouched. However, the strict acceptance condition says all PGID entries must be gone. They remained as zombies because PID 1 does not reap, so this sub-check is recorded FAIL under that criterion. The functional group-signal behavior itself worked.

## 7. Zombie accumulation — PASS (requirement demonstrated)

Exact creation and first inspection command:

```sh
docker exec s7d-thread sh -c '(sleep 0.2 &) ; sleep 1; ps -o pid,stat,comm'
```

Output:

```text
PID   STAT COMMAND
    1 S    sleep
   11 S    sleep
   17 S    sleep
   23 S    sleep
   34 Z    sh
   35 Z    sleep
   36 Z    sleep
   67 R    ps
   73 Z    sleep
```

Exact persistence command:

```sh
sleep 5; docker exec s7d-thread ps -o pid,stat,comm
```

Output:

```text
PID   STAT COMMAND
    1 S    sleep
   11 S    sleep
   17 S    sleep
   23 S    sleep
   34 Z    sh
   35 Z    sleep
   36 Z    sleep
   73 Z    sleep
   75 R    ps
```

The newly created PID 73 zombie persisted after five seconds. The three zombies from the group-kill check also persisted. This directly demonstrates accumulation under non-reaping PID 1.

## 8. Exec latency — PASS / MEASUREMENT

Exact command:

```sh
for i in 1 2 3 4 5; do /usr/bin/time -p docker exec s7d-thread true 2>&1 | grep real; done
```

Output:

```text
real 0.06
real 0.04
real 0.03
real 0.03
real 0.03
```

The five measured wall times were 60, 40, 30, 30, and 30 milliseconds. Mean latency was 38 milliseconds; median latency was 30 milliseconds.

## Conclusion

`ironclaw-exec` must redirect stdin, stdout, and stderr away from the Docker exec stream before backgrounding a child. Redirection alone closed the stream immediately here. It should also use `setsid`, not for stream closure, but to create a process group that a sibling exec can terminate as a tree. On Alpine BusyBox, the portable tested group-kill spelling is `kill -TERM -<PGID>`; the GNU-style `kill -TERM -- -<PGID>` fails in the shell builtin. Sibling-exec group signaling terminated the shell and both child sleeps without touching the detached sleep in another group. A reaping PID 1 (for example, `tini`) is required: otherwise every terminated orphan remains as a persistent zombie, and even a successful group kill cannot satisfy the strict condition that the process entries disappear. Baseline `docker exec ... true` latency was 30–60 ms, with a 38 ms mean and 30 ms median. The unredirected inherited-stream case did not hang to timeout on this Docker version, but it was much slower at about 2.08 seconds.
