#!/bin/bash
cd ~/ironclaw
export ALLOW_PRIVATE_IPS=1
export PGSSLMODE=disable
export HTTP_PORT=9098
export LLM_BASE_URL="http://192.168.1.157:3000/openai/v1"
export HTTP_WEBHOOK_SECRET="cc18474b57fefd3672e6670fae4439542e18395986ce41d4320b07c84cd3ea43"
sleep 2s
exec ./target/release/ironclaw run
