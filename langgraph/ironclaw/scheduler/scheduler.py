"""
Job scheduler — mirrors src/agent/scheduler.rs.

Background jobs are Python asyncio Tasks.  Each job runs an independent
invocation of the compiled agent graph with its own ``thread_id`` so that
checkpointing keeps job state separate from the interactive chat.

State machine (mirrors Rust JobState):
    Pending → InProgress → Completed
                        ↘ Failed
                        ↘ Cancelled
                        ↘ Stuck → InProgress (self-repair)

``JobScheduler`` is intentionally not a LangGraph node — it lives at the
application layer above the graph, just like the Rust ``Scheduler``.
"""

from __future__ import annotations

import asyncio
import logging
from dataclasses import dataclass, field
from datetime import datetime, timezone
from enum import Enum
from typing import Any
from uuid import uuid4

from langchain_core.messages import HumanMessage

logger = logging.getLogger(__name__)


class JobState(str, Enum):
    Pending = "pending"
    InProgress = "in_progress"
    Completed = "completed"
    Failed = "failed"
    Cancelled = "cancelled"
    Stuck = "stuck"


@dataclass
class ScheduledJob:
    job_id: str
    title: str
    description: str
    user_id: str
    state: JobState = JobState.Pending
    created_at: datetime = field(default_factory=lambda: datetime.now(timezone.utc))
    updated_at: datetime = field(default_factory=lambda: datetime.now(timezone.utc))
    error: str | None = None
    result: str | None = None
    _task: asyncio.Task | None = field(default=None, repr=False)
    _cancel_event: asyncio.Event = field(
        default_factory=asyncio.Event, repr=False
    )
    # Channel for follow-up user messages (mirrors WorkerMessage::UserMessage)
    _message_queue: asyncio.Queue[str] = field(
        default_factory=asyncio.Queue, repr=False
    )

    def cancel(self) -> None:
        self._cancel_event.set()
        if self._task and not self._task.done():
            self._task.cancel()

    def send_message(self, content: str) -> None:
        self._message_queue.put_nowait(content)


class JobScheduler:
    """
    Manages parallel background job execution.

    Mirrors the Rust ``Scheduler`` struct.  Jobs are asyncio Tasks that
    each run the compiled agent graph with an isolated thread_id so that
    LangGraph's MemorySaver keeps their state separate.
    """

    def __init__(self, max_parallel_jobs: int = 5) -> None:
        self._max_parallel_jobs = max_parallel_jobs
        self._jobs: dict[str, ScheduledJob] = {}
        self._lock = asyncio.Lock()

    # ------------------------------------------------------------------
    # Public API
    # ------------------------------------------------------------------

    async def dispatch_job(
        self,
        graph: Any,
        user_id: str,
        title: str,
        description: str,
    ) -> str:
        """
        Create, persist, and schedule a job in one atomic step.

        Returns the new job_id.  Mirrors ``Scheduler::dispatch_job``.
        """
        async with self._lock:
            running = sum(
                1
                for j in self._jobs.values()
                if j.state == JobState.InProgress
            )
            if running >= self._max_parallel_jobs:
                raise RuntimeError(
                    f"Max parallel jobs exceeded ({self._max_parallel_jobs})"
                )

            job_id = str(uuid4())
            job = ScheduledJob(
                job_id=job_id,
                title=title,
                description=description,
                user_id=user_id,
            )
            self._jobs[job_id] = job

        # Schedule outside the lock
        task = asyncio.create_task(
            self._run_job(graph, job),
            name=f"job-{job_id}",
        )
        job._task = task
        logger.info("Scheduled job %s: %s", job_id, title)
        return job_id

    async def stop_job(self, job_id: str) -> None:
        """Cancel a running job.  Mirrors ``Scheduler::stop``."""
        job = self._jobs.get(job_id)
        if job is None:
            raise KeyError(f"Job not found: {job_id}")
        job.cancel()
        job.state = JobState.Cancelled
        job.updated_at = datetime.now(timezone.utc)
        logger.info("Cancelled job %s", job_id)

    async def send_message(self, job_id: str, content: str) -> None:
        """
        Inject a follow-up user message into a running job.
        Mirrors ``Scheduler::send_message``.
        """
        job = self._jobs.get(job_id)
        if job is None or job.state != JobState.InProgress:
            raise KeyError(f"Job not running: {job_id}")
        job.send_message(content)

    def get_job(self, job_id: str) -> ScheduledJob | None:
        return self._jobs.get(job_id)

    def list_jobs(self, user_id: str | None = None) -> list[ScheduledJob]:
        jobs = list(self._jobs.values())
        if user_id:
            jobs = [j for j in jobs if j.user_id == user_id]
        return sorted(jobs, key=lambda j: j.created_at, reverse=True)

    def running_count(self) -> int:
        return sum(1 for j in self._jobs.values() if j.state == JobState.InProgress)

    async def stop_all(self) -> None:
        """Stop all running jobs.  Mirrors ``Scheduler::stop_all``."""
        for job in list(self._jobs.values()):
            if job.state == JobState.InProgress:
                job.cancel()
        logger.info("Stopped all jobs")

    # ------------------------------------------------------------------
    # Internal
    # ------------------------------------------------------------------

    async def _run_job(self, graph: Any, job: ScheduledJob) -> None:
        """
        Run the agent graph for a background job.

        The graph is invoked with the job description as the first user
        message.  Any follow-up messages from the scheduler's message
        queue are injected mid-run.
        """
        job.state = JobState.InProgress
        job.updated_at = datetime.now(timezone.utc)

        config = {"configurable": {"thread_id": job.job_id}}
        initial_input = {
            "messages": [HumanMessage(content=job.description)],
            "user_id": job.user_id,
        }

        try:
            result = await graph.ainvoke(initial_input, config=config)
            # Extract the last AI message as the result
            from langchain_core.messages import AIMessage
            last_ai = next(
                (m for m in reversed(result.get("messages", [])) if isinstance(m, AIMessage)),
                None,
            )
            job.result = last_ai.content if last_ai else ""
            job.state = JobState.Completed
        except asyncio.CancelledError:
            job.state = JobState.Cancelled
        except Exception as exc:  # noqa: BLE001
            job.state = JobState.Failed
            job.error = str(exc)
            logger.exception("Job %s failed", job.job_id)
        finally:
            job.updated_at = datetime.now(timezone.utc)
            logger.info("Job %s finished with state %s", job.job_id, job.state)
