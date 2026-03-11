output "namespace" {
  value = kubernetes_namespace.mission_control.metadata[0].name
}

output "router_service" {
  value = kubernetes_service.router.metadata[0].name
}
