#!/bin/bash
cd ~/ironclaw
export ALLOW_PRIVATE_IPS=1
export PGSSLMODE=disable
export HTTP_PORT=9098
export LLM_BASE_URL="http://192.168.1.157:3000/openai/v1"
export HTTP_WEBHOOK_SECRET="testtoken"
sleep 2s
exec ./target/release/ironclaw run
