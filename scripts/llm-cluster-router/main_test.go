package main

import (
	"context"
	"net/http"
	"net/http/httptest"
	"net/url"
	"reflect"
	"testing"
	"time"
)

func TestProbeNodeUsesConfiguredHealthPath(t *testing.T) {
	t.Parallel()

	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/health" {
			t.Fatalf("unexpected path %q", r.URL.Path)
		}
		w.WriteHeader(http.StatusOK)
	}))
	t.Cleanup(server.Close)

	node := testNode(t, server.URL)
	hc := healthConfig{
		Path:    "/health",
		Timeout: durationValue{Duration: time.Second},
	}

	if !probeNode(context.Background(), hc, node) {
		t.Fatal("expected probeNode to succeed on configured health path")
	}
}

func TestProbeNodeFallsBackToModelsForOllamaStyleUpstream(t *testing.T) {
	t.Parallel()

	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case "/health":
			http.NotFound(w, r)
		case "/v1/models":
			w.WriteHeader(http.StatusOK)
		default:
			t.Fatalf("unexpected path %q", r.URL.Path)
		}
	}))
	t.Cleanup(server.Close)

	node := testNode(t, server.URL)
	hc := healthConfig{
		Path:    "/health",
		Timeout: durationValue{Duration: time.Second},
	}

	if !probeNode(context.Background(), hc, node) {
		t.Fatal("expected probeNode to fall back to /v1/models")
	}
}

func TestProbeNodeDoesNotMaskNon404HealthFailures(t *testing.T) {
	t.Parallel()

	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/health" {
			t.Fatalf("unexpected path %q", r.URL.Path)
		}
		w.WriteHeader(http.StatusInternalServerError)
	}))
	t.Cleanup(server.Close)

	node := testNode(t, server.URL)
	hc := healthConfig{
		Path:    "/health",
		Timeout: durationValue{Duration: time.Second},
	}

	if probeNode(context.Background(), hc, node) {
		t.Fatal("expected probeNode to fail on non-404 health failure")
	}
}

func testNode(t *testing.T, rawURL string) *upstreamNode {
	t.Helper()

	parsed, err := url.Parse(rawURL)
	if err != nil {
		t.Fatalf("parse url: %v", err)
	}
	return &upstreamNode{
		cfg:     nodeConfig{Name: "test-node", Tier: "fast"},
		baseURL: parsed,
	}
}

func TestParseGPUCSV(t *testing.T) {
	t.Parallel()

	raw := "0, GPU-3090, 00000000:1B:00.0, NVIDIA GeForce RTX 3090, 24576 MiB, 37 MiB, 0 %, 21\n" +
		"1, GPU-4070, 00000000:68:00.0, NVIDIA GeForce RTX 4070 Ti SUPER, 16376 MiB, 3561 MiB, 22 %, 41\n"

	got, err := parseGPUCSV(raw)
	if err != nil {
		t.Fatalf("parseGPUCSV returned error: %v", err)
	}

	want := []gpuSnapshot{
		{
			Index:          0,
			UUID:           "GPU-3090",
			PCIBusID:       "00000000:1B:00.0",
			Name:           "NVIDIA GeForce RTX 3090",
			MemoryTotalMiB: 24576,
			MemoryUsedMiB:  37,
			UtilizationGPU: 0,
			TemperatureC:   21,
		},
		{
			Index:          1,
			UUID:           "GPU-4070",
			PCIBusID:       "00000000:68:00.0",
			Name:           "NVIDIA GeForce RTX 4070 Ti SUPER",
			MemoryTotalMiB: 16376,
			MemoryUsedMiB:  3561,
			UtilizationGPU: 22,
			TemperatureC:   41,
		},
	}

	if !reflect.DeepEqual(got, want) {
		t.Fatalf("parseGPUCSV mismatch\n got: %#v\nwant: %#v", got, want)
	}
}

func TestParseComputeAppsCSV(t *testing.T) {
	t.Parallel()

	raw := "GPU-4070, 1234, python, 2048 MiB\nGPU-4070, 5678, ollama, 512 MiB\n"

	got, err := parseComputeAppsCSV(raw)
	if err != nil {
		t.Fatalf("parseComputeAppsCSV returned error: %v", err)
	}

	want := map[string][]gpuProcess{
		"GPU-4070": {
			{PID: 1234, ProcessName: "python", UsedMemoryMiB: 2048},
			{PID: 5678, ProcessName: "ollama", UsedMemoryMiB: 512},
		},
	}

	if !reflect.DeepEqual(got, want) {
		t.Fatalf("parseComputeAppsCSV mismatch\n got: %#v\nwant: %#v", got, want)
	}
}

func TestAttachGPUProcesses(t *testing.T) {
	t.Parallel()

	gpus := []gpuSnapshot{
		{UUID: "GPU-3090"},
		{UUID: "GPU-4070"},
	}
	processes := map[string][]gpuProcess{
		"GPU-4070": {
			{PID: 5678, ProcessName: "ollama", UsedMemoryMiB: 512},
		},
	}

	got := attachGPUProcesses(gpus, processes)

	if len(got[0].Processes) != 0 {
		t.Fatalf("expected no processes on first gpu, got %#v", got[0].Processes)
	}
	if !reflect.DeepEqual(got[1].Processes, processes["GPU-4070"]) {
		t.Fatalf("expected attached processes on second gpu, got %#v", got[1].Processes)
	}
}
