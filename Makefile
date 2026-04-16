# BastionClaw – convenience wrapper around docker compose.
#
# Usage:
#   make up        – start the full stack (detached)
#   make build     – build / rebuild images without starting
#   make rebuild   – build then restart (use after code changes)
#   make down      – stop containers, keep volumes
#   make wipe      – stop containers AND delete all volumes (full reset)
#   make restart   – restart only the bastionclaw service
#   make logs      – follow bastionclaw logs
#   make shell     – open a shell inside the running container
#   make status    – show running container state
#   make help      – print this list

# ── Docker socket GID detection ───────────────────────────────────────────────
# The bastionclaw container needs to join the group that owns the Docker socket
# so sandbox jobs can spawn worker containers.
#
# macOS Docker Desktop: the socket lives inside a VM and is not stat-able from
#   the host, so the fallbacks below return 0 (root group), which is what
#   Docker Desktop maps the socket to inside containers.
# Linux VPS: stat -c %g returns the docker group GID (often 998–999).
#
# The value is exported so docker compose picks it up via ${DOCKER_GID:-0}
# in docker-compose.yml — no manual .env editing required.
DOCKER_GID := $(shell stat -c %g /var/run/docker.sock 2>/dev/null \
                    || stat -f %g /var/run/docker.sock 2>/dev/null \
                    || echo 0)
export DOCKER_GID

COMPOSE := docker compose --profile app
SERVICE := bastionclaw

.PHONY: up build rebuild down wipe restart logs shell status help

## Start the full stack (detached). Builds images if they don't exist yet.
up:
	@echo "Using DOCKER_GID=$(DOCKER_GID)"
	$(COMPOSE) up -d

## Build / rebuild images without starting containers.
build:
	@echo "Using DOCKER_GID=$(DOCKER_GID)"
	$(COMPOSE) build

## Build images then restart the stack — use this after code changes.
rebuild: build
	$(COMPOSE) up -d

## Stop containers and remove them. Volumes are preserved (data survives).
down:
	$(COMPOSE) down

## Full reset: stop containers AND delete all volumes. All stored data is lost.
wipe:
	@echo "This will delete all volumes (database, workspace, etc.). Press Ctrl-C to cancel."
	@sleep 3
	$(COMPOSE) down -v

## Restart only the bastionclaw service (e.g. after a config change).
restart:
	$(COMPOSE) restart $(SERVICE)

## Follow bastionclaw logs (Ctrl-C to stop).
logs:
	$(COMPOSE) logs -f $(SERVICE)

## Open an interactive shell inside the running bastionclaw container.
shell:
	docker exec -it bastion-claw-$(SERVICE)-1 sh

## Show current container status.
status:
	$(COMPOSE) ps

## Print available targets.
help:
	@grep -E '^## ' Makefile | sed 's/## /  /'
