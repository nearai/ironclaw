#!/bin/bash
# Wrapper for infrastructure health check cron job

# Set environment if needed
export PATH=/usr/local/bin:/usr/bin:/bin

# Run the health check
exec /home/openjaw/.ironclaw/workspace/infrastructure-health-check/infrastructure-health-check.sh
