"""Throughput benchmark — compare LLM model response times.

Runs the same complex prompt N times against the deployment and
reports per-request latency and throughput stats.

Usage:
    export BASE_URL=... ADMIN_TOKEN=...
    python integrations/abound/tests/test_throughput.py [--n 5] [--models model1,model2]
"""

import argparse
import atexit
import os
import statistics
import time
import uuid

import requests
from openai import OpenAI

BASE_URL = os.environ["BASE_URL"]
ADMIN_TOKEN = os.environ["ADMIN_TOKEN"]

PROMPT = (
    "I want to send $2,500 to India for family maintenance. "
    "Check the current exchange rate, tell me how much INR I'd get, "
    "show me my account limits, and recommend whether now is a good time to send. "
    "Present the payment reason options for me to choose from."
)

admin = requests.Session()
admin.headers.update({
    "Authorization": f"Bearer {ADMIN_TOKEN}",
    "Content-Type": "application/json",
})

user_ids = []


def cleanup():
    for uid in user_ids:
        admin.delete(f"{BASE_URL}/api/admin/users/{uid}")
    if user_ids:
        print(f"\nCleaned up {len(user_ids)} test users")


atexit.register(cleanup)


def create_user():
    r = admin.post(f"{BASE_URL}/api/admin/users", json={
        "display_name": "Throughput Test",
        "email": f"tp-{uuid.uuid4().hex[:8]}@example.com",
        "role": "member",
    })
    assert r.status_code == 200, f"Failed to create user: {r.text}"
    d = r.json()
    user_ids.append(d["id"])
    return d["id"], d["token"]


def set_model(model: str):
    """Switch the deployment's LLM model via settings API."""
    r = admin.put(
        f"{BASE_URL}/api/settings/llm_model",
        json={"value": model},
    )
    if r.status_code != 200:
        print(f"  Warning: could not set model to {model}: {r.status_code} {r.text[:100]}")


def run_benchmark(client: OpenAI, n: int, label: str):
    print(f"\n{'='*60}")
    print(f"  Model: {label}")
    print(f"  Prompt: {PROMPT[:80]}...")
    print(f"  Runs: {n}")
    print(f"{'='*60}")

    latencies = []
    token_counts = []
    errors = 0

    for i in range(n):
        start = time.time()
        try:
            response = client.responses.create(
                model="default",
                input=PROMPT,
            )
            elapsed = time.time() - start

            if response.status == "completed":
                latencies.append(elapsed)
                usage = response.usage
                if usage:
                    token_counts.append(usage.total_tokens)

                text_len = sum(
                    len(c.text)
                    for item in response.output if item.type == "message"
                    for c in item.content if c.type == "output_text"
                )
                print(f"  [{i+1}/{n}] {elapsed:.1f}s | {text_len} chars | {usage.total_tokens if usage else '?'} tokens")
            else:
                errors += 1
                print(f"  [{i+1}/{n}] FAILED ({response.status}) in {elapsed:.1f}s")
        except Exception as e:
            elapsed = time.time() - start
            errors += 1
            print(f"  [{i+1}/{n}] ERROR in {elapsed:.1f}s: {str(e)[:100]}")

    print(f"\n  --- Results for {label} ---")
    if latencies:
        print(f"  Successful: {len(latencies)}/{n}")
        print(f"  Errors:     {errors}")
        print(f"  Min:        {min(latencies):.1f}s")
        print(f"  Max:        {max(latencies):.1f}s")
        print(f"  Mean:       {statistics.mean(latencies):.1f}s")
        print(f"  Median:     {statistics.median(latencies):.1f}s")
        if len(latencies) > 1:
            print(f"  Stdev:      {statistics.stdev(latencies):.1f}s")
        if token_counts:
            avg_tokens = statistics.mean(token_counts)
            avg_time = statistics.mean(latencies)
            print(f"  Avg tokens: {avg_tokens:.0f}")
            print(f"  Tokens/sec: {avg_tokens / avg_time:.0f}")
    else:
        print(f"  All {n} requests failed")

    return latencies


def main():
    parser = argparse.ArgumentParser(description="LLM throughput benchmark")
    parser.add_argument("--n", type=int, default=3, help="Requests per model (default: 3)")
    parser.add_argument(
        "--models",
        type=str,
        default="Qwen/Qwen3.5-122B-A10B,anthropic/claude-sonnet-4-5",
        help="Comma-separated model names",
    )
    args = parser.parse_args()

    models = [m.strip() for m in args.models.split(",")]

    print(f"Throughput Benchmark")
    print(f"Target: {BASE_URL}")
    print(f"Models: {', '.join(models)}")
    print(f"Runs per model: {args.n}")

    # Create one user for all benchmarks
    uid, token = create_user()
    time.sleep(3)
    client = OpenAI(api_key=token, base_url=f"{BASE_URL}/v1")

    all_results = {}

    for model in models:
        set_model(model)
        time.sleep(2)  # Let the setting propagate
        latencies = run_benchmark(client, args.n, model)
        all_results[model] = latencies

    # Summary comparison
    print(f"\n{'='*60}")
    print(f"  COMPARISON SUMMARY")
    print(f"{'='*60}")
    print(f"  {'Model':<40} {'Mean':>8} {'Median':>8} {'Min':>8}")
    print(f"  {'-'*40} {'-'*8} {'-'*8} {'-'*8}")
    for model, lats in all_results.items():
        if lats:
            print(
                f"  {model:<40} "
                f"{statistics.mean(lats):>7.1f}s "
                f"{statistics.median(lats):>7.1f}s "
                f"{min(lats):>7.1f}s"
            )
        else:
            print(f"  {model:<40} {'FAILED':>8} {'':>8} {'':>8}")


if __name__ == "__main__":
    main()
