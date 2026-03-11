terraform {
  required_version = ">= 1.8.0"

  required_providers {
    helm = {
      source  = "hashicorp/helm"
      version = "~> 2.16"
    }
    kubernetes = {
      source  = "hashicorp/kubernetes"
      version = "~> 2.33"
    }
  }
}

provider "kubernetes" {
  config_path = var.kubeconfig_path
}

provider "helm" {
  kubernetes {
    config_path = var.kubeconfig_path
  }
}

resource "kubernetes_namespace" "mission_control" {
  metadata {
    name = var.namespace
  }
}

resource "helm_release" "kube_prometheus_stack" {
  name       = "ironclaw-monitoring"
  namespace  = kubernetes_namespace.mission_control.metadata[0].name
  repository = "https://prometheus-community.github.io/helm-charts"
  chart      = "kube-prometheus-stack"
  version    = "72.6.2"

  values = [yamlencode({
    grafana = {
      service = {
        type = "NodePort"
      }
    }
    prometheus = {
      prometheusSpec = {
        serviceMonitorSelectorNilUsesHelmValues = false
      }
    }
  })]
}

resource "kubernetes_config_map" "router_config" {
  metadata {
    name      = "llm-cluster-router-config"
    namespace = kubernetes_namespace.mission_control.metadata[0].name
  }

  data = {
    "router.yml" = templatefile("${path.module}/router-config.yaml.tmpl", {
      primary_url   = var.primary_upstream_url
      secondary_url = var.secondary_upstream_url
    })
  }
}

resource "kubernetes_deployment" "router" {
  metadata {
    name      = "llm-cluster-router"
    namespace = kubernetes_namespace.mission_control.metadata[0].name
    labels = {
      app = "llm-cluster-router"
    }
  }

  spec {
    replicas = 1

    selector {
      match_labels = {
        app = "llm-cluster-router"
      }
    }

    template {
      metadata {
        labels = {
          app = "llm-cluster-router"
        }
      }

      spec {
        container {
          name  = "router"
          image = var.router_image
          args  = ["serve", "-config", "/etc/router/router.yml"]

          port {
            container_port = 8080
          }

          port {
            container_port = 9091
          }

          volume_mount {
            name       = "router-config"
            mount_path = "/etc/router"
            read_only  = true
          }

          liveness_probe {
            http_get {
              path = "/healthz"
              port = 8080
            }
          }

          readiness_probe {
            http_get {
              path = "/healthz"
              port = 8080
            }
          }
        }

        volume {
          name = "router-config"
          config_map {
            name = kubernetes_config_map.router_config.metadata[0].name
          }
        }
      }
    }
  }
}

resource "kubernetes_service" "router" {
  metadata {
    name      = "llm-cluster-router"
    namespace = kubernetes_namespace.mission_control.metadata[0].name
  }

  spec {
    selector = {
      app = "llm-cluster-router"
    }

    port {
      name        = "http"
      port        = 8080
      target_port = 8080
    }

    port {
      name        = "metrics"
      port        = 9091
      target_port = 9091
    }
  }
}
