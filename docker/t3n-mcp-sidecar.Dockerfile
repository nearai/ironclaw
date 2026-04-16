FROM node:20-alpine

WORKDIR /app

RUN apk add --no-cache dumb-init \
    && npm install --global pnpm

ARG GITHUB_TOKEN

COPY trinity/client/mcp/t3n-mcp/package.json ./package.json
COPY trinity/client/mcp/t3n-mcp/pnpm-lock.yaml ./pnpm-lock.yaml

RUN if [ -n "$GITHUB_TOKEN" ]; then \
      echo "@terminal-3:registry=https://npm.pkg.github.com" > .npmrc && \
      echo "//npm.pkg.github.com/:_authToken=$GITHUB_TOKEN" >> .npmrc; \
    fi

RUN pnpm install --frozen-lockfile && rm -f .npmrc

COPY trinity/client/mcp/t3n-mcp/src ./src
COPY trinity/client/mcp/t3n-mcp/bin ./bin
COPY trinity/client/mcp/t3n-mcp/tsconfig.json ./tsconfig.json
COPY trinity/client/mcp/t3n-mcp/tsconfig.prod.json ./tsconfig.prod.json
COPY trinity/client/mcp/t3n-mcp/config.json ./config.json
COPY trinity/client/mcp/t3n-mcp/config.production.json ./config.production.json
COPY trinity/client/mcp/t3n-mcp/config.staging.json ./config.staging.json
COPY trinity/client/mcp/t3n-mcp/config.local.json ./config.local.json
COPY trinity/client/shared/bin ./shared/bin
COPY bastion-claw/docker/t3n-mcp-bridge.mjs /bridge/t3n-mcp-bridge.mjs

RUN mkdir -p /shared && cp -r /app/shared/bin /shared/bin

RUN pnpm run build

RUN mkdir -p ../.. \
    && ln -s /app/shared ../../shared \
    && addgroup -S t3n \
    && adduser -S -G t3n t3n \
    && chown -R t3n:t3n /app /bridge

USER t3n

ENV NODE_ENV=production
ENV LOG_LEVEL=info
ENV T3N_PROJECT_DIR=/app
ENV MCP_SOCKET_PATH=/var/run/t3n-mcp/t3n-mcp.sock

ENTRYPOINT ["dumb-init", "--"]
CMD ["node", "/bridge/t3n-mcp-bridge.mjs"]
