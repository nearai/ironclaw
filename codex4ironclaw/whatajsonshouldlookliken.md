• For your worker, project_dir should map to the task envelope field:
                                                                     
  "context": {                                                       
    "path": "/workspace"                                             
  }                                                                  
                                                                     
  So if IronClaw has a project_dir setting, set it to:               
                                                                     
  /workspace                                                         
                                                                     
  That is the path inside the Codex worker container.                
                                                                     
  Full task with project dir:                                        
                                                                     
  {                                                                  
    "id": "ironclaw-test-001",                                       
    "type": "task_request",                                          
    "timestamp": "2026-04-18T04:35:00Z",
    "payload": {
      "task_id": "ironclaw-test-001",
      "prompt": "Use shell commands. Create /workspace/
  ironclaw_ws_test.txt containing ironclaw-ok, then list /workspace
  and cat the file. Do not just describe it.",
      "context": {
        "path": "/workspace"
      },
      "timeout_ms": 300000
    }
  }
