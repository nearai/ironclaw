#!/bin/bash
# Test script for the restart tool feature
#
# This script:
# 1. Builds the restart-test Docker image
# 2. Starts PostgreSQL (if not running)
# 3. Runs IronClaw in Docker with the restart loop
# 4. Provides curl commands to test the restart tool

set -euo pipefail

DOCKER_IMAGE="ironclaw:restart-test"
CONTAINER_NAME="ironclaw-restart-test"
GATEWAY_TOKEN="changeme"
GATEWAY_PORT="3001"

echo "=== IronClaw Restart Tool Test ==="
echo ""

# Step 1: Check if .env exists
if [ ! -f .env ]; then
    echo "❌ .env file not found. Creating one..."
    cat > .env << 'EOF'
DATABASE_URL=postgres://ironclaw:ironclaw@postgres:5432/ironclaw
GATEWAY_ENABLED=true
GATEWAY_HOST=0.0.0.0
GATEWAY_PORT=3001
GATEWAY_AUTH_TOKEN=changeme
LLM_BACKEND=nearai
NEARAI_MODEL=zai-org/GLM-5-FP8
SECRETS_MASTER_KEY_METHOD=skip
EOF
    echo "✅ Created .env (edit with your LLM credentials)"
    echo ""
fi

# Step 2: Start PostgreSQL
echo "📦 Starting PostgreSQL..."
docker compose up -d postgres
echo "⏳ Waiting for PostgreSQL to be healthy..."
docker compose exec -T postgres pg_isready -U ironclaw > /dev/null 2>&1 || sleep 5
echo "✅ PostgreSQL ready"
echo ""

# Step 3: Build Docker image
echo "🏗️  Building Docker image: $DOCKER_IMAGE"
docker build -f Dockerfile.restart-test -t "$DOCKER_IMAGE" .
echo "✅ Docker image built"
echo ""

# Step 4: Stop previous container if running
if docker ps -a --format '{{.Names}}' | grep -q "^${CONTAINER_NAME}$"; then
    echo "🧹 Stopping previous container..."
    docker stop "$CONTAINER_NAME" 2>/dev/null || true
    docker rm "$CONTAINER_NAME" 2>/dev/null || true
fi

# Step 5: Run the container on the same network as PostgreSQL
echo "🚀 Starting IronClaw with restart loop..."
echo ""
docker run \
    --env-file .env \
    -p "$GATEWAY_PORT:3001" \
    --network ironclaw_default \
    --name "$CONTAINER_NAME" \
    "$DOCKER_IMAGE" &

# Wait for container to start
sleep 3

echo ""
echo "✅ Container started! Streaming logs..."
echo "   (Press Ctrl+C to stop, container will keep running)"
echo ""
docker logs -f "$CONTAINER_NAME" &
LOGS_PID=$!

# Give user time to see logs start
sleep 5

# Step 6: Print test instructions
echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "🧪 TEST THE RESTART TOOL"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""
echo "In another terminal, run one of these commands:"
echo ""
echo "1️⃣  Quick restart (2 second delay):"
echo ""
echo "curl -X POST http://localhost:$GATEWAY_PORT/api/jobs \\"
echo "  -H 'Authorization: Bearer $GATEWAY_TOKEN' \\"
echo "  -H 'Content-Type: application/json' \\"
echo "  -d '{"
echo '    "message": "Test restart with 2 second delay",'
echo '    "tools": ["restart"],'
echo '    "parameters": {"delay_secs": 2}'
echo "}'"
echo ""
echo "2️⃣  Longer delay (10 seconds):"
echo ""
echo "curl -X POST http://localhost:$GATEWAY_PORT/api/jobs \\"
echo "  -H 'Authorization: Bearer $GATEWAY_TOKEN' \\"
echo "  -H 'Content-Type: application/json' \\"
echo "  -d '{"
echo '    "message": "Test restart with longer delay",'
echo '    "tools": ["restart"],'
echo '    "parameters": {"delay_secs": 10}'
echo "}'"
echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "📊 EXPECTED BEHAVIOR:"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""
echo "✓ You should see in the logs:"
echo "  1. '[agent] Executing tool: restart'"
echo "  2. 'Restarting in X second(s). The process will exit cleanly...'"
echo "  3. '[YYYY-MM-DD HH:MM:SS] Clean exit (code 0), restarting in 5s...'"
echo "  4. '[YYYY-MM-DD HH:MM:SS] Starting IronClaw (attempt 1/10)'"
echo "  5. Back online with new process"
echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""
echo "⏸️  Press Ctrl+C to stop viewing logs (container still runs)"
echo "🛑 To stop container: docker stop $CONTAINER_NAME"
echo "📋 To view logs again: docker logs -f $CONTAINER_NAME"
echo ""

# Wait for Ctrl+C
wait $LOGS_PID 2>/dev/null || true
