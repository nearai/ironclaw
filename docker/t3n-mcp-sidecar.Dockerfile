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
# config.local.json omitted: the sidecar only ever runs in staging or
# production mode, and trinity's .dockerignore blocks config.local.* anyway.
COPY --from=trinity_shared bin ./shared/bin
RUN ln -s /app/shared /shared

COPY docker/t3n-mcp-bridge.mjs /bridge/t3n-mcp-bridge.mjs

RUN addgroup -S t3n \
    && adduser -S -G t3n t3n \
    && chown -R t3n:t3n /app /bridge

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
