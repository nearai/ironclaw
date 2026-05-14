FROM node:20-alpine

WORKDIR /app

RUN apk add --no-cache dumb-init \
    && npm install --global pnpm

# Build the t3n-mcp server directly from the sibling trinity repo rather than
# pulling @terminal-3/t3n-mcp from GitHub Packages. This gives us full control
# over the version: whatever commit is checked out in ../trinity (exposed here
# as the `trinity` build context in docker-compose.yml) is what gets baked
# into the image. No GITHUB_TOKEN required — the public @terminal3/* deps come
# from npmjs.com.
#
# Layout inside the image mirrors trinity's own Dockerfile so tsx can resolve
# the ../../shared/... imports the way the source expects:
#
#   /app/              <- client/mcp/t3n-mcp (source + node_modules)
#   /app/shared/bin/   <- client/shared/bin
#   /shared -> /app/shared  (symlink so ../../shared from /app resolves)

# Bundle the t3n-sdk at the location the mcp package.json's `link:../../t3n-sdk`
# resolves to from /app: that is, /t3n-sdk. Copy package.json + lockfile first
# so pnpm install on the SDK is cached, then overlay source + built dist.
COPY --from=trinity_sdk package.json /t3n-sdk/package.json
COPY --from=trinity_sdk pnpm-lock.yaml /t3n-sdk/pnpm-lock.yaml
RUN cd /t3n-sdk && pnpm install --frozen-lockfile
COPY --from=trinity_sdk . /t3n-sdk

COPY --from=trinity_mcp package.json ./package.json
COPY --from=trinity_mcp pnpm-lock.yaml ./pnpm-lock.yaml

RUN pnpm install --frozen-lockfile

COPY --from=trinity_mcp src ./src
COPY --from=trinity_mcp bin ./bin
COPY --from=trinity_mcp tsconfig.json ./tsconfig.json
COPY --from=trinity_mcp tsconfig.prod.json ./tsconfig.prod.json
COPY --from=trinity_mcp config.json ./config.json
COPY --from=trinity_mcp config.production.json ./config.production.json
COPY --from=trinity_mcp config.staging.json ./config.staging.json
# config.local.json is needed when T3N_SDK_ENV=local (e.g. the payroll-v2
# runbook). Trinity's root .dockerignore has `**/config.local.*` but that
# file is NOT ignored here: the `trinity_mcp` additional context is rooted at
# client/mcp/t3n-mcp, which has no .dockerignore of its own, so Docker applies
# no filter from the parent. The values in config.local.json (localhost URLs)
# are overridden at runtime by T3N_MCP_RPC_URL / T3N_MCP_DASHBOARD_URL env vars.
COPY --from=trinity_mcp config.local.json ./config.local.json
COPY --from=trinity_shared bin ./shared/bin
RUN ln -s /app/shared /shared

# Bake package versions into image labels so `docker inspect` and logs show exactly
# what was installed — catches SDK/package mismatches without shelling into containers.
RUN T3N_MCP_VER=$(node -e "process.stdout.write(require('/app/package.json').version)") && \
    T3N_SDK_VER=$(node -e "process.stdout.write(require('/app/node_modules/@terminal3/t3n-sdk/package.json').version)") && \
    echo "VERSIONS t3n-mcp=${T3N_MCP_VER} t3n-sdk=${T3N_SDK_VER}" && \
    printf 'T3N_MCP_VERSION=%s\nT3N_SDK_VERSION=%s\n' "${T3N_MCP_VER}" "${T3N_SDK_VER}" > /app/.versions

LABEL org.opencontainers.image.title="t3n-mcp-sidecar"

COPY docker/t3n-mcp-bridge.mjs /bridge/t3n-mcp-bridge.mjs

RUN addgroup -S t3n \
    && adduser -S -G t3n t3n \
    && chown -R t3n:t3n /app /bridge \
    && mkdir -p /var/run/t3n-mcp \
    && chown t3n:t3n /var/run/t3n-mcp

USER t3n

ENV NODE_ENV=production
ENV LOG_LEVEL=info
# The bridge auto-detects dist/esm/index.js; since we run from source via tsx,
# that file won't exist and the bridge falls back to `npx tsx src/index.ts`
# with cwd=T3N_PROJECT_DIR (matching trinity's own Dockerfile CMD).
ENV T3N_PROJECT_DIR=/app
ENV MCP_SOCKET_PATH=/var/run/t3n-mcp/t3n-mcp.sock

ENTRYPOINT ["dumb-init", "--"]
CMD ["node", "/bridge/t3n-mcp-bridge.mjs"]
