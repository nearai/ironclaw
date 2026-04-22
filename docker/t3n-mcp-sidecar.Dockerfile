FROM node:20-alpine

WORKDIR /app

RUN apk add --no-cache dumb-init

# Configure the GitHub npm registry for the @terminal-3 scope only.
# @terminal-3/t3n-mcp is a private package on npm.pkg.github.com.
# @terminal3/* (no hyphen) dependencies are public packages on npmjs.com — do NOT
# route that scope to GitHub Packages or they will 404.
#
# GITHUB_TOKEN is provided via a Docker build secret so it never appears in any
# image layer.  In CI this is the auto-provided Actions token (no manual setup).
# For local rebuilds: DOCKER_BUILDKIT=1 docker build --secret id=github_token,env=GITHUB_TOKEN ...
RUN --mount=type=secret,id=npm_github_token \
    GITHUB_TOKEN=$(cat /run/secrets/npm_github_token) && \
    test -n "$GITHUB_TOKEN" || { echo "ERROR: npm_github_token build secret is required (read:packages on Terminal-3/trinity)."; exit 1; } && \
    printf "@terminal-3:registry=https://npm.pkg.github.com\n//npm.pkg.github.com/:_authToken=%s\n" "${GITHUB_TOKEN}" > /root/.npmrc

# Install t3n-mcp directly from the GitHub npm registry.
# The published package ships a pre-built dist/ (ESM output + shared binaries)
# so no compile step is needed — npm install is all that's required.
RUN npm install @terminal-3/t3n-mcp && rm -f /root/.npmrc

# Bake package versions into image labels so `docker inspect` and logs show exactly
# what was installed — catches SDK/package mismatches without shelling into containers.
RUN T3N_MCP_VER=$(node -e "process.stdout.write(require('/app/node_modules/@terminal-3/t3n-mcp/package.json').version)") && \
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
# The bridge spawns dist/esm/index.js relative to T3N_PROJECT_DIR.
# Point it at the installed package rather than the build root.
ENV T3N_PROJECT_DIR=/app/node_modules/@terminal-3/t3n-mcp
ENV MCP_SOCKET_PATH=/var/run/t3n-mcp/t3n-mcp.sock

ENTRYPOINT ["dumb-init", "--"]
CMD ["node", "/bridge/t3n-mcp-bridge.mjs"]
