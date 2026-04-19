  job_prompt job_id="abc123" content="Please wrap up" done=true
- This politely tells the subagent to finish its current task and exit
- The subagent gets a chance to save work and clean up
- You return to the main agent (me) once it completes
                                                                            
2. Cancel Job (Force Stop)                                                  
  cancel_job job_id="abc123"
- This immediately terminates the subagent
- Any unsaved work may be lost
- You return to the main agent immediately

How to find the job ID:
  list_jobs filter="active"

Example workflow:
1. You're stuck talking to a subagent
2. Check what's running: list_jobs filter="active"
3. See job ID like "abc123"
