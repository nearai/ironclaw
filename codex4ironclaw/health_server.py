#!/usr/bin/env python3                       
"""                                          
health_server.py — Lightweight HTTP health-check endpoint (port 8443).                     

Endpoints:                                   
  GET /health  →  200 {"status":"ok", ...}                                                 
  GET /ready   →  200 {"ready": true}  or  503 {"ready": false}                            
  GET /        →  redirect to /health                                                      
"""                                          

import argparse                              
import json                                  
import os                                    
import sys                                   
import time                                  
from http.server import BaseHTTPRequestHandler, HTTPServer                                 

START_TIME = time.time()                     
WS_STATE_FILE = os.environ.get("WS_STATE_FILE", "/tmp/ironclaw_ws_state.json")

def _uptime() -> float:                      
    return round(time.time() - START_TIME, 2)                                              

def _build_health() -> dict:                 
    return {                                 
        "status": "ok",                      
        "uptime_seconds": _uptime(),                                                       
        "mode": os.environ.get("CODEX_MODE", "cli"),                                       
        "version": os.environ.get("CODEX_VERSION", "unknown"),                             
    }                                        

def _load_ws_state() -> dict | None:
    try:
        with open(WS_STATE_FILE, encoding="utf-8") as handle:
            return json.load(handle)
    except FileNotFoundError:
        return None
    except json.JSONDecodeError:
        return {"ready": False, "error": "invalid websocket state file"}

def _build_ready() -> tuple[dict, int]:                                                    
    mode = os.environ.get("CODEX_MODE", "cli")
    body = {"ready": True, "uptime_seconds": _uptime(), "mode": mode}

    if mode != "websocket":
        return body, 200

    ws_state = _load_ws_state()
    if ws_state is None:
        body["ready"] = False
        body["websocket"] = {"ready": False, "error": "state unavailable"}
        return body, 503

    ready = bool(ws_state.get("ready"))
    body["ready"] = ready
    body["websocket"] = ws_state
    status = 200 if ready else 503                                                         
    return body, status
class HealthHandler(BaseHTTPRequestHandler):                                               
    def log_message(self, fmt, *args):  # suppress default access log noise                
        pass                                 

    def _send_json(self, status: int, body: dict) -> None:                                 
        payload = json.dumps(body).encode()                                                
        self.send_response(status)                                                         
        self.send_header("Content-Type", "application/json")                               
        self.send_header("Content-Length", str(len(payload)))                              
        self.end_headers()                   
        self.wfile.write(payload)                                                          

    def do_GET(self):                        
        path = self.path.split("?")[0]                                                     

        if path in ("/", ""):                
            self.send_response(302)                                                        
            self.send_header("Location", "/health")                                        
            self.end_headers()               

        elif path == "/health":              
            self._send_json(200, _build_health())                                          

        elif path == "/ready":               
            body, status = _build_ready()                                                  
            self._send_json(status, body)                                                  

        else:                                
            self._send_json(404, {"error": "not found", "path": path})                     

def main() -> None:                          
    parser = argparse.ArgumentParser(description="Codex worker health server")             
    parser.add_argument("--port", type=int, default=8443, help="Listen port (default 8443)")                                                                                          
    parser.add_argument("--host", default="0.0.0.0", help="Bind address (default 0.0.0.0)")
    args = parser.parse_args()               

    server = HTTPServer((args.host, args.port), HealthHandler)                             
    print(f"[health_server] listening on {args.host}:{args.port}", flush=True)             
    try:                                     
        server.serve_forever()               
    except KeyboardInterrupt:                
        print("[health_server] shutting down", flush=True)                                 
        server.server_close()                
        sys.exit(0)                          

if __name__ == "__main__":                   
    main()
