# T3Claw – convenience wrapper around docker compose.
#
# Usage:
#   make up              – start the full stack including t3n-mcp sidecar (detached)
#   make build           – build / rebuild all images including t3n-mcp sidecar
#   make rebuild         – build then restart (use after code changes)
#   make rebuild-claw    – rebuild t3claw + t3n-mcp-sidecar, then up
#   make rebuild-sidecar – rebuild only the t3n-mcp sidecar, recreate it, refresh t3claw
#   make up-no-t3n       – start stack without the t3n-mcp sidecar (no trinity needed)
#   make build-no-t3n    – build without the t3n-mcp sidecar
#   make rebuild-no-t3n  – build without sidecar then restart
#   make down            – stop containers, keep volumes
#   make wipe            – stop containers AND delete all volumes (full reset)
#   make wipe-all        – wipe + delete built images (forces full rebuild)
#   make restart         – restart only the t3claw service
#   make logs            – follow t3claw logs
#   make shell           – open a shell inside the running container
#   make status          – show running container state
#   make help            – print this list

# ── Docker socket GID detection ───────────────────────────────────────────────
# The t3claw container needs to join the group that owns the Docker socket
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

COMPOSE     := docker compose --profile app --profile mcp
COMPOSE_CORE := docker compose --profile app
SERVICE := t3claw
SIDECAR_SERVICE := t3n-mcp-sidecar
SIDECAR_REGISTRY_IMAGE := ghcr.io/terminal-3/t3n-mcp-sidecar:latest
SIDECAR_RUNTIME_IMAGE := t3claw/t3n-mcp-sidecar:local

.PHONY: up build rebuild rebuild-claw up-no-t3n build-no-t3n rebuild-no-t3n build-sidecar rebuild-sidecar push-sidecar-gcp pull-sidecar down wipe wipe-all restart logs shell status help

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

## Rebuild t3claw + t3n-mcp-sidecar images and bring the stack up.
## Use after edits to either Rust crate or the sidecar bridge. `up -d` recreates
## containers whose images changed, so the new code is picked up immediately.
rebuild-claw:
	@echo "Using DOCKER_GID=$(DOCKER_GID)"
	$(COMPOSE) build $(SERVICE) $(SIDECAR_SERVICE)
	$(COMPOSE) up -d

# TODO: remove the three -no-t3n targets below once the t3n-mcp sidecar image
#   is reliably published to GHCR and `make pull-sidecar` is the standard VPS flow.
## Start the stack without the t3n-mcp sidecar (no trinity repo needed).
up-no-t3n:
	@echo "Using DOCKER_GID=$(DOCKER_GID)"
	$(COMPOSE_CORE) up -d

## Build without the t3n-mcp sidecar (no trinity repo needed).
build-no-t3n:
	@echo "Using DOCKER_GID=$(DOCKER_GID)"
	$(COMPOSE_CORE) build

## Build without sidecar then restart.
rebuild-no-t3n: build-no-t3n
	$(COMPOSE_CORE) up -d

## Build the t3n-mcp-sidecar image for linux/amd64 and push to GCP Artifact Registry.
## Requires the ../trinity repo to be checked out as a sibling directory.
## Usage: make push-sidecar-gcp
push-sidecar-gcp:
	docker buildx build --platform linux/amd64 \
		--build-context trinity_mcp=../trinity/client/mcp/t3n-mcp \
		--build-context trinity_shared=../trinity/client/shared \
		-f docker/t3n-mcp-sidecar.Dockerfile \
		-t us-central1-docker.pkg.dev/gen-lang-client-0263867259/t3claw/t3n-mcp-sidecar:latest \
		--push .

## Build the t3n-mcp-sidecar image locally for the compose stack.
## Requires the ../trinity repo to be checked out as a sibling directory.
build-sidecar:
	$(COMPOSE) build $(SIDECAR_SERVICE)

## Rebuild only the sidecar, recreate it, then restart t3claw so MCP tools refresh.
rebuild-sidecar: build-sidecar
	$(COMPOSE) up -d --force-recreate $(SIDECAR_SERVICE)
	$(COMPOSE) restart $(SERVICE) || $(COMPOSE) up -d $(SERVICE)

## Pull the latest t3n-mcp-sidecar image from GHCR and restart the container.
## Use this after CI has published a new image (i.e. after merging to staging
## and the npm-package-release workflow has run in trinity).
pull-sidecar:
	docker pull $(SIDECAR_REGISTRY_IMAGE)
	docker tag $(SIDECAR_REGISTRY_IMAGE) $(SIDECAR_RUNTIME_IMAGE)
	$(COMPOSE) up -d --no-build --force-recreate $(SIDECAR_SERVICE)
	$(COMPOSE) restart $(SERVICE) || $(COMPOSE) up -d $(SERVICE)

## Stop containers and remove them. Volumes are preserved (data survives).
down:
	$(COMPOSE) down

## Full reset: stop containers AND delete all volumes. All stored data is lost.
wipe:
	@echo "This will delete all volumes (database, workspace, etc.). Press Ctrl-C to cancel."
	@sleep 3
	$(COMPOSE) down -v

## Nuclear reset: delete all volumes AND locally-built images. Forces a full rebuild next time.
wipe-all:
	@echo "This will delete all volumes AND built images. Press Ctrl-C to cancel."
	@sleep 3
	$(COMPOSE) down -v --rmi local

## Restart only the t3claw service (e.g. after a config change).
restart:
	$(COMPOSE) restart $(SERVICE)

## Follow t3claw logs (Ctrl-C to stop).
logs:
	$(COMPOSE) logs -f $(SERVICE)

## Open an interactive shell inside the running t3claw container.
shell:
	docker exec -it t3-claw-$(SERVICE)-1 sh

## Show current container status.
status:
	$(COMPOSE) ps

## Print available targets.
help:
	@grep -E '^## ' Makefile | sed 's/## /  /'
