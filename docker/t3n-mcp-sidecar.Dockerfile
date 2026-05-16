FROM node:20-alpine

RUN apk add --no-cache dumb-init \
    && npm install --global pnpm

# Install from Trinity's pnpm workspace (client/). The lockfile and workspace:*
# protocol deps live at the workspace root — leaf packages (mcp/t3n-mcp, t3n-sdk)
# do not carry their own pnpm-lock.yaml.
WORKDIR /workspace
COPY --from=trinity_client package.json pnpm-lock.yaml pnpm-workspace.yaml ./
COPY --from=trinity_client t3n-sdk/package.json ./t3n-sdk/package.json
COPY --from=trinity_client mcp/t3n-mcp/package.json ./mcp/t3n-mcp/package.json

RUN pnpm install --frozen-lockfile --filter @terminal-3/t3n-mcp...

COPY --from=trinity_client t3n-sdk ./t3n-sdk
COPY --from=trinity_client mcp/t3n-mcp ./mcp/t3n-mcp
COPY --from=trinity_client shared ./shared

# Runtime cwd matches trinity's Dockerfile: sources under mcp/t3n-mcp, shared at ../../shared.
WORKDIR /workspace/mcp/t3n-mcp

RUN T3N_MCP_VER=$(node -e "process.stdout.write(require('./package.json').version)") && \
    T3N_SDK_VER=$(node -e "process.stdout.write(require('../../t3n-sdk/package.json').version)") && \
    echo "VERSIONS t3n-mcp=${T3N_MCP_VER} t3n-sdk=${T3N_SDK_VER}" && \
    printf 'T3N_MCP_VERSION=%s\nT3N_SDK_VERSION=%s\n' "${T3N_MCP_VER}" "${T3N_SDK_VER}" > ./.versions

LABEL org.opencontainers.image.title="t3n-mcp-sidecar"

COPY docker/t3n-mcp-bridge.mjs /bridge/t3n-mcp-bridge.mjs

RUN addgroup -S t3n \
    && adduser -S -G t3n t3n \
    && chown -R t3n:t3n /workspace /bridge \
    && mkdir -p /var/run/t3n-mcp \
    && chown t3n:t3n /var/run/t3n-mcp

USER t3n

ENV NODE_ENV=production
ENV LOG_LEVEL=info
# The bridge auto-detects dist/esm/index.js; when absent it runs `npx tsx src/index.ts`
# with cwd=T3N_PROJECT_DIR (matching trinity's own Dockerfile CMD).
ENV T3N_PROJECT_DIR=/workspace/mcp/t3n-mcp
ENV MCP_SOCKET_PATH=/var/run/t3n-mcp/t3n-mcp.sock

ENTRYPOINT ["dumb-init", "--"]
CMD ["node", "/bridge/t3n-mcp-bridge.mjs"]
