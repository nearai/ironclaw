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

.PHONY: up build rebuild build-sidecar pull-sidecar down wipe restart logs shell status help

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

## Build the t3n-mcp-sidecar image locally (requires NPM_GITHUB_TOKEN env var).
## Only needed when you want to test sidecar changes before they are published and
## pushed to GHCR by CI. Normal workflow: publish t3n-mcp via npm-package-release
## workflow in trinity, then use `make pull-sidecar` instead.
build-sidecar:
	@test -n "$(NPM_GITHUB_TOKEN)" || { echo "ERROR: NPM_GITHUB_TOKEN is not set"; exit 1; }
	DOCKER_BUILDKIT=1 docker build \
		--secret id=npm_github_token,env=NPM_GITHUB_TOKEN \
		-f docker/t3n-mcp-sidecar.Dockerfile \
		-t ghcr.io/terminal-3/t3n-mcp-sidecar:latest \
		.

## Pull the latest t3n-mcp-sidecar image from GHCR and restart the container.
## Use this after CI has published a new image (i.e. after merging to staging
## and the npm-package-release workflow has run in trinity).
pull-sidecar:
	docker pull ghcr.io/terminal-3/t3n-mcp-sidecar:latest
	$(COMPOSE) up -d t3n-mcp-sidecar

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
