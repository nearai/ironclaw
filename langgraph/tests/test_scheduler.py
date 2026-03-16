"""Tests for the job scheduler."""

from __future__ import annotations

import asyncio
import pytest
from langchain_core.messages import AIMessage, HumanMessage

from ironclaw.scheduler.scheduler import JobScheduler, JobState


class _ImmediateGraph:
    """Graph stub that immediately returns a fixed response."""

    async def ainvoke(self, state, config=None):
        return {
            "messages": [
                HumanMessage(content=state["messages"][0].content),
                AIMessage(content="done"),
            ]
        }


@pytest.mark.asyncio
async def test_dispatch_job_completes():
    scheduler = JobScheduler(max_parallel_jobs=3)
    graph = _ImmediateGraph()

    job_id = await scheduler.dispatch_job(graph, "user1", "Test job", "Do something")

    # Give the task time to finish
    await asyncio.sleep(0.1)

    job = scheduler.get_job(job_id)
    assert job is not None
    assert job.state == JobState.Completed
    assert job.result == "done"


@pytest.mark.asyncio
async def test_dispatch_job_exceeds_max():
    scheduler = JobScheduler(max_parallel_jobs=1)

    class _SlowGraph:
        async def ainvoke(self, state, config=None):
            await asyncio.sleep(10)  # long running
            return {"messages": [AIMessage(content="done")]}

    # Dispatch first job (will hold the slot)
    await scheduler.dispatch_job(_SlowGraph(), "user1", "Job 1", "task 1")

    with pytest.raises(RuntimeError, match="Max parallel jobs exceeded"):
        await scheduler.dispatch_job(_SlowGraph(), "user1", "Job 2", "task 2")

    await scheduler.stop_all()


@pytest.mark.asyncio
async def test_stop_job():
    class _NeverEndingGraph:
        async def ainvoke(self, state, config=None):
            await asyncio.sleep(100)
            return {"messages": []}

    scheduler = JobScheduler(max_parallel_jobs=3)
    job_id = await scheduler.dispatch_job(_NeverEndingGraph(), "u", "x", "y")
    await asyncio.sleep(0.05)

    await scheduler.stop_job(job_id)
    job = scheduler.get_job(job_id)
    assert job.state == JobState.Cancelled


@pytest.mark.asyncio
async def test_list_jobs():
    scheduler = JobScheduler(max_parallel_jobs=5)
    graph = _ImmediateGraph()

    id1 = await scheduler.dispatch_job(graph, "alice", "Job A", "task a")
    id2 = await scheduler.dispatch_job(graph, "bob", "Job B", "task b")
    id3 = await scheduler.dispatch_job(graph, "alice", "Job C", "task c")

    alice_jobs = scheduler.list_jobs(user_id="alice")
    assert len(alice_jobs) == 2
    assert all(j.user_id == "alice" for j in alice_jobs)

    all_jobs = scheduler.list_jobs()
    assert len(all_jobs) == 3

    await scheduler.stop_all()
