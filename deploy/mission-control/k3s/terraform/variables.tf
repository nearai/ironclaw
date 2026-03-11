variable "kubeconfig_path" {
  type        = string
  description = "Path to the k3s kubeconfig."
  default     = "~/.kube/config"
}

variable "namespace" {
  type        = string
  description = "Namespace for mission-control components."
  default     = "ironclaw-mission-control"
}

variable "router_image" {
  type        = string
  description = "Container image for the llm-cluster-router."
  default     = "ghcr.io/nfsarch33/llm-cluster-router:latest"
}

variable "primary_upstream_url" {
  type        = string
  description = "Primary vLLM upstream."
  default     = "http://127.0.0.1:8001"
}

variable "secondary_upstream_url" {
  type        = string
  description = "Secondary vLLM upstream."
  default     = "http://127.0.0.1:8002"
}
