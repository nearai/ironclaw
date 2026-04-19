**Status:** Draft — waiting on confirmation of package name and base image preference.
                                                                                                  
## 1) Open Questions                                                                              
- **Which Codex CLI?** OpenAI Codex CLI (`@openai/codex` via npm) or something else?
- **Base image preference:** `debian:bookworm-slim` (default), `ubuntu:24.04`, or `alpine`?       
- **Python + Node both needed?** Or only one?    
                                                 
## 2) Proposed Layout 
```                                                                                               
codex-worker-image/                              
2026-04-12 23:52:38     kageho  Dockerfile       
  README.md                                                                                       
  docker-bake.hcl  (optional)      
  Makefile         (optional)                                                                     
```                                              
                                                 
## 3) Draft Dockerfile (Debian slim, non-root)                                                    
                                                 
```dockerfile                                                                                     
# syntax=docker/dockerfile:1.7                   
FROM debian:bookworm-slim                                                                         
                                                 
ARG DEBIAN_FRONTEND=noninteractive                                                                
ARG USER=worker                                                                                   
ARG UID=1000                                     
ARG GID=1000                                                                                      
                                                 
# Core tools                                                                                      
RUN apt-get update && apt-get install -y --no-install-recommends \                                
    ca-certificates \                                                                             
    curl \          
    git \                                                                                         
    jq \                                                                                          
2026-04-12 23:52:39     kageho  openssh-client \                                                  
    python3 \           
    python3-venv \                                                                                
    ripgrep \                                                                                     
  && rm -rf /var/lib/apt/lists/*                 
                                                                                                  
# Node.js (for Codex CLI if it's the npm package)                                                 
# Uses NodeSource; for stricter pinning, switch to tarball + sha256.                              
RUN curl -fsSL https://deb.nodesource.com/setup_20.x | bash - \                                   
  && apt-get update && apt-get install -y --no-install-recommends nodejs \                        
  && rm -rf /var/lib/apt/lists/*
  2026-04-12 23:52:40     kageho  # Install Codex CLI (CONFIRM PACKAGE NAME)
# Example placeholder:
# RUN npm install -g @openai/codex@<PINNED_VERSION>
RUN npm install -g @openai/codex

# Create non-root user
RUN groupadd -g ${GID} ${USER} \
  && useradd -m -u ${UID} -g ${GID} -s /bin/bash ${USER}

USER ${USER}
WORKDIR /workspace

# Keep image behavior predictable
ENV HOME=/home/worker \
    PATH=/home/worker/.local/bin:/usr/local/bin:/usr/bin:/bin
2026-04-12 23:52:41     kageho  ```

## 4) Integration Notes for IronClaw
- Mount project into `/workspace`
- Pass secrets via env (e.g. `OPENAI_API_KEY`) rather than baking into image
- Run as non-root (already)
- Optional runtime hardening: `--cap-drop=ALL --security-opt=no-new-privileges`

## 5) Security Hardening Checklist
- [ ] Pin Codex CLI version
- [ ] Pin Node.js version (or use tarball + sha256)
2026-04-12 23:52:42     kageho  - [ ] Verify checksums for any downloaded binaries
- [ ] Drop capabilities at runtime
- [ ] Run as non-root user
- [ ] Clean apt cache (already in Dockerfile)

## 6) Next Steps
1. Confirm Codex CLI package name + version to pin
2. Decide base image
3. Add version pinning and checksum verification
4. Test build locally
5. Integrate with IronClaw `create_job` runner

---

**Created:** 2026-04-12   
2026-04-12 23:52:43     kageho  **Author:** Kageho + cmc

<suggestions>["Update it to pin Node and codex versions + checksum verify"]</suggestions>
<suggestions>["Add a README section for IronClaw integration details"]</suggestions>