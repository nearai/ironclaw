# ──────────────────────────────────────────────────────────────────────────────           
# Stage 1: dependency builder                
# ──────────────────────────────────────────────────────────────────────────────           
FROM node:20-alpine AS builder               

WORKDIR /build                               

# Install Python build deps needed by any native npm modules                               
RUN apk add --no-cache python3 make g++                                                    

COPY package*.json ./                        
RUN npm install --omit=dev && npm cache clean --force                                      

# ──────────────────────────────────────────────────────────────────────────────           
# Stage 2: runtime image                     
# ──────────────────────────────────────────────────────────────────────────────           
FROM node:20-alpine AS runtime               

LABEL org.opencontainers.image.title="IronClaw Codex Worker" \                             
      org.opencontainers.image.description="Hybrid Codex worker with WebSocket agent communication" \                                                                                 
      org.opencontainers.image.version="1.0.0"                                             

# ── system packages ────────────────────────────────────────────────────────────          
# tini  : PID-1 signal reaping               
# python3 + pip : health server + websockets library                                       
# ca-certificates : TLS for outbound WebSocket connections                                 
RUN apk add --no-cache \                     
      tini \                                 
      python3 \                              
      py3-pip \                              
      ca-certificates \                      
    && pip3 install --no-cache-dir --break-system-packages websockets \                    
    && rm -rf /root/.cache                   

# ── non-root user ──────────────────────────────────────────────────────────────          
RUN addgroup -S codex && adduser -S codex -G codex -h /home/codex                          

# ── app layout ─────────────────────────────────────────────────────────────────          
WORKDIR /app                                 
COPY --from=builder /build/node_modules ./node_modules                                     
COPY --chown=codex:codex . .                 

# Ensure scripts are executable              
RUN chmod +x /app/entrypoint.sh /app/health_server.py /app/scripts/acp_bridge.py

# ── codex config directory + symlink ──────────────────────────────────────────           
RUN mkdir -p /home/codex/.codex \                                                          
    && if [ -f /app/config/config.toml ] && [ ! -f /app/config/codex.toml ]; then cp /app/config/config.toml /app/config/codex.toml; fi \
    && ln -s /app/config/codex.toml /home/codex/.codex/config.toml \                       
    && chown -R codex:codex /home/codex/.codex                                             

# ── WebSocket protocol config ──────────────────────────────────────────────────          
COPY --chown=codex:codex agent_comm_protocol.json /app/config/agent_comm_protocol.json     

# ── ports ──────────────────────────────────────────────────────────────────────          
# 8443 : health-check HTTP endpoint (Starforce convention)                                 
# 9090 : inbound IronClaw WebSocket server (server role)                                   
EXPOSE 8443                                  
EXPOSE 9090                                  

# ── health check ───────────────────────────────────────────────────────────────          
HEALTHCHECK --interval=30s --timeout=5s --start-period=15s --retries=3 CMD ["wget", "-qO-", "http://127.0.0.1:8443/health"]
USER codex                                   

ENTRYPOINT ["/sbin/tini", "--", "/app/entrypoint.sh"]                                      
CMD ["--mode", "cli"]
